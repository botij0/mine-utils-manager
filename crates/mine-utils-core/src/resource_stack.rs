//! The enabled resource pack stack: what `options.txt` calls `resourcePacks`.
//!
//! The array is load order, low to high: the last entry wins. The UI shows the
//! reverse, so the top card is the highest priority. Entries that are not
//! `file/<filename>` (`vanilla`, `fabric`, `fabric-convention-tags-v2`, …) are
//! foreign and locked: they never move, toggle, or disappear.
//!
//! Every write goes through the same sequence: block while the game runs,
//! re-read the file, apply the change to the latest content, copy the previous
//! bytes to a backup, then write atomically. A change whose file set moved out
//! from under the UI comes back as `StaleStack` so the UI refreshes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backup::{self, BackupOutcome};
use crate::config::BackupsConfig;
use crate::error::AppError;
use crate::game_lock;
use crate::options_txt::{self, OptionsTxt};
use crate::pack_scan::{delete_pack_with, validate_delete_target, TrashService};

/// The parts of a two-array pack stack that do not depend on which stack it is:
/// entry parsing, filename rules, and reorder validation. The resource and
/// datapack stacks share this module so each invariant has one definition.
pub(crate) mod stack_core {
    use serde::{Deserialize, Serialize};

    use crate::error::AppError;

    /// One `file/<filename>` entry. The wire tags each entry with `kind`, and
    /// the frontend mirrors this shape in `src/lib/types.ts`.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Entry {
        /// The exact `file/<filename>` value plus the filename alone. The
        /// filename is a direct child of the pack folder, never `pack.name`,
        /// which strips `.zip`.
        File { value: String, filename: String },
        /// A locked string the app never edits, such as `vanilla`.
        Foreign { value: String },
    }

    impl Entry {
        pub fn parse(value: &str) -> Entry {
            match value.strip_prefix("file/") {
                Some(filename) => Entry::File {
                    value: value.to_string(),
                    filename: filename.to_string(),
                },
                None => Entry::Foreign {
                    value: value.to_string(),
                },
            }
        }

        pub fn value(&self) -> &str {
            match self {
                Entry::File { value, .. } => value,
                Entry::Foreign { value } => value,
            }
        }

        pub fn is_file(&self) -> bool {
            matches!(self, Entry::File { .. })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum EntryError {
        InvalidFilename { filename: String },
        ForeignMoved { value: String, index: usize },
        OrderChanged,
    }

    /// A pack filename must be a plain direct child of the pack folder. Only
    /// path separators, NUL, and the two directory names are rejected; dots
    /// inside a name are ordinary characters, so `Epic..Pack.zip` is valid.
    pub fn validate_filename(filename: &str) -> Result<(), EntryError> {
        let unsafe_name = filename.is_empty()
            || filename == "."
            || filename == ".."
            || filename.contains('/')
            || filename.contains('\\')
            || filename.contains('\0');
        if unsafe_name {
            return Err(EntryError::InvalidFilename {
                filename: filename.to_string(),
            });
        }
        Ok(())
    }

    /// The exact `file/<filename>` value Minecraft writes.
    pub fn file_value(filename: &str) -> Result<String, EntryError> {
        validate_filename(filename)?;
        Ok(format!("file/{filename}"))
    }

    pub fn is_file_entry(value: &str) -> bool {
        value.starts_with("file/")
    }

    /// Accept the full array order, low to high. Only `file/` entries may move.
    /// Foreign entries keep their indices and relative order, and the file
    /// multiset is preserved, so duplicates survive a reorder.
    pub fn reorder(current: &[String], order: &[String]) -> Result<(), EntryError> {
        if order.len() != current.len() {
            return Err(EntryError::OrderChanged);
        }
        for (index, (before, after)) in current.iter().zip(order.iter()).enumerate() {
            if !is_file_entry(before) && before != after {
                return Err(EntryError::ForeignMoved {
                    value: before.clone(),
                    index,
                });
            }
        }
        let mut before_files: Vec<&String> = current
            .iter()
            .filter(|entry| is_file_entry(entry))
            .collect();
        let mut after_files: Vec<&String> =
            order.iter().filter(|entry| is_file_entry(entry)).collect();
        before_files.sort();
        after_files.sort();
        if before_files != after_files {
            return Err(EntryError::OrderChanged);
        }
        Ok(())
    }

    /// The two string arrays a stack owns. `primary` is the load order;
    /// `secondary` is the kind-specific companion (`incompatibleResourcePacks`
    /// or `DataPacks.Disabled`).
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct Lists {
        pub primary: Vec<String>,
        pub secondary: Vec<String>,
    }

    /// The two arrays the UI rendered.
    #[derive(Debug, Clone, Copy)]
    pub struct Expected<'a> {
        pub primary: &'a [String],
        pub secondary: &'a [String],
    }

    /// The mutation vocabulary both stacks share. The per-kind meaning of each
    /// operation lives in [`Rules`].
    #[derive(Debug, Clone)]
    pub enum Change {
        Toggle { filename: String, enabled: bool },
        Reorder { order: Vec<String> },
        Remove { filename: String },
    }

    impl Change {
        pub fn apply(&self, lists: &mut Lists, rules: Rules) -> Result<bool, EntryError> {
            match self {
                Change::Toggle {
                    filename,
                    enabled: true,
                } => (rules.enable)(lists, filename),
                Change::Toggle {
                    filename,
                    enabled: false,
                } => Ok((rules.disable)(lists, filename)),
                Change::Remove { filename } => (rules.remove)(lists, filename),
                Change::Reorder { order } => {
                    reorder(&lists.primary, order)?;
                    if lists.primary == *order {
                        return Ok(false);
                    }
                    lists.primary.clone_from(order);
                    Ok(true)
                }
            }
        }

        pub fn order_related(&self) -> bool {
            matches!(self, Change::Reorder { .. })
        }
    }

    /// The mutations that differ between the stacks.
    #[derive(Clone, Copy)]
    pub struct Rules {
        pub enable: fn(&mut Lists, &str) -> Result<bool, EntryError>,
        pub disable: fn(&mut Lists, &str) -> bool,
        pub remove: fn(&mut Lists, &str) -> Result<bool, EntryError>,
    }

    /// One stack file: how to read, render, back up, and replace it. The commit
    /// driver owns the guard and rebase sequence; each stack owns only these
    /// file-specific steps.
    pub trait StackFile {
        type Source;
        type Backup;
        type Rendered;

        fn prepare(&self) -> Result<(), AppError>;
        fn guard(&self) -> Result<(), AppError>;
        fn read(&self) -> Result<(Self::Source, Lists), AppError>;
        fn bytes<'a>(&self, source: &'a Self::Source) -> &'a [u8];
        fn render(&self, source: &Self::Source, lists: &Lists) -> Result<Self::Rendered, AppError>;
        fn write(&self, rendered: Self::Rendered) -> Result<(), AppError>;
        fn backup(&self, previous: &[u8]) -> Result<Self::Backup, AppError>;
        fn skipped_backup(&self) -> Self::Backup;
        fn unchanged(&self, previous: &[u8]) -> Result<bool, AppError>;
        fn unchanged_before_write(&self, previous: &[u8]) -> Result<bool, AppError> {
            self.unchanged(previous)
        }
        fn reject(&self, err: EntryError) -> AppError;
        fn stale(&self) -> AppError;
        fn rules(&self) -> Rules;
    }

    pub struct Committed<B> {
        pub lists: Lists,
        pub backup: B,
        pub wrote: bool,
    }

    /// Run one change end to end: prepare, re-read, apply to the latest content,
    /// back up, and replace atomically. An external edit that races the write is
    /// rebased once; a second race comes back as stale.
    pub fn commit<F: StackFile>(
        file: &F,
        expected: Option<Expected<'_>>,
        change: &Change,
    ) -> Result<Committed<F::Backup>, AppError> {
        file.prepare()?;

        for attempt in 0..2 {
            let (source, mut lists) = file.read()?;
            let stale = expected.is_some_and(|expected| {
                lists.primary != expected.primary || lists.secondary != expected.secondary
            });

            let changed = match change.apply(&mut lists, file.rules()) {
                Ok(changed) => changed,
                Err(_) if stale && change.order_related() => return Err(file.stale()),
                Err(err) => return Err(file.reject(err)),
            };

            if !changed {
                return Ok(Committed {
                    lists,
                    backup: file.skipped_backup(),
                    wrote: false,
                });
            }

            let previous = file.bytes(&source);
            if !file.unchanged(previous)? {
                // An external writer landed between our read and our write.
                if attempt == 1 {
                    return Err(file.stale());
                }
                continue;
            }

            // A failed backup aborts the write; the previous bytes stay intact.
            let backup = file.backup(previous)?;
            // Re-check after the backup and immediately before the replace. A
            // game that started during the backup leaves the old bytes in place.
            file.guard()?;
            let rendered = file.render(&source, &lists)?;
            if !file.unchanged_before_write(previous)? {
                if attempt == 1 {
                    return Err(file.stale());
                }
                continue;
            }
            file.write(rendered)?;
            return Ok(Committed {
                lists,
                backup,
                wrote: true,
            });
        }
        Err(file.stale())
    }
}

