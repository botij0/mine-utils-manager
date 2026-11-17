//! One shared Modrinth install routine: download, verify, commit, enable.
//!
//! Both the Tauri `install_modrinth_pack` command and the `install-modrinth`
//! probe subcommand run through [`prepare_modrinth_install`] (network, no
//! locks held) and [`finish_modrinth_install`] (import/enable/provenance,
//! caller holds the per-game-dir lock). The app path was previously broken:
//! the download temp was handed to the import paths directly, so a resource
//! pack install could never enable what it downloaded.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256, Sha512};

use crate::data_stack::{self, DataPaths, DatapackImportOutcome};
use crate::download::{self, DownloadError, InstallControl, VerifiedDownload, ZipRoot};
use crate::error::AppError;
use crate::game_lock;
use crate::level_dat;
use crate::modrinth::{validate_modrinth_id, ModrinthClient, ModrinthError, SearchKind, Transport};
use crate::options_txt::OptionsError;
use crate::pack::{ImportChoice, ImportOutcome};
use crate::provenance::{self, ProvenanceEntry};
use crate::resource_stack::{self, StackPaths};
use crate::world::WorldPaths;
use crate::{HashCache, IconCache};

/// Parse the wire install kind. The frontend sends `resourcepack` (one word,
/// matching the Modrinth project type) or `datapack`.
pub fn parse_install_kind(kind: &str) -> Result<SearchKind, ModrinthError> {
    match kind {
        "resourcepack" => Ok(SearchKind::ResourcePack),
        "datapack" => Ok(SearchKind::Datapack),
        other => Err(ModrinthError::Parse {
            reason: format!("unknown project type {other:?}; want resourcepack or datapack"),
        }),
    }
}

/// Where the verified bytes came from.
#[derive(Debug)]
pub enum PreparedSource {
    /// Freshly downloaded and verified this run. The temp deletes itself on
    /// drop unless committed.
    Downloaded(VerifiedDownload),
    /// The exact bytes (by SHA-512) were already in place; no download ran.
    /// The expected hash is carried into `finish` so a swap between prepare
    /// and finish is detected instead of misattributed.
    Existing {
        sha256: String,
        sha512: String,
    },
    Collision,
}

/// Everything [`finish_modrinth_install`] needs. Built without holding any
/// game lock: the metadata fetch and the download run outside the
/// import/enable/provenance phase.
#[derive(Debug)]
pub struct PreparedInstall {
    pub kind: SearchKind,
    pub project_id: String,
    pub version_id: String,
    pub filename: String,
    pub target_dir: PathBuf,
    pub world: Option<String>,
    pub source: PreparedSource,
    control: InstallControl,
}

/// What an install did. A `provenance_warning` never fails the install: the
/// file is in place and enabled, only the record write reported a problem.
#[derive(Debug)]
pub enum CoreInstallOutcome {
    ResourcePack {
        outcome: ImportOutcome,
        provenance_warning: Option<String>,
    },
    Datapack {
        outcome: DatapackImportOutcome,
        provenance_warning: Option<String>,
    },
}

