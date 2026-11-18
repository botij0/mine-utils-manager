use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::error::AppError;
use crate::fs_commit::atomic_no_replace;
use crate::icon_cache::IconCache;
use crate::pack::{ImportChoice, ImportOutcome, PackError, PackKind, PackLocation};
use crate::pack_scan::{
    detect_kind, inspect_entry, is_temp_name, read_meta, reject_ambiguous_name,
    validate_delete_target,
};
use crate::pack_source::{display_name, Fingerprint, PackSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingPack {
    None,
    Destination,
    Elsewhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportPlan {
    Copy,
    Duplicate,
    Skip,
    ReplaceDuplicate,
    ReplaceDestination,
    Collision,
}

pub fn plan_import(
    existing: ExistingPack,
    destination_taken: bool,
    choice: ImportChoice,
) -> ImportPlan {
    match (existing, destination_taken, choice) {
        (ExistingPack::None, false, _) => ImportPlan::Copy,
        (ExistingPack::None, true, ImportChoice::Replace) => ImportPlan::ReplaceDestination,
        (ExistingPack::None, true, _) => ImportPlan::Collision,
        (_, _, ImportChoice::Default) => ImportPlan::Duplicate,
        (_, _, ImportChoice::Skip) => ImportPlan::Skip,
        (ExistingPack::Elsewhere, true, ImportChoice::Replace) => ImportPlan::Collision,
        (_, _, ImportChoice::Replace) => ImportPlan::ReplaceDuplicate,
    }
}

/// The one destructive step in an import or a delete. The app uses the system
/// trash; tests inject a recorder so the call itself can be asserted.
pub trait TrashService {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError>;
}

/// Moves packs to the freedesktop trash. Nothing is ever permanently deleted.
pub struct SystemTrash;

impl TrashService for SystemTrash {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
        trash::delete(path).map_err(|err| PackError::Trash {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })
    }
}

/// Memoizes pack hashes using file metadata.
#[derive(Default)]
pub struct HashCache {
    pub(crate) entries: Mutex<HashMap<PathBuf, HashEntry>>,
}

pub(crate) struct HashEntry {
    fingerprint: Fingerprint,
    sha256: String,
}

impl HashCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hash(&self, path: &Path, location: PackLocation) -> Result<String, AppError> {
        let source = PackSource::new(path, location)?;
        let fingerprint = source.fingerprint()?;
        let cached = {
            let entries = self.entries.lock().unwrap_or_else(|err| err.into_inner());
            entries
                .get(path)
                .and_then(|entry| (entry.fingerprint == fingerprint).then(|| entry.sha256.clone()))
        };
        if let Some(sha256) = cached {
            return Ok(sha256);
        }
        let sha256 = source.digest()?;
        self.entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                path.to_path_buf(),
                HashEntry {
                    fingerprint,
                    sha256: sha256.clone(),
                },
            );
        Ok(sha256)
    }
}

/// Import a zip or directory pack. The caller resolves `default`, `skip`, and
/// `replace` choices from the UI prompt. Nothing is copied until the kind and
/// the duplicate check pass, and nothing old is removed until the new copy is
/// validated in a temp path.
pub fn import_pack(
    source: &Path,
    resourcepacks: &Path,
    icons: &IconCache,
    choice: ImportChoice,
    hashes: &HashCache,
) -> Result<ImportOutcome, AppError> {
    import_pack_with(source, resourcepacks, icons, choice, hashes, &SystemTrash)
}

/// `import_pack` with an injectable trash, so tests can prove that Replace
/// moves exactly one pack to the trash and rolls back before it touches any.
pub fn import_pack_with(
    source: &Path,
    resourcepacks: &Path,
    icons: &IconCache,
    choice: ImportChoice,
    hashes: &HashCache,
    trash: &dyn TrashService,
) -> Result<ImportOutcome, AppError> {
    import_pack_with_services(
        source,
        resourcepacks,
        icons,
        choice,
        hashes,
        &ImportServices { trash, hooks: &() },
        atomic_no_replace,
    )
}

pub(crate) trait ImportHooks {
    fn before_write(&self) -> Result<(), AppError>;
    fn replaced(&self, _old: &Path, _new: &Path) -> Result<(), AppError> {
        Ok(())
    }
}

impl ImportHooks for () {
    fn before_write(&self) -> Result<(), AppError> {
        Ok(())
    }
}

pub(crate) struct ImportServices<'a> {
    pub trash: &'a dyn TrashService,
    pub hooks: &'a dyn ImportHooks,
}

pub(crate) fn import_pack_guarded(
    source: &Path,
    resourcepacks: &Path,
    icons: &IconCache,
    choice: ImportChoice,
    hashes: &HashCache,
    services: &ImportServices<'_>,
) -> Result<ImportOutcome, AppError> {
    import_pack_with_services(
        source,
        resourcepacks,
        icons,
        choice,
        hashes,
        services,
        atomic_no_replace,
    )
}