/// The game-running check a write performs before touching disk. Production
/// uses the real process and session-lock probe; tests inject a scripted guard
/// to prove that the re-checks after the backup and before the trash still run.
pub trait WriteGuard {
    fn ensure_writable(&self, game_dir: &Path) -> Result<(), AppError>;
}

/// The real guard: both independent checks in [`game_lock`].
pub struct SystemGuard;

impl WriteGuard for SystemGuard {
    fn ensure_writable(&self, game_dir: &Path) -> Result<(), AppError> {
        game_lock::ensure_writable(game_dir)
    }
}

/// Everything that can go wrong with a stack change request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StackError {
    #[error("{filename} is not a safe resource pack file name")]
    InvalidFilename { filename: String },
    #[error("the locked entry {value} cannot move; only file/ entries can be reordered")]
    ForeignMoved { value: String, index: usize },
    #[error("the requested order is not a reorder of the current stack")]
    OrderChanged,
    #[error("the stack changed on disk; reload it and try again")]
    StaleStack,
}

impl StackError {
    pub fn code(&self) -> &'static str {
        match self {
            StackError::StaleStack => "stale_stack",
            _ => "resource_stack",
        }
    }
}

pub use stack_core::Entry as StackEntry;

/// The active stack as the app sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceStack {
    /// Load order, low to high. The UI shows the reverse, so the top card is
    /// the highest priority and loads last.
    pub entries: Vec<StackEntry>,
    /// The `incompatibleResourcePacks` set as written.
    pub incompatible: Vec<String>,
    /// The running-game state. Writes are blocked while this is set.
    pub running: Option<game_lock::GameRunning>,
    /// Stale `session.lock` files. They never block a write.
    pub stale_locks: Vec<PathBuf>,
    /// False where POSIX lock probing is unsupported.
    pub detection_supported: bool,
}