/// Read-only half of the install: validate, guard-check, fetch metadata,
/// download and verify. Holds no locks.
#[allow(clippy::too_many_arguments)]
pub fn prepare_modrinth_install<T: Transport + 'static>(
    client: &ModrinthClient<T>,
    game_dir: &Path,
    world: Option<&str>,
    project_id: &str,
    version_id: &str,
    filename: &str,
    kind: &str,
) -> Result<PreparedInstall, AppError> {
    prepare_modrinth_install_controlled(
        client,
        game_dir,
        world,
        project_id,
        version_id,
        filename,
        kind,
        &InstallControl::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_modrinth_install_controlled<T: Transport + 'static>(
    client: &ModrinthClient<T>,
    game_dir: &Path,
    world: Option<&str>,
    project_id: &str,
    version_id: &str,
    filename: &str,
    kind: &str,
    control: &InstallControl,
) -> Result<PreparedInstall, AppError> {
    control.check()?;
    let kind = parse_install_kind(kind)?;
    validate_modrinth_id(project_id)?;
    validate_modrinth_id(version_id)?;
    download::validate_filename(filename)?;
    // Refuse before any download when the game is running: nothing is
    // copied, so there is no half-installed pack to explain. A locked
    // datapack world is the exception: the PR-04 path copies the file and
    // leaves it disabled with the `Locked` outcome, so the pre-check is
    // skipped and the world lock is re-probed at enable time.
    let (target_dir, world_version) = match kind {
        SearchKind::ResourcePack => {
            // A fresh game dir has no `options.txt` until Minecraft launches
            // once; enabling would fail after the commit. Refuse before any
            // download so the install is atomic.
            let options = game_dir.join("options.txt");
            if !options.is_file() {
                return Err(AppError::Options(OptionsError::Missing { path: options }));
            }
            (game_dir.join("resourcepacks"), None)
        }
        SearchKind::Datapack => {
            let name = world.ok_or_else(|| ModrinthError::Parse {
                reason: "a datapack install needs a target world".to_string(),
            })?;
            // Resolve the world eagerly so a typo fails before any download.
            let resolved = WorldPaths::for_world(game_dir, name)?;
            // A symlinked datapacks folder or level.dat fails here, before any
            // metadata or CDN request, with the same error finish gives.
            data_stack::reject_link_targets(resolved.world_dir())?;
            // The name may resolve while the world is missing or unreadable
            // (typo, deleted world): refuse before any metadata request.
            if !resolved.world_dir().is_dir() {
                return Err(AppError::World {
                    reason: format!("world {name} does not exist in this game dir"),
                });
            }
            if !resolved.level_dat_path().is_file() {
                return Err(AppError::World {
                    reason: format!("world {name} has no readable level.dat"),
                });
            }
            let version =
                world_version_name(&resolved.level_dat_path()).ok_or_else(|| AppError::World {
                    reason: format!("world {name} has no readable level.dat"),
                })?;
            (resolved.world_dir().join("datapacks"), Some(version))
        }
    };
    let world_locked = match kind {
        SearchKind::ResourcePack => false,
        // `datapacks` always has the world dir as its parent.
        SearchKind::Datapack => target_dir
            .parent()
            .and_then(game_lock::world_locked)
            .is_some(),
    };
    if !world_locked {
        game_lock::ensure_writable(game_dir)?;
    }
    let versions = client.versions(project_id)?;
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| ModrinthError::Parse {
            reason: format!("version {version_id} is not in project {project_id}"),
        })?;
    let file = version
        .files
        .iter()
        .find(|f| f.filename == filename)
        .ok_or_else(|| ModrinthError::Parse {
            reason: missing_file_reason(version_id, filename, version),
        })?;
    validate_sha512(&file.hashes.sha512, version_id, filename)?;
    // Datapacks and resource packs install as `.zip` files (`import_pack`
    // and the `data/` root check both require it). A version can also carry
    // mod `.jar` files, so refuse those before any download.
    match kind {
        SearchKind::Datapack | SearchKind::ResourcePack => {
            if !file.filename.to_ascii_lowercase().ends_with(".zip") {
                return Err(ModrinthError::Parse {
                    reason: format!(
                        "version {version_id} file {:?} is not a .zip pack; refusing before download",
                        file.filename,
                    ),
                }
                .into());
            }
        }
    }
    if let (SearchKind::Datapack, Some(world_v)) = (kind, world_version.as_deref()) {
        if !version.game_versions.is_empty() && !version.game_versions.iter().any(|g| g == world_v)
        {
            return Err(ModrinthError::Parse {
                reason: format!(
                    "version {version_id} supports {}, but world {} is {world_v}",
                    supported_list(version),
                    world.unwrap_or("?"),
                ),
            }
            .into());
        }
    }
    let root = match kind {
        SearchKind::ResourcePack => ZipRoot::Assets,
        SearchKind::Datapack => ZipRoot::Data,
    };
    let dest = target_dir.join(filename);
    crate::pack_scan::reject_ambiguous_name(&dest, &target_dir)?;
    let source = if dest.is_file() {
        match hash_file(&dest)? {
            hashes if hashes.sha512_eq(&file.hashes.sha512) => {
                // Identical bytes already in place: reuse them, no download.
                // The expected hash rides along so `finish` can re-hash and
                // refuse a swap that landed between the two phases.
                PreparedSource::Existing {
                    sha256: hashes.sha256,
                    sha512: file.hashes.sha512.to_ascii_lowercase(),
                }
            }
            _ => PreparedSource::Collision,
        }
    } else {
        let verified = download::download_verified_controlled(
            client,
            &file.url,
            &target_dir,
            &file.filename,
            &file.hashes.sha512,
            root,
            crate::modrinth::MAX_DOWNLOAD_BYTES,
            control,
        )?;
        PreparedSource::Downloaded(verified)
    };
    Ok(PreparedInstall {
        kind,
        project_id: project_id.to_string(),
        version_id: version_id.to_string(),
        filename: file.filename.clone(),
        target_dir,
        world: world.map(str::to_string),
        source,
        control: control.clone(),
    })
}