#[cfg(test)]
pub(crate) fn import_pack_with_commit(
    source: &Path,
    resourcepacks: &Path,
    icons: &IconCache,
    choice: ImportChoice,
    hashes: &HashCache,
    trash: &dyn TrashService,
    commit: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<ImportOutcome, AppError> {
    import_pack_with_services(
        source,
        resourcepacks,
        icons,
        choice,
        hashes,
        &ImportServices { trash, hooks: &() },
        commit,
    )
}

fn import_pack_with_services(
    source: &Path,
    resourcepacks: &Path,
    icons: &IconCache,
    choice: ImportChoice,
    hashes: &HashCache,
    services: &ImportServices<'_>,
    commit: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<ImportOutcome, AppError> {
    services.hooks.before_write()?;
    let source_meta = std::fs::symlink_metadata(source).map_err(|_| PackError::NotFound {
        path: source.to_path_buf(),
    })?;
    if source_meta.file_type().is_symlink() {
        return Err(PackError::Symlink {
            path: source.to_path_buf(),
        }
        .into());
    }
    let source = std::fs::canonicalize(source).map_err(|_| PackError::NotFound {
        path: source.to_path_buf(),
    })?;
    if is_temp_name(&source) {
        return Err(PackError::BrokenArchive {
            name: display_name(&source),
            reason:
                "this name is reserved for temporary imports; rename the pack before importing it"
                    .to_string(),
        }
        .into());
    }
    let is_dir = source.is_dir();
    if !is_dir
        && !source
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("zip"))
            .unwrap_or(false)
    {
        return Err(PackError::UnknownKind {
            name: display_name(&source),
        }
        .into());
    }

    let location = if is_dir {
        PackLocation::Directory
    } else {
        PackLocation::Zip
    };
    let name = display_name(&source);
    let kind = detect_kind(&source, location)?;
    if kind == PackKind::Unknown {
        return Err(PackError::UnknownKind { name }.into());
    }
    validate_meta(&source, location, &name)?;

    std::fs::create_dir_all(resourcepacks)?;
    let root = std::fs::canonicalize(resourcepacks)?;
    if source.parent() == Some(root.as_path()) {
        let mut bytes_read = 0;
        let pack = inspect_entry(&source, location, icons, &mut bytes_read)?;
        return Ok(ImportOutcome::Imported {
            pack: Box::new(pack),
        });
    }

    let sha256 = hashes.hash(&source, location)?;
    let existing = find_duplicate(&root, &sha256, hashes)?;

    if existing.as_deref() == Some(source.as_path()) {
        let mut bytes_read = 0;
        let pack = inspect_entry(&source, location, icons, &mut bytes_read)?;
        return Ok(ImportOutcome::Imported {
            pack: Box::new(pack),
        });
    }

    let file_name = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| PackError::BrokenArchive {
            name: name.clone(),
            reason: "source has no file name".to_string(),
        })?;
    let destination = root.join(&file_name);
    reject_ambiguous_name(&destination, &root)?;
    let destination_taken = std::fs::symlink_metadata(&destination).is_ok();

    let existing_pack = match &existing {
        None => ExistingPack::None,
        Some(path) if path == &destination => ExistingPack::Destination,
        Some(_) => ExistingPack::Elsewhere,
    };
    let removal = match plan_import(existing_pack, destination_taken, choice) {
        ImportPlan::Copy => None,
        ImportPlan::Skip => return Ok(ImportOutcome::Skipped),
        ImportPlan::Duplicate => {
            return Ok(ImportOutcome::Duplicate {
                name,
                sha256,
                existing: existing.unwrap().to_string_lossy().to_string(),
            });
        }
        ImportPlan::Collision => return Ok(ImportOutcome::Collision { name }),
        ImportPlan::ReplaceDuplicate => existing,
        ImportPlan::ReplaceDestination => Some(destination.clone()),
    };

    let tmp = unique_temp_path(&root, &file_name);
    let copy_result = if is_dir {
        copy_dir(&source, &tmp)
    } else {
        copy_file(&source, &tmp)
    };
    if let Err(err) = copy_result {
        let _ = remove_any(&tmp);
        return Err(err.into());
    }
    if let Err(err) = validate_meta(&tmp, location, &name) {
        let _ = remove_any(&tmp);
        return Err(err.into());
    }

    let mut held = if let Some(target) = &removal {
        let holding = unique_temp_path(
            &root,
            &format!(
                "mine-utils-replace-{}",
                target.file_name().unwrap().to_string_lossy()
            ),
        );
        let result = (|| {
            validate_delete_target(target, &root)?;
            services.hooks.before_write()?;
            atomic_no_replace(target, &holding)?;
            Ok::<_, AppError>(())
        })();
        if let Err(err) = result {
            let _ = remove_any(&tmp);
            return Err(err);
        }
        Some((target, holding))
    } else {
        None
    };

    let mut installed = false;
    let result = (|| {
        services.hooks.before_write()?;
        commit(&tmp, &destination)?;
        installed = true;
        let mut bytes_read = 0;
        let pack = inspect_entry(&destination, location, icons, &mut bytes_read)?;
        if let Some((old, holding)) = &mut held {
            services.hooks.replaced(old, &destination)?;
            services.hooks.before_write()?;
            if *old != &destination && atomic_no_replace(holding, old).is_ok() {
                holding.clone_from(old);
            }
            services.trash.move_to_trash(holding)?;
        }
        Ok::<_, AppError>(pack)
    })();
    let pack = match result {
        Ok(pack) => pack,
        Err(err) => {
            let _ = remove_any(&tmp);
            let mut context = Vec::new();
            if installed {
                context.push(format!("New pack installed at {}", destination.display()));
            }
            let mut recovery_failed = false;
            if let Some((old, holding)) = &held {
                let restore = if holding == *old {
                    Ok(())
                } else {
                    atomic_no_replace(holding, old)
                };
                match restore {
                    Ok(()) => context.push(format!("Original pack retained at {}", old.display())),
                    Err(restore) => {
                        recovery_failed = true;
                        context.push(format!(
                            "Original pack retained at {}. Could not restore {}: {restore}. Restart Mine Utils to restore the holding entry",
                            holding.display(),
                            old.display()
                        ));
                    }
                }
            }
            if !installed
                && !recovery_failed
                && matches!(&err, AppError::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists)
            {
                return Ok(ImportOutcome::Collision { name });
            }
            return Err(if context.is_empty() {
                err
            } else {
                err.with_context(context.join(". "))
            });
        }
    };
    sync_dir(&root);
    Ok(ImportOutcome::Imported {
        pack: Box::new(pack),
    })
}

