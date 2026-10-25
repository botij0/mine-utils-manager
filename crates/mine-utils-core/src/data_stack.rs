//! The datapack stack: what `level.dat` keeps at `Data.DataPacks`.
//!
//! The `Enabled` list is load order, last wins. The UI shows the reverse, so
//! the top card is the highest priority. Entries that are not `file/<name>`
//! (`vanilla`, `fabric`, `fabric-convention-tags-v2`, experiment names, …)
//! are foreign and locked: they never move, toggle, or disappear.
//!
//! Every write goes through the same sequence as the resource stack: block
//! while the game runs or the world is locked, re-read the file, apply the
//! change to the latest content, copy the previous bytes to a backup, then
//! write atomically. A change whose file set moved out from under the UI comes
//! back as `StaleFile` so the UI refreshes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::game_lock;
use crate::level_backup::{self, LevelBackupOutcome};
use crate::level_dat::{self, LevelDatError};
use crate::pack::{ImportChoice, PackError};
use crate::pack_scan::{
    self, HashCache, PackListing, SystemTrash, TrashService, MAX_ICON_BYTES, MAX_META_BYTES,
    MAX_ZIP_ENTRIES,
};
use crate::resource_stack::{SystemGuard, WriteGuard};

/// Everything that can go wrong with a datapack stack change request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DataStackError {
    #[error("{filename} is not a safe datapack file name")]
    InvalidFilename { filename: String },
    #[error("the locked entry {value} cannot move; only file/ entries can be reordered")]
    ForeignMoved { value: String, index: usize },
    #[error("the requested order is not a reorder of the current stack")]
    OrderChanged,
    #[error("level.dat changed on disk; reload it and try again")]
    StaleFile,
    #[error("{name} is a resource pack, not a datapack: it has no data root")]
    ResourceOnly { name: String },
}

impl DataStackError {
    pub fn code(&self) -> &'static str {
        match self {
            DataStackError::StaleFile => "stale_stack",
            _ => "level_dat",
        }
    }
}

/// One `DataPacks` entry. The wire tags each entry with `kind`, mirroring the
/// resource stack shape in `src/lib/types.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataStackEntry {
    /// The exact `file/<filename>` value plus the filename alone.
    File { value: String, filename: String },
    /// A locked string the app never edits, such as `vanilla`.
    Foreign { value: String },
}

impl DataStackEntry {
    pub fn parse(value: &str) -> DataStackEntry {
        match value.strip_prefix("file/") {
            Some(filename) => DataStackEntry::File {
                value: value.to_string(),
                filename: filename.to_string(),
            },
            None => DataStackEntry::Foreign {
                value: value.to_string(),
            },
        }
    }

    pub fn value(&self) -> &str {
        match self {
            DataStackEntry::File { value, .. } => value,
            DataStackEntry::Foreign { value } => value,
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, DataStackEntry::File { .. })
    }
}

/// The active datapack stack as the app sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataStack {
    /// Load order, low to high. The UI shows the reverse, so the top card is
    /// the highest priority and loads last.
    pub enabled: Vec<DataStackEntry>,
    /// Disabled packs, in file order. Only `file/` entries are actionable.
    pub disabled: Vec<DataStackEntry>,
    /// The running-game state for the game dir. Writes are blocked while set.
    pub running: Option<game_lock::GameRunning>,
    /// The lock on this world's own `session.lock`, when held. Names the
    /// world in the banner.
    pub world_locked: Option<game_lock::GameRunning>,
    /// Stale `session.lock` files. They never block a write.
    pub stale_locks: Vec<PathBuf>,
    /// False where POSIX lock probing is unsupported.
    pub detection_supported: bool,
}

/// What the UI rendered. A mutation that lands on a different file set comes
/// back as [`DataStackError::StaleFile`] instead of clobbering the new content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSnapshot {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
}