/// A Modrinth file hash must be 128 lowercase-able hex chars (SHA-512).
/// Checked before any download so an empty or malformed hash fails fast with
/// a clear parse error instead of a confusing `expected ""` mismatch after
/// the bytes arrive.
fn validate_sha512(hash: &str, version_id: &str, filename: &str) -> Result<(), ModrinthError> {
    let ok = hash.len() == 128 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(ModrinthError::Parse {
            reason: format!(
                "version {version_id} file {filename:?} has no usable SHA-512 hash; refusing before download"
            ),
        })
    }
}
fn missing_file_reason(
    version_id: &str,
    filename: &str,
    version: &crate::modrinth::ModrinthVersion,
) -> String {
    if version.files.is_empty() {
        return format!("version {version_id} has no downloadable file");
    }
    let mut names: Vec<&str> = version.files.iter().map(|f| f.filename.as_str()).collect();
    names.sort();
    names.truncate(5);
    format!(
        "version {version_id} has no file named {filename:?}; files: {}",
        names.join(", ")
    )
}

fn supported_list(version: &crate::modrinth::ModrinthVersion) -> String {
    if version.game_versions.is_empty() {
        return "no game versions".to_string();
    }
    let mut list = version.game_versions.clone();
    list.sort();
    list.truncate(8);
    list.join(", ")
}

struct FileHashes {
    sha512: String,
    sha256: String,
}

impl FileHashes {
    fn sha512_eq(&self, expected: &str) -> bool {
        self.sha512.len() == expected.len()
            && self
                .sha512
                .bytes()
                .zip(expected.to_ascii_lowercase().bytes())
                .all(|(a, b)| a == b)
    }
}

/// Hash an installed file with both digests the install commit needs.
/// Stays separate from `pack_import`'s single-SHA-256 loop: one pass must feed
/// both hashes, so sharing would cost a second read or a threaded signal.
fn hash_file(path: &Path) -> Result<FileHashes, AppError> {
    let mut file = std::fs::File::open(path).map_err(AppError::from)?;
    let mut sha512 = Sha512::new();
    let mut sha256 = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk).map_err(AppError::from)?;
        if n == 0 {
            break;
        }
        sha512.update(&chunk[..n]);
        sha256.update(&chunk[..n]);
    }
    Ok(FileHashes {
        sha512: hex::encode(sha512.finalize()),
        sha256: hex::encode(sha256.finalize()),
    })
}

/// Read a world's game version from its `level.dat`. `None` when the file is
/// missing or unreadable; `prepare` treats that as a refusal (a typo or an
/// unreadable world must fail before any download), never as a skip.
fn world_version_name(level_dat: &Path) -> Option<String> {
    let bytes = std::fs::read(level_dat).ok()?;
    level_dat::parse(&bytes).ok().map(|view| view.version_name)
}

