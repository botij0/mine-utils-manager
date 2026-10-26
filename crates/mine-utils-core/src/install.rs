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
use crate::download::{self, DownloadError, VerifiedDownload, ZipRoot};
use crate::error::AppError;
use crate::game_lock;
use crate::level_dat;
use crate::modrinth::{validate_modrinth_id, ModrinthClient, ModrinthError, SearchKind, Transport};
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
    Existing { sha256: String },
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
pub fn prepare_modrinth_install<T: Transport>(
    client: &ModrinthClient<T>,
    game_dir: &Path,
    world: Option<&str>,
    project_id: &str,
    version_id: &str,
    filename: &str,
    kind: &str,
) -> Result<PreparedInstall, AppError> {
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
        SearchKind::ResourcePack => (game_dir.join("resourcepacks"), None),
        SearchKind::Datapack => {
            let name = world.ok_or_else(|| ModrinthError::Parse {
                reason: "a datapack install needs a target world".to_string(),
            })?;
            // Resolve the world eagerly so a typo fails before any download.
            let resolved = WorldPaths::for_world(game_dir, name)?;
            let version = world_version_name(&resolved.world_dir);
            (resolved.world_dir.join("datapacks"), version)
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
    let source = if dest.is_file() {
        match hash_file(&dest)? {
            hashes if hashes.sha512_eq(&file.hashes.sha512) => {
                // Identical bytes already in place: reuse them, no download.
                PreparedSource::Existing {
                    sha256: hashes.sha256,
                }
            }
            _ => {
                // Same name, different bytes: download and verify the real
                // file, then report a collision without touching the dest.
                let verified = download::download_verified(
                    client,
                    &file.url,
                    &target_dir,
                    &file.filename,
                    &file.hashes.sha512,
                    root,
                )?;
                PreparedSource::Downloaded(verified)
            }
        }
    } else {
        let verified = download::download_verified(
            client,
            &file.url,
            &target_dir,
            &file.filename,
            &file.hashes.sha512,
            root,
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
    })
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
/// missing or unreadable; the caller then skips target validation rather
/// than refusing the install.
fn world_version_name(world_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(world_dir.join("level.dat")).ok()?;
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
    let ids = (
        prepared.project_id.clone(),
        prepared.version_id.clone(),
        prepared.filename.clone(),
    );
    let kind = prepared.kind;
    match kind {
        SearchKind::ResourcePack => {
            let (dest, sha256) = match commit_source(prepared)? {
                Committed::Ready(dest, sha256) => (dest, sha256),
                Committed::Collision { name } => {
                    return Ok(CoreInstallOutcome::ResourcePack {
                        outcome: ImportOutcome::Collision { name },
                        provenance_warning: None,
                    });
                }
            };
            let outcome = match import_resource(&dest, stack_paths, icons, hashes)? {
                ImportOutcome::Collision { name } => {
                    // Same name, different bytes: leave the dest alone.
                    return Ok(CoreInstallOutcome::ResourcePack {
                        outcome: ImportOutcome::Collision { name },
                        provenance_warning: None,
                    });
                }
                outcome => outcome,
            };
            let provenance_warning =
                record_provenance(data_dir, &sha256, &ids, outcome_is_imported(&outcome));
            // Enabling can still lose a race with a game start after the
            // pre-check. The verified file is already committed, so record
            // provenance (best effort) before reporting the failure: never
            // leave a copied pack with a failed command and no provenance.
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
            let paths = data_paths.expect("datapack target checked in prepare");
            let (dest, sha256) = match commit_source(prepared)? {
                Committed::Ready(dest, sha256) => (dest, sha256),
                Committed::Collision { name } => {
                    return Ok(CoreInstallOutcome::Datapack {
                        outcome: DatapackImportOutcome::Collision { name },
                        provenance_warning: None,
                    });
                }
            };
            let outcome = match import_datapack(paths, &dest, icons, hashes)? {
                DatapackImportOutcome::Collision { name } => {
                    return Ok(CoreInstallOutcome::Datapack {
                        outcome: DatapackImportOutcome::Collision { name },
                        provenance_warning: None,
                    });
                }
                outcome => outcome,
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
/// collision. Returns the final path and its SHA-256, or the collision.
enum Committed {
    Ready(PathBuf, String),
    Collision { name: String },
}

fn commit_source(prepared: PreparedInstall) -> Result<Committed, AppError> {
    let PreparedInstall {
        target_dir,
        filename,
        source,
        ..
    } = prepared;
    match source {
        PreparedSource::Existing { sha256 } => {
            Ok(Committed::Ready(target_dir.join(&filename), sha256))
        }
        PreparedSource::Downloaded(verified) => {
            let sha256 = verified.sha256.clone();
            match verified.commit(&target_dir, &filename) {
                Ok(dest) => Ok(Committed::Ready(dest, sha256)),
                Err(DownloadError::Exists { .. }) => {
                    // Lost a race: reuse identical bytes, else collide. The
                    // temp was cleaned up by `Drop` on the failed commit.
                    let dest = target_dir.join(&filename);
                    let hashes = hash_file(&dest)?;
                    if hashes.sha256 == sha256 {
                        Ok(Committed::Ready(dest, sha256))
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
    crate::pack_scan::import_pack(
        dest,
        &stack_paths.game_dir.join("resourcepacks"),
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