impl DataStack {
    pub fn snapshot(&self) -> DataSnapshot {
        DataSnapshot {
            enabled: self.enabled.iter().map(|e| e.value().to_string()).collect(),
            disabled: self
                .disabled
                .iter()
                .map(|e| e.value().to_string())
                .collect(),
        }
    }
}

/// Where a world's datapack stack lives and which backup policy applies.
#[derive(Debug, Clone)]
pub struct DataPaths {
    pub game_dir: PathBuf,
    pub world_dir: PathBuf,
    pub level_dat_path: PathBuf,
    pub datapacks_dir: PathBuf,
    pub backups_root: PathBuf,
    pub backups: crate::config::BackupsConfig,
}

impl DataPaths {
    pub fn for_world(
        game_dir: &Path,
        world_dir: &Path,
        data_dir: &Path,
        backups: crate::config::BackupsConfig,
    ) -> DataPaths {
        DataPaths {
            game_dir: game_dir.to_path_buf(),
            world_dir: world_dir.to_path_buf(),
            level_dat_path: world_dir.join("level.dat"),
            datapacks_dir: world_dir.join("datapacks"),
            backups_root: level_backup::root_for(data_dir, game_dir),
            backups,
        }
    }

    pub fn world_name(&self) -> String {
        self.world_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("world")
            .to_string()
    }
}

/// The result of one attempted write.
#[derive(Debug, Clone)]
pub struct DataWrite {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub backup: LevelBackupOutcome,
    pub wrote: bool,
}

/// What an import did. `Locked` means the file was copied but left disabled
/// because the world was locked; the UI explains why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DatapackImportOutcome {
    Imported {
        filename: String,
    },
    Locked {
        filename: String,
        detail: String,
    },
    Duplicate {
        name: String,
        sha256: String,
        existing: String,
    },
    Collision {
        name: String,
    },
    Skipped,
}

/// A pack filename must be a plain direct child of `datapacks`.
pub fn validate_filename(filename: &str) -> Result<(), DataStackError> {
    let unsafe_name = filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0');
    if unsafe_name {
        return Err(DataStackError::InvalidFilename {
            filename: filename.to_string(),
        });
    }
    Ok(())
}

/// The exact `file/<filename>` value Minecraft writes.
pub fn file_value(filename: &str) -> Result<String, DataStackError> {
    validate_filename(filename)?;
    Ok(format!("file/{filename}"))
}

fn is_file_entry(value: &str) -> bool {
    value.starts_with("file/")
}

/// Enable: drop `file/<filename>` from `Disabled`, append to `Enabled` when
/// absent. Idempotent. Foreign entries are untouched.
pub fn enable(
    enabled: &mut Vec<String>,
    disabled: &mut Vec<String>,
    filename: &str,
) -> Result<bool, DataStackError> {
    let value = file_value(filename)?;
    let removed = disabled.iter().filter(|entry| *entry == &value).count();
    disabled.retain(|entry| entry != &value);
    if enabled.contains(&value) {
        return Ok(removed > 0);
    }
    enabled.push(value);
    Ok(true)
}

/// Disable: drop every occurrence from both lists, then append one copy to
/// `Disabled`. Removal matches strings, so a stale entry whose file is gone
/// is still cleaned. A pack that is already disabled exactly once is a no-op.
pub fn disable(enabled: &mut Vec<String>, disabled: &mut Vec<String>, filename: &str) -> bool {
    let value = format!("file/{filename}");
    let was_enabled = enabled.iter().any(|entry| entry == &value);
    enabled.retain(|entry| entry != &value);
    let copies = disabled.iter().filter(|entry| *entry == &value).count();
    disabled.retain(|entry| entry != &value);
    disabled.push(value);
    was_enabled || copies != 1
}

