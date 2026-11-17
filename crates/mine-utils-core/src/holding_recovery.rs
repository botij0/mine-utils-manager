use std::path::Path;

use serde::Serialize;

use crate::{game_lock, AppError, DataPaths, PackError, StackPaths};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct HoldingEntry {
    pub holding_name: String,
    pub original_name: String,
}

fn parse(name: &str) -> Option<(HoldingEntry, u32)> {
    let body = name
        .strip_prefix(".mine-utils-replace-")?
        .strip_suffix(".tmp")?;
    let (original, suffix) = body.rsplit_once('.')?;
    crate::resource_stack::validate_filename(original).ok()?;
    let mut parts = suffix.split('-');
    let pid = decimal(parts.next()?)?.parse::<u32>().ok()?;
    decimal(parts.next()?)?.parse::<u128>().ok()?;
    decimal(parts.next()?)?.parse::<u64>().ok()?;
    if pid == 0 || parts.next().is_some() {
        return None;
    }
    Some((
        HoldingEntry {
            holding_name: name.to_owned(),
            original_name: original.to_owned(),
        },
        pid,
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
        let Some((holding, pid)) = entry.file_name().to_str().and_then(parse) else {
            continue;
        };
        let kind = entry.file_type()?;
        if pid != std::process::id() && (kind.is_file() || kind.is_dir()) {
            holdings.push(holding);
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
    let invalid = || PackError::BrokenArchive {
        name: name.to_owned(),
        reason: "not an inactive Replace holding entry in this folder".to_owned(),
    };
    let (holding, pid) = parse(name).ok_or_else(invalid)?;
    if pid == std::process::id() {
        return Err(invalid().into());
    }
    reject_symlink_folder(folder)?;
    if let Some(world) = world {
        reject_symlink_folder(world)?;
    }
    let source = folder.join(name);
    let meta = std::fs::symlink_metadata(&source)?;
    if meta.file_type().is_symlink() {
        return Err(PackError::Symlink { path: source }.into());
    }
    if !meta.is_file() && !meta.is_dir() {
        return Err(invalid().into());
    }
    game_lock::ensure_writable(game_dir)?;
    if let Some(running) = world.and_then(game_lock::world_locked) {
        return Err(AppError::GameRunning { running });
    }
    let destination = folder.join(&holding.original_name);
    crate::fs_commit::strict_no_replace(&source, &destination).map_err(|err| {
        AppError::from(err).with_context(format!(
            "Could not restore {}. The original name must be free; the holding entry remains at {}",
            holding.original_name,
            source.display()
        ))
    })
}