/// What the UI rendered. A mutation that lands on a different file set comes
/// back as [`StackError::StaleStack`] instead of clobbering the new content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackSnapshot {
    pub resource: Vec<String>,
    pub incompatible: Vec<String>,
}

impl ResourceStack {
    pub fn snapshot(&self) -> StackSnapshot {
        StackSnapshot {
            resource: self
                .entries
                .iter()
                .map(|entry| entry.value().to_string())
                .collect(),
            incompatible: self.incompatible.clone(),
        }
    }
}

/// Where a stack lives and which backup policy applies to its writes.
#[derive(Debug, Clone)]
pub struct StackPaths {
    pub game_dir: PathBuf,
    pub options_path: PathBuf,
    pub backups_root: PathBuf,
    pub backups: BackupsConfig,
}

impl StackPaths {
    pub fn for_game_dir(game_dir: &Path, data_dir: &Path, backups: BackupsConfig) -> StackPaths {
        StackPaths {
            game_dir: game_dir.to_path_buf(),
            options_path: game_dir.join("options.txt"),
            backups_root: backup::root_for(data_dir, game_dir),
            backups,
        }
    }
}

/// The result of one attempted write.
#[derive(Debug, Clone)]
pub struct StackWrite {
    pub resource: Vec<String>,
    pub incompatible: Vec<String>,
    pub backup: BackupOutcome,
    pub wrote: bool,
}

/// A pack filename must be a plain direct child of `resourcepacks`.
pub fn validate_filename(filename: &str) -> Result<(), StackError> {
    stack_core::validate_filename(filename).map_err(StackError::from)
}

/// The exact `file/<filename>` value Minecraft writes.
pub fn file_value(filename: &str) -> Result<String, StackError> {
    stack_core::file_value(filename).map_err(StackError::from)
}

/// The filename of a pack on disk. This is the basename of `pack.path`, so a
/// zip keeps its extension and a directory keeps its folder name. The name is
/// only ever matched against what is already written in the arrays.
pub fn filename_for(path: &Path) -> Result<String, StackError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| StackError::InvalidFilename {
            filename: path.display().to_string(),
        })
}

/// Enable: append `file/<filename>` when absent. Idempotent. This is the only
/// place a new reference to a pack file is created, so this is where the
/// filename is validated.
pub fn enable(resource: &mut Vec<String>, filename: &str) -> Result<bool, StackError> {
    enable_entry(resource, filename).map_err(StackError::from)
}

/// Disable: drop every occurrence from both arrays. Removal only matches
/// strings, so it works for a stale entry whose pack file is already gone and
/// for names no new reference would be allowed to use.
pub fn disable(resource: &mut Vec<String>, incompatible: &mut Vec<String>, filename: &str) -> bool {
    let value = format!("file/{filename}");
    let before = resource.len() + incompatible.len();
    resource.retain(|entry| entry != &value);
    incompatible.retain(|entry| entry != &value);
    resource.len() + incompatible.len() != before
}

/// Reorder: accept the full array order, low to high. Only `file/` entries may
/// move. Foreign entries keep their indices and relative order, and the file
/// multiset is preserved, so duplicates survive a reorder.
pub fn reorder(current: &[String], order: &[String]) -> Result<(), StackError> {
    stack_core::reorder(current, order).map_err(StackError::from)
}

impl From<stack_core::EntryError> for StackError {
    fn from(err: stack_core::EntryError) -> Self {
        match err {
            stack_core::EntryError::InvalidFilename { filename } => {
                StackError::InvalidFilename { filename }
            }
            stack_core::EntryError::ForeignMoved { value, index } => {
                StackError::ForeignMoved { value, index }
            }
            stack_core::EntryError::OrderChanged => StackError::OrderChanged,
        }
    }
}

/// Read the stack. Reads never block, even while the game runs.
pub fn load(paths: &StackPaths) -> Result<ResourceStack, AppError> {
    let txt = options_txt::read_options(&paths.options_path)?;
    Ok(stack_from(&txt, &game_lock::check(&paths.game_dir)))
}

fn stack_from(txt: &OptionsTxt, status: &game_lock::GameStatus) -> ResourceStack {
    ResourceStack {
        entries: txt
            .resource_packs()
            .iter()
            .map(|v| StackEntry::parse(v))
            .collect(),
        incompatible: txt.incompatible_packs().to_vec(),
        running: status.running.clone(),
        stale_locks: status.stale_locks.clone(),
        detection_supported: status.supported,
    }
}

/// Enable or disable one pack. The mutation is applied to the latest content,
/// so an external edit to any other line survives.
pub fn set_enabled(
    paths: &StackPaths,
    filename: &str,
    enabled: bool,
) -> Result<ResourceStack, AppError> {
    set_enabled_with_guard(paths, filename, enabled, &SystemGuard)
}

/// [`set_enabled`] with an injected guard, so tests can fail the check that
/// runs after the backup and prove the old bytes survive.
#[doc(hidden)]
pub fn set_enabled_with_guard(
    paths: &StackPaths,
    filename: &str,
    enabled: bool,
    guard: &dyn WriteGuard,
) -> Result<ResourceStack, AppError> {
    let change = stack_core::Change::Toggle {
        filename: filename.to_string(),
        enabled,
    };
    let write = commit(paths, None, &change, guard)?;
    if write.wrote {
        load(paths)
    } else {
        Ok(resource_stack_of(paths, &write)?)
    }
}