/// Reorder: accept the full `Enabled` order, low to high. Only `file/`
/// entries may move. Foreign entries keep their indices and relative order,
/// and the file multiset is preserved, so duplicates survive a reorder.
pub fn reorder(current: &[String], order: &[String]) -> Result<(), DataStackError> {
    if order.len() != current.len() {
        return Err(DataStackError::OrderChanged);
    }
    for (index, (before, after)) in current.iter().zip(order.iter()).enumerate() {
        if !is_file_entry(before) && before != after {
            return Err(DataStackError::ForeignMoved {
                value: before.clone(),
                index,
            });
        }
    }
    let mut before_files: Vec<&String> = current.iter().filter(|e| is_file_entry(e)).collect();
    let mut after_files: Vec<&String> = order.iter().filter(|e| is_file_entry(e)).collect();
    before_files.sort();
    after_files.sort();
    if before_files != after_files {
        return Err(DataStackError::OrderChanged);
    }
    Ok(())
}

fn writable(game_dir: &Path, world_dir: &Path, guard: &dyn WriteGuard) -> Result<(), AppError> {
    guard.ensure_writable(game_dir)?;
    if let Some(locked) = game_lock::world_locked(world_dir) {
        return Err(AppError::GameRunning { running: locked });
    }
    Ok(())
}

/// Read the stack. Reads never block, even while the game runs.
pub fn load(paths: &DataPaths) -> Result<DataStack, AppError> {
    let bytes = std::fs::read(&paths.level_dat_path)?;
    let view = level_dat::parse(&bytes)?;
    Ok(stack_of(&view, paths))
}

fn stack_of(view: &level_dat::LevelDatView, paths: &DataPaths) -> DataStack {
    let status = game_lock::check(&paths.game_dir);
    DataStack {
        enabled: view
            .enabled
            .iter()
            .map(|v| DataStackEntry::parse(v))
            .collect(),
        disabled: view
            .disabled
            .iter()
            .map(|v| DataStackEntry::parse(v))
            .collect(),
        world_locked: game_lock::world_locked(&paths.world_dir),
        running: status.running,
        stale_locks: status.stale_locks,
        detection_supported: status.supported,
    }
}

/// Enable or disable one datapack by file name.
pub fn set_enabled(
    paths: &DataPaths,
    filename: &str,
    enabled: bool,
) -> Result<DataStack, AppError> {
    set_enabled_with_guard(paths, filename, enabled, &SystemGuard)
}

#[doc(hidden)]
pub fn set_enabled_with_guard(
    paths: &DataPaths,
    filename: &str,
    enabled: bool,
    guard: &dyn WriteGuard,
) -> Result<DataStack, AppError> {
    validate_filename(filename).map_err(AppError::from)?;
    let change = Change::Toggle {
        filename: filename.to_string(),
        enabled,
    };
    let write = commit(paths, None, &change, guard)?;
    Ok(stack_of_lists(&write, paths))
}

/// Reorder `Enabled` to `order`, low to high. `expected` is what the UI
/// rendered; a request against a changed file set comes back as stale.
pub fn reorder_stack(
    paths: &DataPaths,
    order: &[String],
    expected: &DataSnapshot,
) -> Result<DataStack, AppError> {
    let change = Change::Reorder {
        order: order.to_vec(),
    };
    let write = commit(paths, Some(expected), &change, &SystemGuard)?;
    Ok(stack_of_lists(&write, paths))
}

/// Remove every reference to a deleted pack from both lists.
pub fn remove_references(paths: &DataPaths, filename: &str) -> Result<bool, AppError> {
    if !paths.level_dat_path.exists() {
        return Ok(false);
    }
    let change = Change::Remove {
        filename: filename.to_string(),
    };
    Ok(commit(paths, None, &change, &SystemGuard)?.wrote)
}

/// Restore one backup over `level.dat` and refresh the stack.
pub fn restore_backup(paths: &DataPaths, file_name: &str) -> Result<DataStack, AppError> {
    writable(&paths.game_dir, &paths.world_dir, &SystemGuard)?;
    let outcome = level_backup::restore_guarded(
        &paths.backups_root,
        &paths.world_name(),
        &paths.level_dat_path,
        file_name,
        &paths.backups,
        || writable(&paths.game_dir, &paths.world_dir, &SystemGuard),
    )?;
    let _ = outcome;
    load(paths)
}