/// Write half of the install: commit, import/enable, provenance. The caller
/// holds the per-game-dir lock across this call; nothing here touches the
/// network.
pub fn finish_modrinth_install(
    prepared: PreparedInstall,
    stack_paths: &StackPaths,
    data_paths: Option<&DataPaths>,
    data_dir: &Path,
    icons: &IconCache,
    hashes: &HashCache,
) -> Result<CoreInstallOutcome, AppError> {
    prepared.control.begin_commit()?;
    let ids = (
        prepared.project_id.clone(),
        prepared.version_id.clone(),
        prepared.filename.clone(),
    );
    let kind = prepared.kind;
    match kind {
        SearchKind::ResourcePack => {
            game_lock::ensure_writable(stack_paths.game_dir())?;
            let (dest, sha256, fresh) = match commit_source(prepared)? {
                Committed::Ready(dest, sha256, fresh) => (dest, sha256, fresh),
                Committed::Collision { name } => {
                    return Ok(CoreInstallOutcome::ResourcePack {
                        outcome: ImportOutcome::Collision { name },
                        provenance_warning: None,
                    });
                }
            };
            let outcome = match import_resource(&dest, stack_paths, icons, hashes) {
                Ok(outcome) => match outcome {
                    ImportOutcome::Collision { name } => {
                        // Same name, different bytes: leave the dest alone.
                        return Ok(CoreInstallOutcome::ResourcePack {
                            outcome: ImportOutcome::Collision { name },
                            provenance_warning: None,
                        });
                    }
                    outcome => outcome,
                },
                Err(err) => {
                    // A fresh commit that fails validation must not linger as
                    // a stray file with no provenance. An `Existing` file is
                    // never removed: it predates this install.
                    if fresh {
                        let _ = std::fs::remove_file(&dest);
                    }
                    return Err(err);
                }
            };
            let provenance_warning =
                record_provenance(data_dir, &sha256, &ids, outcome_is_imported(&outcome));
            // A game can still start after the finish guard. Preserve the
            // committed file and its provenance if enabling then fails.
            if let Err(err) = enable_resource(stack_paths, &outcome) {
                let _ = best_effort_provenance(data_dir, &sha256, &ids);
                return Err(err);
            }
            Ok(CoreInstallOutcome::ResourcePack {
                outcome,
                provenance_warning,
            })
        }
        SearchKind::Datapack => {
            let paths = data_paths.ok_or_else(|| AppError::World {
                reason: "a datapack install needs a target world".to_string(),
            })?;
            let (dest, sha256, fresh) = match commit_source(prepared)? {
                Committed::Ready(dest, sha256, fresh) => (dest, sha256, fresh),
                Committed::Collision { name } => {
                    return Ok(CoreInstallOutcome::Datapack {
                        outcome: DatapackImportOutcome::Collision { name },
                        provenance_warning: None,
                    });
                }
            };
            let outcome = match import_datapack(paths, &dest, icons, hashes) {
                Ok(outcome) => match outcome {
                    DatapackImportOutcome::Collision { name } => {
                        return Ok(CoreInstallOutcome::Datapack {
                            outcome: DatapackImportOutcome::Collision { name },
                            provenance_warning: None,
                        });
                    }
                    outcome => outcome,
                },
                Err(err) => {
                    if fresh {
                        let _ = std::fs::remove_file(&dest);
                    }
                    return Err(err);
                }
            };
            // `import_datapack` enables internally (`Locked` when the world
            // is locked). Record provenance for installed packs; a record
            // failure is a warning, never a failed install.
            let installed = matches!(
                outcome,
                DatapackImportOutcome::Imported { .. } | DatapackImportOutcome::Locked { .. }
            );
            let provenance_warning = record_provenance(data_dir, &sha256, &ids, installed);
            Ok(CoreInstallOutcome::Datapack {
                outcome,
                provenance_warning,
            })
        }
    }
}

/// Commit the verified temp (or reuse verified-existing bytes). A lost race
/// on an identical file reuses it; a lost race on different bytes reports a
/// collision. An `Existing` dest is re-hashed immediately before reuse: if
/// the bytes changed between prepare and finish, the install fails instead
/// of recording provenance for the wrong bytes. Returns the final path, its
/// SHA-256, whether the commit was fresh this run, or the collision.
/// A second filename with identical bytes is an intentional second copy under
/// the API filename (not a `Duplicate`): the picker installs by filename and
/// the import path treats a direct child as the pack itself.
enum Committed {
    Ready(PathBuf, String, bool),
    Collision { name: String },
}