/// Reorder the stack to `order`, low to high. `expected` is what the UI
/// rendered; when the file set moved on disk and the request no longer fits,
/// the call fails with [`StackError::StaleStack`] so the UI refreshes.
pub fn reorder_stack(
    paths: &StackPaths,
    order: &[String],
    expected: &StackSnapshot,
) -> Result<ResourceStack, AppError> {
    let change = stack_core::Change::Reorder {
        order: order.to_vec(),
    };
    let write = commit(paths, Some(expected), &change, &SystemGuard)?;
    if write.wrote {
        load(paths)
    } else {
        Ok(resource_stack_of(paths, &write)?)
    }
}

/// Remove every reference to a deleted pack from both arrays. The backend
/// delete path calls this before trashing the file, so the arrays can never
/// point at a trashed pack. Returns whether anything was written.
pub fn remove_references(paths: &StackPaths, filename: &str) -> Result<bool, AppError> {
    if !paths.options_path.exists() {
        // Nothing can reference the pack when the file is missing.
        return Ok(false);
    }
    let change = stack_core::Change::Remove {
        filename: filename.to_string(),
    };
    Ok(commit(paths, None, &change, &SystemGuard)?.wrote)
}

/// Restore one backup over `options.txt` and refresh the stack.
pub fn restore_backup(paths: &StackPaths, file_name: &str) -> Result<ResourceStack, AppError> {
    game_lock::ensure_writable(&paths.game_dir)?;
    backup::restore_guarded(
        &paths.backups_root,
        &paths.options_path,
        file_name,
        &paths.backups,
        // Re-check after the pre-restore backup exists and immediately before
        // the current file is replaced.
        || game_lock::ensure_writable(&paths.game_dir),
    )?;
    load(paths)
}

/// What a guarded delete did before moving the pack to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub removed_references: bool,
}

/// Delete one pack safely. The target is validated as a direct child of the
/// resolved `resourcepacks` before anything else; the game guard runs before
/// the `options.txt` cleanup and again immediately before the trash move, so a
/// client that starts in between leaves the pack installed but disabled rather
/// than pointing at a trashed file. The injectable trash makes the ordering
/// testable.
pub fn delete_pack_safely(
    paths: &StackPaths,
    path: &Path,
    trash: &dyn TrashService,
) -> Result<DeleteOutcome, AppError> {
    delete_pack_safely_with_guard(paths, path, trash, &SystemGuard)
}

/// [`delete_pack_safely`] with an injected guard, so tests can fail the last
/// check and prove the references are cleaned but the pack is not trashed.
#[doc(hidden)]
pub fn delete_pack_safely_with_guard(
    paths: &StackPaths,
    path: &Path,
    trash: &dyn TrashService,
    guard: &dyn WriteGuard,
) -> Result<DeleteOutcome, AppError> {
    let resourcepacks = paths.game_dir.join("resourcepacks");
    validate_delete_target(path, &resourcepacks)?;
    guard.ensure_writable(&paths.game_dir)?;
    let filename = filename_for(path)?;
    // The cleanup write keeps its own real guard; the injected guard controls
    // the two outer checks that bracket it.
    let removed_references = remove_references(paths, &filename)?;
    guard.ensure_writable(&paths.game_dir)?;
    delete_pack_with(path, &resourcepacks, trash)?;
    Ok(DeleteOutcome { removed_references })
}

fn resource_stack_of(paths: &StackPaths, write: &StackWrite) -> Result<ResourceStack, AppError> {
    let status = game_lock::check(&paths.game_dir);
    Ok(ResourceStack {
        entries: write
            .resource
            .iter()
            .map(|v| StackEntry::parse(v))
            .collect(),
        incompatible: write.incompatible.clone(),
        running: status.running,
        stale_locks: status.stale_locks,
        detection_supported: status.supported,
    })
}

fn rules() -> stack_core::Rules {
    stack_core::Rules {
        enable: enable_lists,
        disable: disable_lists,
        remove: remove_lists,
    }
}

fn enable_lists(
    lists: &mut stack_core::Lists,
    filename: &str,
) -> Result<bool, stack_core::EntryError> {
    enable_entry(&mut lists.primary, filename)
}

fn enable_entry(
    resource: &mut Vec<String>,
    filename: &str,
) -> Result<bool, stack_core::EntryError> {
    let value = stack_core::file_value(filename)?;
    if resource.contains(&value) {
        return Ok(false);
    }
    resource.push(value);
    Ok(true)
}

fn disable_lists(lists: &mut stack_core::Lists, filename: &str) -> bool {
    disable(&mut lists.primary, &mut lists.secondary, filename)
}

fn remove_lists(
    lists: &mut stack_core::Lists,
    filename: &str,
) -> Result<bool, stack_core::EntryError> {
    Ok(disable_lists(lists, filename))
}

struct ResourceFile<'a> {
    paths: &'a StackPaths,
    guard: &'a dyn WriteGuard,
}