/// What a guarded delete did before moving the pack to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatapackDeleteOutcome {
    pub removed_references: bool,
}

/// Delete one datapack safely: guard, clean both lists, guard again, trash.
/// A trash failure leaves the pack installed but disabled.
pub fn delete_datapack_safely(
    paths: &DataPaths,
    path: &Path,
    trash: &dyn TrashService,
) -> Result<DatapackDeleteOutcome, AppError> {
    delete_datapack_safely_with_guard(paths, path, trash, &SystemGuard)
}

#[doc(hidden)]
pub fn delete_datapack_safely_with_guard(
    paths: &DataPaths,
    path: &Path,
    trash: &dyn TrashService,
    guard: &dyn WriteGuard,
) -> Result<DatapackDeleteOutcome, AppError> {
    pack_scan::validate_delete_target(path, &paths.datapacks_dir)?;
    writable(&paths.game_dir, &paths.world_dir, guard)?;
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::from(DataStackError::InvalidFilename {
                filename: path.display().to_string(),
            })
        })?;
    let removed_references = remove_references(paths, &filename)?;
    writable(&paths.game_dir, &paths.world_dir, guard)?;
    pack_scan::delete_pack_with(path, &paths.datapacks_dir, trash)?;
    Ok(DatapackDeleteOutcome { removed_references })
}

/// Import a zip or directory datapack into `saves/<world>/datapacks`.
/// Validates `pack.mcmeta` and a `data` root; rejects resource-only packs and
/// symlinks. When the world is locked the file is copied but left disabled.
pub fn import_datapack(
    paths: &DataPaths,
    source: &Path,
    choice: ImportChoice,
    icons: &crate::icon_cache::IconCache,
    hashes: &HashCache,
) -> Result<DatapackImportOutcome, AppError> {
    import_datapack_with(
        paths,
        source,
        choice,
        icons,
        hashes,
        &SystemTrash,
        &SystemGuard,
    )
}

#[doc(hidden)]
pub fn import_datapack_with(
    paths: &DataPaths,
    source: &Path,
    choice: ImportChoice,
    icons: &crate::icon_cache::IconCache,
    hashes: &HashCache,
    trash: &dyn TrashService,
    guard: &dyn WriteGuard,
) -> Result<DatapackImportOutcome, AppError> {
    reject_symlink_source(source)?;
    let location = source_location(source)?;
    require_data_root(source, location)?;
    std::fs::create_dir_all(&paths.datapacks_dir)?;
    let outcome =
        pack_scan::import_pack_with(source, &paths.datapacks_dir, icons, choice, hashes, trash)?;
    let filename = match &outcome {
        crate::pack::ImportOutcome::Imported { pack } => pack_filename(&pack.path)?,
        crate::pack::ImportOutcome::Duplicate { .. }
        | crate::pack::ImportOutcome::Collision { .. }
        | crate::pack::ImportOutcome::Skipped => {
            return Ok(map_pack_outcome(&outcome));
        }
    };
    if writable(&paths.game_dir, &paths.world_dir, guard).is_err() {
        let status = game_lock::check(&paths.game_dir);
        let detail = status
            .running
            .map(|r| r.detail.clone())
            .or_else(|| {
                game_lock::world_locked(&paths.world_dir).map(|running| running.detail.clone())
            })
            .unwrap_or_else(|| "the world is locked".to_string());
        return Ok(DatapackImportOutcome::Locked { filename, detail });
    }
    set_enabled_with_guard(paths, &filename, true, guard)?;
    Ok(DatapackImportOutcome::Imported { filename })
}

