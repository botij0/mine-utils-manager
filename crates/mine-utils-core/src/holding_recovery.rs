use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{game_lock, AppError, DataPaths, PackError, StackPaths};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct HoldingEntry {
    pub holding_name: String,
    pub original_name: String,
}

enum HoldingFormat {
    Directory,
    Legacy(String),
}

fn parse(name: &str) -> Option<(HoldingFormat, u32)> {
    let body = name.strip_suffix(".tmp")?;
    let (format, suffix) = if let Some(suffix) = body.strip_prefix(".mine-utils-replace.") {
        (HoldingFormat::Directory, suffix)
    } else {
        let (original, suffix) = body
            .strip_prefix(".mine-utils-replace-")?
            .rsplit_once('.')?;
        crate::resource_stack::validate_filename(original).ok()?;
        (HoldingFormat::Legacy(original.to_owned()), suffix)
    };
    let mut parts = suffix.split('-');
    let pid = decimal(parts.next()?)?.parse::<u32>().ok()?;
    decimal(parts.next()?)?.parse::<u128>().ok()?;
    decimal(parts.next()?)?.parse::<u64>().ok()?;
    if pid == 0 || parts.next().is_some() {
        return None;
    }
    Some((format, pid))
}

fn resolve(
    folder: &Path,
    name: &str,
    format: HoldingFormat,
) -> Result<(HoldingEntry, PathBuf), AppError> {
    let invalid = || PackError::InvalidHolding {
        name: name.to_owned(),
    };
    let holding = folder.join(name);
    let meta =
        std::fs::symlink_metadata(&holding).map_err(|err| restore_error(err, &holding, folder))?;
    if meta.is_symlink() {
        return Err(PackError::Symlink { path: holding }.into());
    }
    let (original_name, source) = match format {
        HoldingFormat::Legacy(original) if meta.is_file() || meta.is_dir() => (original, holding),
        HoldingFormat::Directory if meta.is_dir() => {
            let mut children = std::fs::read_dir(&holding)?;
            let child = children.next().transpose()?.ok_or_else(invalid)?;
            if children.next().is_some() {
                return Err(invalid().into());
            }
            let original = child.file_name().into_string().map_err(|_| invalid())?;
            crate::resource_stack::validate_filename(&original)?;
            if crate::pack_scan::is_temp_name(&child.path()) {
                return Err(invalid().into());
            }
            let kind = child.file_type()?;
            if kind.is_symlink() {
                return Err(PackError::Symlink { path: child.path() }.into());
            }
            if !kind.is_file() && !kind.is_dir() {
                return Err(invalid().into());
            }
            (original, child.path())
        }
        _ => return Err(invalid().into()),
    };
    Ok((
        HoldingEntry {
            holding_name: name.to_owned(),
            original_name,
        },
        source,
    ))
}

fn decimal(value: &str) -> Option<&str> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
}

pub(crate) fn reject_symlink_folder(folder: &Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(folder) {
        Ok(meta) if meta.file_type().is_symlink() => Err(PackError::Symlink {
            path: folder.to_owned(),
        }
        .into()),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn list(folder: &Path) -> Result<Vec<HoldingEntry>, AppError> {
    if std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Ok(Vec::new());
    }
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut holdings = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((format, pid)) = parse(name) else {
            continue;
        };
        if pid != std::process::id() {
            if let Ok((holding, _)) = resolve(folder, name, format) {
                holdings.push(holding);
            }
        }
    }
    holdings.sort_by(|a, b| a.holding_name.cmp(&b.holding_name));
    Ok(holdings)
}

pub fn restore_resource_holding(paths: &StackPaths, holding_name: &str) -> Result<(), AppError> {
    restore(
        paths.game_dir(),
        &paths.game_dir().join("resourcepacks"),
        None,
        holding_name,
    )
}

pub fn restore_datapack_holding(paths: &DataPaths, holding_name: &str) -> Result<(), AppError> {
    restore(
        paths.game_dir(),
        paths.datapacks_dir(),
        Some(paths.world_dir()),
        holding_name,
    )
}

fn restore(
    game_dir: &Path,
    folder: &Path,
    world: Option<&Path>,
    name: &str,
) -> Result<(), AppError> {
    let invalid = || PackError::InvalidHolding {
        name: name.to_owned(),
    };
    let (format, pid) = parse(name).ok_or_else(invalid)?;
    if pid == std::process::id() {
        return Err(invalid().into());
    }
    reject_symlink_folder(folder)?;
    if let Some(world) = world {
        reject_symlink_folder(world)?;
    }
    let directory_holding = matches!(format, HoldingFormat::Directory);
    let (holding, source) = resolve(folder, name, format)?;
    let destination = folder.join(&holding.original_name);
    game_lock::ensure_writable(game_dir)?;
    if let Some(running) = world.and_then(game_lock::world_locked) {
        return Err(AppError::GameRunning { running });
    }
    crate::fs_commit::strict_no_replace(&source, &destination)
        .map_err(|err| restore_error(err, &source, &destination))?;
    if directory_holding {
        let _ = std::fs::remove_dir(folder.join(name));
    }
    Ok(())
}

fn restore_error(err: std::io::Error, source: &Path, destination: &Path) -> AppError {
    if err.kind() == std::io::ErrorKind::NotFound {
        return AppError::from(PackError::NotFound {
            path: source.to_owned(),
        })
        .with_context(
            "The leftover Replace entry is gone. Refresh the listing before trying again"
                .to_owned(),
        );
    }
    let message = match err.kind() {
        std::io::ErrorKind::AlreadyExists => format!(
            "Could not restore {}. The original name must be free; the holding entry remains at {}",
            destination.display(),
            source.display()
        ),
        std::io::ErrorKind::Unsupported => format!(
            "To restore manually, rename {} by hand to {} without replacing an existing entry",
            source.display(),
            destination.display()
        ),
        _ => format!(
            "Could not restore the holding entry at {}",
            source.display()
        ),
    };
    AppError::from(err).with_context(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_reports_manual_rename_when_atomic_rename_is_unsupported() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("resourcepacks");
        std::fs::create_dir(&folder).unwrap();
        for name in [
            ".mine-utils-replace-Pack.zip.4294967295-123-0.tmp",
            ".mine-utils-replace.4294967295-123-0.tmp",
        ] {
            let source = if name.starts_with(".mine-utils-replace.") {
                std::fs::create_dir(folder.join(name)).unwrap();
                folder.join(name).join("Pack.zip")
            } else {
                folder.join(name)
            };
            let destination = folder.join("Pack.zip");
            std::fs::write(&source, b"original").unwrap();
            let paths = StackPaths::for_game_dir(
                root.path(),
                &root.path().join("data"),
                Default::default(),
            );
            crate::fs_commit::FORCE_UNSUPPORTED.with(|flag| flag.set(true));
            let err = restore_resource_holding(&paths, name).unwrap_err();
            crate::fs_commit::FORCE_UNSUPPORTED.with(|flag| flag.set(false));
            assert_eq!(err.to_string(), format!("io error: atomic no-replace rename is unsupported on this filesystem. To restore manually, rename {} by hand to {} without replacing an existing entry", source.display(), destination.display()));
            assert_eq!(std::fs::read(source).unwrap(), b"original");
            assert!(!destination.exists());
        }
    }
}