impl stack_core::StackFile for ResourceFile<'_> {
    type Source = OptionsTxt;
    type Backup = BackupOutcome;
    type Rendered = String;

    fn prepare(&self) -> Result<(), AppError> {
        self.guard.ensure_writable(&self.paths.game_dir)
    }

    fn guard(&self) -> Result<(), AppError> {
        self.guard.ensure_writable(&self.paths.game_dir)
    }

    fn read(&self) -> Result<(OptionsTxt, stack_core::Lists), AppError> {
        let txt = options_txt::read_options(&self.paths.options_path)?;
        let lists = stack_core::Lists {
            primary: txt.resource_packs().to_vec(),
            secondary: txt.incompatible_packs().to_vec(),
        };
        Ok((txt, lists))
    }

    fn bytes<'a>(&self, source: &'a OptionsTxt) -> &'a [u8] {
        source.source().as_bytes()
    }

    fn render(&self, source: &OptionsTxt, lists: &stack_core::Lists) -> Result<String, AppError> {
        Ok(source.render(&lists.primary, &lists.secondary))
    }

    fn write(&self, rendered: String) -> Result<(), AppError> {
        options_txt::write_options(&self.paths.options_path, &rendered)
    }

    fn backup(&self, previous: &[u8]) -> Result<BackupOutcome, AppError> {
        backup::create(&self.paths.backups_root, previous, &self.paths.backups)
    }

    fn skipped_backup(&self) -> BackupOutcome {
        BackupOutcome {
            created: None,
            skipped: !self.paths.backups.enabled,
        }
    }

    fn unchanged(&self, previous: &[u8]) -> Result<bool, AppError> {
        Ok(std::fs::read(&self.paths.options_path).ok().as_deref() == Some(previous))
    }

    fn reject(&self, err: stack_core::EntryError) -> AppError {
        AppError::Stack(err.into())
    }

    fn stale(&self) -> AppError {
        AppError::Stack(StackError::StaleStack)
    }

    fn rules(&self) -> stack_core::Rules {
        rules()
    }
}