fn map_pack_outcome(outcome: &crate::pack::ImportOutcome) -> DatapackImportOutcome {
    match outcome {
        crate::pack::ImportOutcome::Duplicate {
            name,
            sha256,
            existing,
        } => DatapackImportOutcome::Duplicate {
            name: name.clone(),
            sha256: sha256.clone(),
            existing: existing.clone(),
        },
        crate::pack::ImportOutcome::Collision { name } => {
            DatapackImportOutcome::Collision { name: name.clone() }
        }
        crate::pack::ImportOutcome::Skipped => DatapackImportOutcome::Skipped,
        crate::pack::ImportOutcome::Imported { .. } => {
            unreachable!("imported is handled by the caller")
        }
    }
}

fn pack_filename(path: &Path) -> Result<String, AppError> {
    path.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::from(DataStackError::InvalidFilename {
                filename: path.display().to_string(),
            })
        })
}

fn reject_symlink_source(source: &Path) -> Result<(), AppError> {
    let meta = std::fs::symlink_metadata(source).map_err(|_| PackError::NotFound {
        path: source.to_path_buf(),
    })?;
    if meta.file_type().is_symlink() {
        return Err(PackError::Symlink {
            path: source.to_path_buf(),
        }
        .into());
    }
    Ok(())
}

fn source_location(source: &Path) -> Result<crate::pack::PackLocation, AppError> {
    use crate::pack::PackLocation;
    let canonical = std::fs::canonicalize(source).map_err(|_| PackError::NotFound {
        path: source.to_path_buf(),
    })?;
    if canonical.is_dir() {
        reject_dir_symlinks(&canonical)?;
        return Ok(PackLocation::Directory);
    }
    if canonical
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        return Ok(PackLocation::Zip);
    }
    Err(PackError::UnknownKind {
        name: display_name(source),
    }
    .into())
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn reject_dir_symlinks(dir: &Path) -> Result<(), AppError> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|_| PackError::NotFound {
            path: current.clone(),
        })?;
        for entry in entries.filter_map(|e| e.ok()) {
            let file_type = entry
                .file_type()
                .map_err(|_| PackError::NotFound { path: entry.path() })?;
            if file_type.is_symlink() {
                return Err(PackError::Symlink { path: entry.path() }.into());
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    Ok(())
}

/// A datapack must carry a `data` root and a `pack.mcmeta`. A resource-only
/// pack is rejected before anything is copied.
fn require_data_root(source: &Path, location: crate::pack::PackLocation) -> Result<(), AppError> {
    use crate::pack::PackLocation;
    let name = display_name(source);
    let (has_data, has_meta) = match location {
        PackLocation::Directory => {
            let has_data = source.join("data").is_dir();
            let has_meta = source.join("pack.mcmeta").is_file();
            (has_data, has_meta)
        }
        PackLocation::Zip => zip_roots(source, &name)?,
    };
    if !has_meta {
        return Err(PackError::MissingMeta { name }.into());
    }
    if !has_data {
        return Err(AppError::from(DataStackError::ResourceOnly { name }));
    }
    Ok(())
}

fn zip_roots(source: &Path, name: &str) -> Result<(bool, bool), AppError> {
    let file = std::fs::File::open(source).map_err(|_| PackError::NotFound {
        path: source.to_path_buf(),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| PackError::BrokenArchive {
        name: name.to_string(),
        reason: "invalid Zip archive".to_string(),
    })?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(PackError::BrokenArchive {
            name: name.to_string(),
            reason: "archive has too many entries".to_string(),
        }
        .into());
    }
    let mut has_data = false;
    let mut has_meta = false;
    let mut meta_bytes: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| PackError::BrokenArchive {
                name: name.to_string(),
                reason: "invalid Zip archive".to_string(),
            })?;
        let entry_name = entry.name().to_string();
        if entry_name == "pack.mcmeta" || entry_name.starts_with("data/") {
            if entry_name == "pack.mcmeta" {
                has_meta = true;
                meta_bytes = entry.size();
            } else {
                has_data = true;
            }
        } else if entry_name.starts_with("assets/") {
            // Resource roots are allowed alongside data; they do not decide.
        }
        if has_data && has_meta {
            break;
        }
    }
    if meta_bytes > MAX_META_BYTES {
        return Err(PackError::BrokenArchive {
            name: name.to_string(),
            reason: "pack.mcmeta is too large".to_string(),
        }
        .into());
    }
    let _ = MAX_ICON_BYTES;
    Ok((has_data, has_meta))
}