fn validate_meta(path: &Path, location: PackLocation, name: &str) -> Result<(), PackError> {
    let source = PackSource::new(path, location)?;
    read_meta(&source, name, &mut 0).map(|_| ())
}

/// I/O errors propagate: silently skipping an unreadable pack would hide a
/// real duplicate.
pub(crate) fn find_duplicate(
    resourcepacks: &Path,
    sha256: &str,
    hashes: &HashCache,
) -> Result<Option<PathBuf>, AppError> {
    let entries = match std::fs::read_dir(resourcepacks) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if is_temp_name(&path) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let location = if file_type.is_dir() {
            PackLocation::Directory
        } else if path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("zip"))
            .unwrap_or(false)
        {
            PackLocation::Zip
        } else {
            continue;
        };
        match hashes.hash(&path, location) {
            Ok(hash) if hash == sha256 => return Ok(Some(path)),
            Ok(_) | Err(AppError::Pack(PackError::Symlink { .. })) => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(None)
}

fn copy_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut input = File::open(source)?;
    let mut output = File::create(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn copy_dir(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            copy_file(&entry.path(), &target)?;
        }
    }
    let handle = File::open(destination)?;
    let _ = handle.sync_all();
    Ok(())
}

/// Permanent removal, only for temp paths this module created. Packs always
/// go through `TrashService`.
fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn unique_temp_path(root: &Path, file_name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    root.join(format!(
        ".{file_name}.{}-{stamp}-{counter}.tmp",
        std::process::id()
    ))
}

fn sync_dir(path: &Path) {
    if let Ok(handle) = File::open(path) {
        let _ = handle.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_import_covers_every_existing_taken_choice_combination() {
        use ExistingPack::{Destination, Elsewhere, None};
        use ImportChoice::{Default, Replace, Skip};
        let cases = [
            (None, false, Default, ImportPlan::Copy),
            (None, false, Skip, ImportPlan::Copy),
            (None, false, Replace, ImportPlan::Copy),
            (None, true, Default, ImportPlan::Collision),
            (None, true, Skip, ImportPlan::Collision),
            (None, true, Replace, ImportPlan::ReplaceDestination),
            (Destination, false, Default, ImportPlan::Duplicate),
            (Destination, false, Skip, ImportPlan::Skip),
            (Destination, false, Replace, ImportPlan::ReplaceDuplicate),
            (Destination, true, Default, ImportPlan::Duplicate),
            (Destination, true, Skip, ImportPlan::Skip),
            (Destination, true, Replace, ImportPlan::ReplaceDuplicate),
            (Elsewhere, false, Default, ImportPlan::Duplicate),
            (Elsewhere, false, Skip, ImportPlan::Skip),
            (Elsewhere, false, Replace, ImportPlan::ReplaceDuplicate),
            (Elsewhere, true, Default, ImportPlan::Duplicate),
            (Elsewhere, true, Skip, ImportPlan::Skip),
            (Elsewhere, true, Replace, ImportPlan::Collision),
        ];
        for (existing, taken, choice, expected) in cases {
            assert_eq!(
                plan_import(existing, taken, choice),
                expected,
                "existing={existing:?}, taken={taken}, choice={choice:?}"
            );
        }
    }
}