/// Run one change end to end: game guard, re-read, apply to the latest
/// content, backup, atomic write. The caller holds the write mutex for the
/// game dir. An external edit that races the write is rebased once; a second
/// race comes back as [`StackError::StaleStack`].
fn commit(
    paths: &StackPaths,
    expected: Option<&StackSnapshot>,
    change: &stack_core::Change,
    guard: &dyn WriteGuard,
) -> Result<StackWrite, AppError> {
    let file = ResourceFile { paths, guard };
    let expected = expected.map(|snapshot| stack_core::Expected {
        primary: &snapshot.resource,
        secondary: &snapshot.incompatible,
    });
    let committed = stack_core::commit(&file, expected, change)?;
    Ok(StackWrite {
        resource: committed.lists.primary,
        incompatible: committed.lists.secondary,
        backup: committed.backup,
        wrote: committed.wrote,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrays(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    #[test]
    fn enable_is_idempotent() {
        let mut resource = arrays(&["vanilla", "file/a.zip"]);
        assert!(!enable(&mut resource, "a.zip").unwrap());
        assert_eq!(resource, arrays(&["vanilla", "file/a.zip"]));

        assert!(enable(&mut resource, "b.zip").unwrap());
        assert_eq!(resource, arrays(&["vanilla", "file/a.zip", "file/b.zip"]));
    }

    #[test]
    fn disable_removes_every_occurrence_from_both_arrays() {
        let mut resource = arrays(&["file/a.zip", "vanilla", "file/a.zip"]);
        let mut incompatible = arrays(&["file/a.zip"]);

        assert!(disable(&mut resource, &mut incompatible, "a.zip"));
        assert_eq!(resource, arrays(&["vanilla"]));
        assert!(incompatible.is_empty());

        assert!(!disable(&mut resource, &mut incompatible, "a.zip"));
    }

    #[test]
    fn disable_works_for_a_stale_entry() {
        let mut resource = arrays(&["file/gone.zip"]);
        let mut incompatible = Vec::new();

        assert!(disable(&mut resource, &mut incompatible, "gone.zip"));
        assert!(resource.is_empty());
    }

    #[test]
    fn foreign_entries_cannot_move() {
        let current = arrays(&["vanilla", "fabric", "file/a.zip"]);
        let moved = arrays(&["fabric", "vanilla", "file/a.zip"]);
        let err = reorder(&current, &moved).unwrap_err();
        assert_eq!(err.code(), "resource_stack");

        let crossed = arrays(&["vanilla", "file/a.zip", "fabric"]);
        let err = reorder(&current, &crossed).unwrap_err();
        assert_eq!(err.code(), "resource_stack");
    }

    #[test]
    fn reorder_preserves_foreign_positions() {
        let current = arrays(&["vanilla", "fabric", "file/a.zip", "file/b.zip"]);
        let order = arrays(&["vanilla", "fabric", "file/b.zip", "file/a.zip"]);
        reorder(&current, &order).unwrap();

        for (before, after) in current.iter().zip(order.iter()) {
            if !stack_core::is_file_entry(before) {
                assert_eq!(before, after, "foreign entries keep their index");
            }
        }
    }

    #[test]
    fn reorder_is_multiset_preserving() {
        let current = arrays(&["vanilla", "file/a.zip", "file/a.zip", "file/b.zip"]);
        let order = arrays(&["vanilla", "file/a.zip", "file/b.zip", "file/a.zip"]);
        reorder(&current, &order).unwrap();

        let dropped = arrays(&["vanilla", "file/a.zip", "file/b.zip", "file/c.zip"]);
        assert_eq!(
            reorder(&current, &dropped).unwrap_err(),
            StackError::OrderChanged
        );

        let shortened = arrays(&["vanilla", "file/a.zip", "file/b.zip"]);
        assert_eq!(
            reorder(&current, &shortened).unwrap_err(),
            StackError::OrderChanged
        );
    }

    #[test]
    fn foreign_subsequence_may_not_gain_entries() {
        let current = arrays(&["vanilla", "file/a.zip"]);
        let smuggled = arrays(&["vanilla", "fabric"]);
        assert!(reorder(&current, &smuggled).is_err());
    }

    #[test]
    fn filenames_are_validated() {
        for name in ["", ".", "..", "sub/a.zip", "sub\\a.zip", "a\0b"] {
            assert_eq!(
                enable(&mut Vec::new(), name).unwrap_err(),
                StackError::InvalidFilename {
                    filename: name.to_string()
                },
                "{name} must be rejected"
            );
        }
        assert!(enable(&mut Vec::new(), "Cool Pack.zip").is_ok());
        assert!(enable(&mut Vec::new(), "Epic..Pack.zip").is_ok());
    }

    #[test]
    fn filename_for_uses_the_path_basename() {
        assert_eq!(
            filename_for(Path::new("/packs/VanillaTweaks.zip")).unwrap(),
            "VanillaTweaks.zip"
        );
        assert_eq!(
            filename_for(Path::new("/packs/AutumnPack")).unwrap(),
            "AutumnPack"
        );
    }

    #[test]
    fn entries_carry_value_and_filename() {
        assert_eq!(
            StackEntry::parse("file/a.zip"),
            StackEntry::File {
                value: "file/a.zip".to_string(),
                filename: "a.zip".to_string(),
            }
        );
        assert_eq!(
            StackEntry::parse("fabric-convention-tags-v2"),
            StackEntry::Foreign {
                value: "fabric-convention-tags-v2".to_string(),
            }
        );
    }

    fn fixture_stack() -> (tempfile::TempDir, StackPaths) {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        std::fs::create_dir_all(game_dir.join("resourcepacks")).unwrap();
        let paths = StackPaths::for_game_dir(
            &game_dir,
            &dir.path().join("data"),
            BackupsConfig::default(),
        );
        (dir, paths)
    }

    fn write_fixture(paths: &StackPaths, text: &str) {
        std::fs::write(&paths.options_path, text).unwrap();
    }

    fn snapshot(resource: &[&str], incompatible: &[&str]) -> StackSnapshot {
        StackSnapshot {
            resource: arrays(resource),
            incompatible: arrays(incompatible),
        }
    }

    #[test]
    fn toggle_rebases_an_edit_during_guard() {
        struct EditingGuard(std::cell::Cell<usize>, std::path::PathBuf);
        impl WriteGuard for EditingGuard {
            fn ensure_writable(&self, _: &Path) -> Result<(), AppError> {
                let calls = self.0.get() + 1;
                self.0.set(calls);
                if calls == 2 {
                    use std::io::Write;
                    writeln!(
                        std::fs::OpenOptions::new()
                            .append(true)
                            .open(&self.1)
                            .unwrap(),
                        "lang:de_de"
                    )
                    .unwrap();
                }
                Ok(())
            }
        }
        let (_dir, paths) = fixture_stack();
        write_fixture(&paths, "resourcePacks:[\"vanilla\"]\n");
        let guard = EditingGuard(std::cell::Cell::new(0), paths.options_path.clone());
        set_enabled_with_guard(&paths, "a.zip", true, &guard).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.options_path).unwrap(),
            "resourcePacks:[\"vanilla\",\"file/a.zip\"]\nlang:de_de\n"
        );
    }

    #[test]
    fn set_enabled_writes_and_backs_up() {
        let (_dir, paths) = fixture_stack();
        write_fixture(&paths, "version:4556\nresourcePacks:[\"vanilla\"]\n");

        let stack = set_enabled(&paths, "a.zip", true).unwrap();

        assert_eq!(
            stack.entries.last().unwrap().value(),
            "file/a.zip",
            "the new pack lands at the top of the load order"
        );
        let after = std::fs::read_to_string(&paths.options_path).unwrap();
        assert!(after.contains("resourcePacks:[\"vanilla\",\"file/a.zip\"]"));
        assert!(after.contains("version:4556"));
        assert_eq!(
            backup::list(&paths.backups_root).unwrap().len(),
            1,
            "every write leaves a backup"
        );
    }

    #[test]
    fn delete_cleanup_leaves_both_arrays() {
        let (_dir, paths) = fixture_stack();
        write_fixture(
            &paths,
            "resourcePacks:[\"vanilla\",\"file/a.zip\"]\nincompatibleResourcePacks:[\"file/a.zip\"]\n",
        );

        assert!(remove_references(&paths, "a.zip").unwrap());

        let after = std::fs::read_to_string(&paths.options_path).unwrap();
        assert!(after.contains("resourcePacks:[\"vanilla\"]"));
        assert!(after.contains("incompatibleResourcePacks:[]"));
    }

    #[test]
    fn delete_cleanup_without_a_reference_writes_nothing() {
        let (_dir, paths) = fixture_stack();
        write_fixture(&paths, "resourcePacks:[\"vanilla\"]\n");

        assert!(!remove_references(&paths, "a.zip").unwrap());
        assert!(backup::list(&paths.backups_root).unwrap().is_empty());
    }

    #[test]
    fn delete_cleanup_without_options_txt_is_a_no_op() {
        let (_dir, paths) = fixture_stack();

        assert!(!remove_references(&paths, "a.zip").unwrap());
        assert!(!paths.options_path.exists());
    }

    #[test]
    fn reorder_stack_rejects_a_stale_file_set() {
        let (_dir, paths) = fixture_stack();
        write_fixture(
            &paths,
            "resourcePacks:[\"vanilla\",\"file/a.zip\",\"file/b.zip\"]\n",
        );

        // The UI rendered this snapshot, then the file set changed on disk.
        let expected = snapshot(&["vanilla", "file/a.zip", "file/other.zip"], &[]);
        let order = arrays(&["vanilla", "file/other.zip", "file/a.zip"]);
        let err = reorder_stack(&paths, &order, &expected).unwrap_err();

        assert_eq!(err.code(), "stale_stack");
    }

    #[test]
    fn reorder_stack_applies_against_the_latest_content() {
        let (_dir, paths) = fixture_stack();
        write_fixture(
            &paths,
            "resourcePacks:[\"vanilla\",\"file/a.zip\",\"file/b.zip\"]\n",
        );

        // A snapshot from before an unrelated external edit still reorders.
        let expected = snapshot(&["vanilla", "file/a.zip", "file/b.zip"], &[]);
        std::fs::write(
            &paths.options_path,
            "lang:de_de\nresourcePacks:[\"vanilla\",\"file/a.zip\",\"file/b.zip\"]\n",
        )
        .unwrap();

        let stack = reorder_stack(
            &paths,
            &arrays(&["vanilla", "file/b.zip", "file/a.zip"]),
            &expected,
        )
        .unwrap();

        assert_eq!(
            stack.snapshot().resource,
            arrays(&["vanilla", "file/b.zip", "file/a.zip"])
        );
        let after = std::fs::read_to_string(&paths.options_path).unwrap();
        assert!(after.contains("lang:de_de"), "the external edit survives");
    }

    #[test]
    fn duplicate_entries_survive_a_reorder_but_not_a_disable() {
        let (_dir, paths) = fixture_stack();
        write_fixture(
            &paths,
            "resourcePacks:[\"vanilla\",\"file/a.zip\",\"file/a.zip\"]\n",
        );

        let expected = snapshot(&["vanilla", "file/a.zip", "file/a.zip"], &[]);
        let stack = reorder_stack(
            &paths,
            &arrays(&["vanilla", "file/a.zip", "file/a.zip"]),
            &expected,
        )
        .unwrap();
        assert_eq!(stack.snapshot().resource.len(), 3);

        let stack = set_enabled(&paths, "a.zip", false).unwrap();
        assert_eq!(stack.snapshot().resource, arrays(&["vanilla"]));
    }

    #[test]
    fn an_external_unrelated_edit_survives_a_toggle() {
        let (_dir, paths) = fixture_stack();
        write_fixture(&paths, "resourcePacks:[\"vanilla\"]\nlang:en_us\n");

        std::fs::write(
            &paths.options_path,
            "resourcePacks:[\"vanilla\"]\nlang:de_de\n",
        )
        .unwrap();
        set_enabled(&paths, "a.zip", true).unwrap();

        let after = std::fs::read_to_string(&paths.options_path).unwrap();
        assert!(
            after.contains("lang:de_de"),
            "the external edit survives: {after}"
        );
    }

    struct OrderedTrash {
        trash_dir: PathBuf,
        options_path: PathBuf,
        observed: std::sync::Mutex<Vec<String>>,
    }

    impl OrderedTrash {
        fn new(root: &Path, options_path: &Path) -> Self {
            let trash_dir = root.join("trash");
            std::fs::create_dir_all(&trash_dir).unwrap();
            OrderedTrash {
                trash_dir,
                options_path: options_path.to_path_buf(),
                observed: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn observed(&self) -> Vec<String> {
            self.observed.lock().unwrap().clone()
        }
    }

    impl TrashService for OrderedTrash {
        fn move_to_trash(&self, path: &Path) -> Result<(), crate::pack::PackError> {
            // Capture options.txt at the exact moment the trash is called.
            let content = std::fs::read_to_string(&self.options_path).unwrap_or_default();
            self.observed.lock().unwrap().push(content);
            let target = self.trash_dir.join(path.file_name().unwrap());
            std::fs::rename(path, target).map_err(|err| crate::pack::PackError::Trash {
                path: path.to_path_buf(),
                reason: err.to_string(),
            })
        }
    }

    #[test]
    fn delete_removes_both_references_before_trashing() {
        let (dir, paths) = fixture_stack();
        write_fixture(
            &paths,
            "resourcePacks:[\"vanilla\",\"file/Pack.zip\"]\nincompatibleResourcePacks:[\"file/Pack.zip\"]\n",
        );
        let pack = paths.game_dir.join("resourcepacks/Pack.zip");
        std::fs::write(&pack, b"zip").unwrap();
        let trash = OrderedTrash::new(dir.path(), &paths.options_path);

        let outcome = delete_pack_safely(&paths, &pack, &trash).unwrap();

        assert!(outcome.removed_references);
        assert_eq!(
            std::fs::read_to_string(&paths.options_path).unwrap(),
            "resourcePacks:[\"vanilla\"]\nincompatibleResourcePacks:[]\n"
        );
        let observed = trash.observed();
        assert_eq!(observed.len(), 1);
        assert!(
            observed[0].contains("resourcePacks:[\"vanilla\"]"),
            "references are gone before the trash call: {}",
            observed[0]
        );
        assert!(
            observed[0].contains("incompatibleResourcePacks:[]"),
            "both arrays are cleaned before the trash call: {}",
            observed[0]
        );
        assert!(!pack.exists(), "the pack moved to the trash");
        assert_eq!(backup::list(&paths.backups_root).unwrap().len(), 1);
    }

    #[test]
    fn delete_of_an_out_of_root_path_changes_nothing() {
        let (dir, paths) = fixture_stack();
        let original = "resourcePacks:[\"vanilla\",\"file/Pack.zip\"]\n";
        write_fixture(&paths, original);
        let outside = dir.path().join("outside.zip");
        std::fs::write(&outside, b"zip").unwrap();
        let trash = OrderedTrash::new(dir.path(), &paths.options_path);

        let err = delete_pack_safely(&paths, &outside, &trash).unwrap_err();

        assert_eq!(err.code(), "pack_outside_root");
        assert_eq!(
            std::fs::read_to_string(&paths.options_path).unwrap(),
            original
        );
        assert!(backup::list(&paths.backups_root).unwrap().is_empty());
        assert!(trash.observed().is_empty());
    }

    #[test]
    fn delete_of_a_nested_path_changes_nothing() {
        let (dir, paths) = fixture_stack();
        let original = "resourcePacks:[\"vanilla\",\"file/Pack.zip\"]\n";
        write_fixture(&paths, original);
        let nested_dir = paths.game_dir.join("resourcepacks/sub");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let nested = nested_dir.join("Pack.zip");
        std::fs::write(&nested, b"zip").unwrap();
        let trash = OrderedTrash::new(dir.path(), &paths.options_path);

        let err = delete_pack_safely(&paths, &nested, &trash).unwrap_err();

        assert_eq!(err.code(), "pack_outside_root");
        assert_eq!(
            std::fs::read_to_string(&paths.options_path).unwrap(),
            original
        );
        assert!(backup::list(&paths.backups_root).unwrap().is_empty());
        assert!(trash.observed().is_empty());
    }

    #[test]
    fn delete_of_a_missing_path_changes_nothing() {
        let (dir, paths) = fixture_stack();
        let original = "resourcePacks:[\"vanilla\",\"file/Pack.zip\"]\n";
        write_fixture(&paths, original);
        let missing = paths.game_dir.join("resourcepacks/Pack.zip");
        let trash = OrderedTrash::new(dir.path(), &paths.options_path);

        let err = delete_pack_safely(&paths, &missing, &trash).unwrap_err();

        assert_eq!(err.code(), "pack_not_found");
        assert_eq!(
            std::fs::read_to_string(&paths.options_path).unwrap(),
            original
        );
        assert!(backup::list(&paths.backups_root).unwrap().is_empty());
        assert!(trash.observed().is_empty());
    }

    #[test]
    fn delete_of_a_symlinked_pack_changes_nothing() {
        let (dir, paths) = fixture_stack();
        let original = "resourcePacks:[\"vanilla\",\"file/Link.zip\"]\n";
        write_fixture(&paths, original);
        let target = paths.game_dir.join("resourcepacks/Real.zip");
        std::fs::write(&target, b"zip").unwrap();
        let link = paths.game_dir.join("resourcepacks/Link.zip");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let trash = OrderedTrash::new(dir.path(), &paths.options_path);

        let err = delete_pack_safely(&paths, &link, &trash).unwrap_err();

        assert_eq!(err.code(), "pack_symlink");
        assert_eq!(
            std::fs::read_to_string(&paths.options_path).unwrap(),
            original
        );
        assert!(backup::list(&paths.backups_root).unwrap().is_empty());
        assert!(trash.observed().is_empty());
    }

    #[test]
    fn filenames_with_inner_dots_are_valid() {
        let mut resource = Vec::new();
        assert!(enable(&mut resource, "my..pack.zip").unwrap());
        assert_eq!(resource, arrays(&["file/my..pack.zip"]));
        assert!(file_value("Epic..Pack.zip").is_ok());
    }

    #[test]
    fn a_noop_reorder_writes_no_backup() {
        let (_dir, paths) = fixture_stack();
        let text = "resourcePacks:[\"vanilla\",\"file/a.zip\",\"file/b.zip\"]\n";
        write_fixture(&paths, text);
        let order = arrays(&["vanilla", "file/a.zip", "file/b.zip"]);
        let expected = snapshot(&["vanilla", "file/a.zip", "file/b.zip"], &[]);

        let stack = reorder_stack(&paths, &order, &expected).unwrap();

        assert_eq!(stack.snapshot().resource, order);
        assert_eq!(std::fs::read_to_string(&paths.options_path).unwrap(), text);
        assert!(
            backup::list(&paths.backups_root).unwrap().is_empty(),
            "a reorder that changes nothing must not create a backup"
        );
    }

    #[test]
    fn stale_locks_never_block_a_write() {
        let (dir, paths) = fixture_stack();
        write_fixture(&paths, "resourcePacks:[\"vanilla\"]\n");
        let world = paths.game_dir.join("saves").join("world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("session.lock"), b"\x00").unwrap();

        let stack = load(&paths).unwrap();
        assert!(stack.running.is_none(), "an unlocked session.lock is stale");
        assert_eq!(stack.stale_locks, vec![world.join("session.lock")]);
        assert!(set_enabled(&paths, "a.zip", true).is_ok());
        drop(dir);
    }
}