/// List the installed datapacks of one world: a zip or a directory with
/// `pack.mcmeta` directly under `datapacks`. Broken entries become error
/// cards so one bad pack never hides the rest.
pub fn scan_datapacks(
    datapacks: &Path,
    icons: &crate::icon_cache::IconCache,
) -> Result<PackListing, AppError> {
    pack_scan::scan_packs(datapacks, icons)
}

fn stack_of_lists(write: &DataWrite, paths: &DataPaths) -> DataStack {
    let status = game_lock::check(&paths.game_dir);
    DataStack {
        enabled: write
            .enabled
            .iter()
            .map(|v| DataStackEntry::parse(v))
            .collect(),
        disabled: write
            .disabled
            .iter()
            .map(|v| DataStackEntry::parse(v))
            .collect(),
        world_locked: game_lock::world_locked(&paths.world_dir),
        running: status.running,
        stale_locks: status.stale_locks,
        detection_supported: status.supported,
    }
}

enum Change {
    Toggle { filename: String, enabled: bool },
    Reorder { order: Vec<String> },
    Remove { filename: String },
}

impl Change {
    fn apply(
        &self,
        enabled: &mut Vec<String>,
        disabled: &mut Vec<String>,
    ) -> Result<bool, DataStackError> {
        match self {
            Change::Toggle {
                filename,
                enabled: true,
            } => enable(enabled, disabled, filename),
            Change::Toggle {
                filename,
                enabled: false,
            } => {
                validate_filename(filename)?;
                Ok(disable(enabled, disabled, filename))
            }
            Change::Remove { filename } => {
                let value = format!("file/{filename}");
                validate_filename(filename)?;
                let before = enabled.len() + disabled.len();
                enabled.retain(|entry| entry != &value);
                disabled.retain(|entry| entry != &value);
                Ok(enabled.len() + disabled.len() != before)
            }
            Change::Reorder { order } => {
                reorder(enabled, order)?;
                if enabled.as_slice() == order.as_slice() {
                    return Ok(false);
                }
                *enabled = order.clone();
                Ok(true)
            }
        }
    }

    fn order_related(&self) -> bool {
        matches!(self, Change::Reorder { .. })
    }
}

/// Run one change end to end: game guard plus the per-world lock, re-read,
/// apply to the latest content, backup, atomic write. The caller holds the
/// write mutex for the game dir. An external edit that races the write is
/// rebased once; a second race comes back as stale.
fn commit(
    paths: &DataPaths,
    expected: Option<&DataSnapshot>,
    change: &Change,
    guard: &dyn WriteGuard,
) -> Result<DataWrite, AppError> {
    writable(&paths.game_dir, &paths.world_dir, guard)?;

    for attempt in 0..2 {
        let before = std::fs::read(&paths.level_dat_path)?;
        let view = level_dat::parse(&before)?;
        let mut enabled = view.enabled.clone();
        let mut disabled = view.disabled.clone();
        let stale = expected
            .is_some_and(|snapshot| snapshot.enabled != enabled || snapshot.disabled != disabled);

        let applied = change.apply(&mut enabled, &mut disabled);
        let changed = match applied {
            Ok(changed) => changed,
            Err(_) if stale && change.order_related() => {
                return Err(AppError::from(DataStackError::StaleFile));
            }
            Err(err) => return Err(AppError::from(err)),
        };

        if !changed {
            return Ok(DataWrite {
                enabled,
                disabled,
                backup: LevelBackupOutcome {
                    created: None,
                    skipped: !paths.backups.enabled,
                },
                wrote: false,
            });
        }

        if std::fs::read(&paths.level_dat_path).ok().as_deref() != Some(before.as_slice()) {
            // An external writer landed between our read and our write.
            if attempt == 1 {
                return Err(AppError::from(DataStackError::StaleFile));
            }
            continue;
        }

        // A failed backup aborts the write; the previous bytes stay intact.
        let backup_outcome = level_backup::create(
            &paths.backups_root,
            &paths.world_name(),
            &before,
            &paths.backups,
        )?;
        // Re-check after the backup and immediately before the replace. A game
        // that started during the backup leaves the old bytes in place.
        writable(&paths.game_dir, &paths.world_dir, guard)?;
        let rendered =
            level_dat::render_with(&before, &enabled, &disabled).map_err(AppError::from)?;
        level_dat::write_level_dat(&paths.world_dir, &rendered)?;
        return Ok(DataWrite {
            enabled,
            disabled,
            backup: backup_outcome,
            wrote: true,
        });
    }
    Err(AppError::from(DataStackError::StaleFile))
}