fn commit_source(prepared: PreparedInstall) -> Result<Committed, AppError> {
    let PreparedInstall {
        target_dir,
        filename,
        source,
        ..
    } = prepared;
    crate::pack_scan::reject_ambiguous_name(&target_dir.join(&filename), &target_dir)?;
    match source {
        PreparedSource::Collision => Ok(Committed::Collision { name: filename }),
        PreparedSource::Existing { sha256, sha512 } => {
            let dest = target_dir.join(&filename);
            let current = hash_file(&dest)?;
            if !current.sha512_eq(&sha512) {
                return Err(AppError::Download(DownloadError::HashMismatch {
                    expected: sha512,
                    actual: current.sha512,
                }));
            }
            Ok(Committed::Ready(dest, sha256, false))
        }
        PreparedSource::Downloaded(verified) => {
            let sha256 = verified.sha256.clone();
            match verified.commit(&target_dir, &filename) {
                Ok(dest) => Ok(Committed::Ready(dest, sha256, true)),
                Err(DownloadError::Exists { .. }) => {
                    // Lost a race: reuse identical bytes, else collide. The
                    // temp was cleaned up by `Drop` on the failed commit.
                    let dest = target_dir.join(&filename);
                    if std::fs::symlink_metadata(&dest).is_ok_and(|entry| !entry.is_file()) {
                        return Ok(Committed::Collision { name: filename });
                    }
                    let hashes = hash_file(&dest)?;
                    if hashes.sha256 == sha256 {
                        Ok(Committed::Ready(dest, sha256, false))
                    } else {
                        Ok(Committed::Collision { name: filename })
                    }
                }
                Err(err) => Err(err.into()),
            }
        }
    }
}

fn import_resource(
    dest: &Path,
    stack_paths: &StackPaths,
    icons: &IconCache,
    hashes: &HashCache,
) -> Result<ImportOutcome, AppError> {
    // `dest` is a direct child of `resourcepacks`, so a fresh commit imports
    // as a no-op returning the pack; duplicates/collisions surface here.
    crate::pack_import::import_pack(
        dest,
        &stack_paths.game_dir().join("resourcepacks"),
        icons,
        ImportChoice::Default,
        hashes,
    )
}

fn outcome_is_imported(outcome: &ImportOutcome) -> bool {
    matches!(outcome, ImportOutcome::Imported { .. })
}

fn enable_resource(stack_paths: &StackPaths, outcome: &ImportOutcome) -> Result<(), AppError> {
    if let ImportOutcome::Imported { pack } = outcome {
        let filename = crate::resource_stack::filename_for(&pack.path)?;
        resource_stack::set_enabled(stack_paths, &filename, true)?;
    }
    Ok(())
}

fn import_datapack(
    paths: &DataPaths,
    dest: &Path,
    icons: &IconCache,
    hashes: &HashCache,
) -> Result<DatapackImportOutcome, AppError> {
    // `dest` is a direct child of the world's `datapacks`, so a fresh commit
    // imports as a no-op; enabling (or the locked-disabled outcome) happens
    // inside through the PR-04 path.
    data_stack::import_datapack(paths, dest, ImportChoice::Default, icons, hashes)
}

fn record_provenance(
    data_dir: &Path,
    sha256: &str,
    ids: &(String, String, String),
    installed: bool,
) -> Option<String> {
    if !installed {
        return None;
    }
    match provenance::record(
        data_dir,
        sha256,
        ProvenanceEntry {
            project_id: ids.0.clone(),
            version_id: ids.1.clone(),
            file_name: ids.2.clone(),
        },
    ) {
        Ok(()) => None,
        Err(err) => Some(format!("installed, but provenance was not recorded: {err}")),
    }
}

fn best_effort_provenance(
    data_dir: &Path,
    sha256: &str,
    ids: &(String, String, String),
) -> Result<(), provenance::ProvenanceError> {
    provenance::record(
        data_dir,
        sha256,
        ProvenanceEntry {
            project_id: ids.0.clone(),
            version_id: ids.1.clone(),
            file_name: ids.2.clone(),
        },
    )
}