impl From<DataStackError> for AppError {
    fn from(err: DataStackError) -> Self {
        AppError::DataStack(err)
    }
}

impl From<LevelDatError> for AppError {
    fn from(err: LevelDatError) -> Self {
        AppError::LevelDat(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrays(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn enable_moves_from_disabled_to_enabled() {
        let mut enabled = arrays(&["vanilla"]);
        let mut disabled = arrays(&["file/a.zip"]);
        assert!(enable(&mut enabled, &mut disabled, "a.zip").unwrap());
        assert_eq!(enabled, arrays(&["vanilla", "file/a.zip"]));
        assert!(disabled.is_empty());
    }

    #[test]
    fn enable_is_idempotent() {
        let mut enabled = arrays(&["vanilla", "file/a.zip"]);
        let mut disabled: Vec<String> = Vec::new();
        assert!(!enable(&mut enabled, &mut disabled, "a.zip").unwrap());
    }

    #[test]
    fn disable_moves_to_disabled_and_drops_duplicates() {
        let mut enabled = arrays(&["file/a.zip", "vanilla", "file/a.zip"]);
        let mut disabled = arrays(&["file/a.zip"]);
        assert!(disable(&mut enabled, &mut disabled, "a.zip"));
        assert_eq!(enabled, arrays(&["vanilla"]));
        assert_eq!(disabled, arrays(&["file/a.zip"]));
    }

    #[test]
    fn foreign_entries_cannot_move() {
        let current = arrays(&["vanilla", "fabric", "file/a.zip"]);
        let moved = arrays(&["fabric", "vanilla", "file/a.zip"]);
        let err = reorder(&current, &moved).unwrap_err();
        assert_eq!(err.code(), "level_dat");
    }

    #[test]
    fn reorder_preserves_foreign_positions() {
        let current = arrays(&["vanilla", "fabric", "file/a.zip", "file/b.zip"]);
        let order = arrays(&["vanilla", "fabric", "file/b.zip", "file/a.zip"]);
        reorder(&current, &order).unwrap();
    }

    #[test]
    fn reorder_rejects_a_changed_file_set() {
        let current = arrays(&["vanilla", "file/a.zip"]);
        let dropped = arrays(&["vanilla", "file/b.zip"]);
        assert_eq!(
            reorder(&current, &dropped).unwrap_err(),
            DataStackError::OrderChanged
        );
    }

    #[test]
    fn filenames_are_validated() {
        for name in ["", ".", "..", "sub/a.zip", "a\0b"] {
            assert!(enable(&mut Vec::new(), &mut Vec::new(), name).is_err());
        }
        assert!(enable(&mut Vec::new(), &mut Vec::new(), "Cool Pack.zip").is_ok());
    }

    #[test]
    fn stale_file_code_matches_the_wire() {
        assert_eq!(DataStackError::StaleFile.code(), "stale_stack");
    }
}
