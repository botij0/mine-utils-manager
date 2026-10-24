use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::format_table::FormatTable;
use crate::icon_cache::IconCache;
use crate::pack::{
    ImportChoice, ImportOutcome, Pack, PackError, PackIssue, PackKind, PackLocation,
};
use crate::pack_meta::parse_pack_meta;

/// A pack.png larger than this is ignored rather than cached.
pub const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// pack.mcmeta larger than this is treated as broken.
pub const MAX_META_BYTES: u64 = 1024 * 1024;
/// Rough size of one zip central directory record plus its 22 byte end record.
const CENTRAL_DIR_ENTRY_BYTES: u64 = 46;
const CENTRAL_DIR_END_BYTES: u64 = 22;

#[derive(Debug, Clone, Serialize)]
pub struct PackListing {
    pub packs: Vec<Pack>,
    pub count: usize,
    pub bytes_read: u64,
    pub elapsed_ms: f64,
}

/// List only direct children of resourcepacks. A zip or a directory with
/// pack.mcmeta is a pack. Broken entries become error cards so one bad pack
/// never hides the rest.
pub fn scan_packs(resourcepacks: &Path, icons: &IconCache) -> Result<PackListing, AppError> {
    let started = Instant::now();
    let mut packs = Vec::new();
    let mut bytes_read = 0u64;

    let entries = match std::fs::read_dir(resourcepacks) {
        Ok(entries) => entries.filter_map(|entry| entry.ok()).map(|entry| entry.path()).collect(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err.into()),
    };
    let mut children: Vec<PathBuf> = entries;
    children.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

    for path in children {
        let is_dir = path.is_dir();
        let is_zip = path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        if !is_dir && !is_zip {
            continue;
        }
        if is_dir && !path.join("pack.mcmeta").is_file() {
            continue;
        }
        let location = if is_dir {
            PackLocation::Directory
        } else {
            PackLocation::Zip
        };
        let read_before = bytes_read;
        let result = inspect_entry(&path, location, icons, &mut bytes_read);
        let pack = match result {
            Ok(pack) => pack,
            Err(err) => error_pack(&path, location, err, bytes_read - read_before),
        };
        packs.push(pack);
    }

    Ok(PackListing {
        count: packs.len(),
        packs,
        bytes_read,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

fn sort_key(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn inspect_entry(
    path: &Path,
    location: PackLocation,
    icons: &IconCache,
    bytes_read: &mut u64,
) -> Result<Pack, PackError> {
    match location {
        PackLocation::Zip => inspect_zip(path, icons, bytes_read),
        PackLocation::Directory => inspect_dir(path, icons, bytes_read),
    }
}

fn inspect_zip(path: &Path, icons: &IconCache, bytes_read: &mut u64) -> Result<Pack, PackError> {
    let name = display_name(path);
    let file = File::open(path).map_err(|err| broken(&name, err))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| broken(&name, err))?;

    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    *bytes_read += names
        .iter()
        .map(|entry| CENTRAL_DIR_ENTRY_BYTES + entry.len() as u64)
        .sum::<u64>()
        + CENTRAL_DIR_END_BYTES;

    let kind = kind_from_names(names.iter().map(String::as_str));
    let meta_bytes = read_zip_entry(&mut archive, "pack.mcmeta", MAX_META_BYTES, bytes_read)
        .map_err(|err| match err {
            PackError::MissingMeta { .. } => err,
            other => other,
        })?;
    let meta = parse_pack_meta(&meta_bytes).map_err(|err| match err {
        PackError::BrokenArchive { reason, .. } => PackError::BrokenArchive { name, reason },
        other => other,
    })?;

    let icon = read_zip_entry_optional(&mut archive, "pack.png", MAX_ICON_BYTES, bytes_read)
        .and_then(|bytes| icons.store(&bytes).ok());

    Ok(finish_pack(path, PackLocation::Zip, kind, meta, icon, None, *bytes_read))
}

fn inspect_dir(path: &Path, icons: &IconCache, bytes_read: &mut u64) -> Result<Pack, PackError> {
    let name = display_name(path);
    let meta_path = path.join("pack.mcmeta");
    let size = std::fs::metadata(&meta_path)
        .map_err(|_| PackError::MissingMeta { name: name.clone() })?
        .len();
    if size > MAX_META_BYTES {
        return Err(broken(&name, "pack.mcmeta is larger than 1 MB"));
    }
    let meta_bytes = std::fs::read(&meta_path).map_err(|err| broken(&name, err))?;
    *bytes_read += meta_bytes.len() as u64;

    let children: Vec<String> = std::fs::read_dir(path)
        .map_err(|err| broken(&name, err))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    let kind = kind_from_names(children.iter().map(String::as_str));
    let meta = parse_pack_meta(&meta_bytes)
        .map_err(|err| match err {
            PackError::BrokenArchive { reason, .. } => PackError::BrokenArchive { name, reason },
            other => other,
        })?;

    let icon = read_dir_icon(path, icons, bytes_read);
    Ok(finish_pack(
        path,
        PackLocation::Directory,
        kind,
        meta,
        icon,
        None,
        *bytes_read,
    ))
}

fn read_dir_icon(path: &Path, icons: &IconCache, bytes_read: &mut u64) -> Option<crate::pack::PackIcon> {
    let icon_path = path.join("pack.png");
    let metadata = std::fs::metadata(&icon_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
        return None;
    }
    let bytes = std::fs::read(&icon_path).ok()?;
    *bytes_read += bytes.len() as u64;
    icons.store(&bytes).ok()
}

fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    entry: &str,
    cap: u64,
    bytes_read: &mut u64,
) -> Result<Vec<u8>, PackError> {
    let mut file = match archive.by_name(entry) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(PackError::MissingMeta {
                name: entry.to_string(),
            })
        }
        Err(err) => {
            return Err(PackError::BrokenArchive {
                name: entry.to_string(),
                reason: err.to_string(),
            })
        }
    };
    if file.size() > cap {
        return Err(PackError::BrokenArchive {
            name: entry.to_string(),
            reason: format!("entry is larger than {} bytes", cap),
        });
    }
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|err| PackError::BrokenArchive {
            name: entry.to_string(),
            reason: err.to_string(),
        })?;
    *bytes_read += bytes.len() as u64;
    Ok(bytes)
}

fn read_zip_entry_optional<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    entry: &str,
    cap: u64,
    bytes_read: &mut u64,
) -> Option<Vec<u8>> {
    let file = archive.by_name(entry).ok()?;
    if file.size() > cap {
        return None;
    }
    drop(file);
    read_zip_entry(archive, entry, cap, bytes_read).ok()
}

fn finish_pack(
    path: &Path,
    location: PackLocation,
    kind: PackKind,
    meta: crate::pack::PackMeta,
    icon: Option<crate::pack::PackIcon>,
    error: Option<PackIssue>,
    bytes_read: u64,
) -> Pack {
    let table = FormatTable::embedded();
    let range = meta.effective_range();
    let game_label = range.and_then(|range| table.game_label(kind, range.min, range.max));
    Pack {
        id: path.to_string_lossy().to_string(),
        name: display_name(path),
        path: path.to_path_buf(),
        location,
        kind,
        description: meta.description.clone(),
        format: range.map(|range| range.min),
        format_range: range,
        game_label,
        icon,
        error,
        meta,
        bytes_read,
    }
}

fn error_pack(path: &Path, location: PackLocation, err: PackError, bytes_read: u64) -> Pack {
    let code = err.code().to_string();
    finish_pack(
        path,
        location,
        PackKind::Unknown,
        crate::pack::PackMeta::default(),
        None,
        Some(PackIssue {
            code,
            message: err.to_string(),
        }),
        bytes_read,
    )
}

fn display_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    name.strip_suffix(".zip")
        .or_else(|| name.strip_suffix(".ZIP"))
        .unwrap_or(&name)
        .to_string()
}

fn kind_from_names<'a>(names: impl Iterator<Item = &'a str>) -> PackKind {
    let mut assets = false;
    let mut data = false;
    for name in names {
        let root = name.trim_start_matches("./").split('/').next().unwrap_or("");
        match root {
            "assets" => assets = true,
            "data" => data = true,
            _ => {}
        }
    }
    match (assets, data) {
        (true, true) => PackKind::Both,
        (true, false) => PackKind::Resource,
        (false, true) => PackKind::Data,
        (false, false) => PackKind::Unknown,
    }
}

fn broken(name: &str, reason: impl std::fmt::Display) -> PackError {
    PackError::BrokenArchive {
        name: name.to_string(),
        reason: reason.to_string(),
    }
}

/// Import a zip or directory pack. The caller resolves `default`, `skip`, and
/// `replace` choices from the UI prompt. Nothing is copied until the kind and
/// the duplicate check pass.
pub fn import_pack(
    source: &Path,
    resourcepacks: &Path,
    icons: &IconCache,
    choice: ImportChoice,
) -> Result<ImportOutcome, AppError> {
    let source = std::fs::canonicalize(source).map_err(|_| PackError::NotFound {
        path: source.to_path_buf(),
    })?;
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
    if is_dir {
        reject_symlinks(&source)?;
    }
    let name = display_name(&source);
    let kind = detect_kind(&source, location)?;
    if kind == PackKind::Unknown {
        return Err(PackError::UnknownKind { name }.into());
    }

    let sha256 = hash_source(&source, location)?;
    let existing = find_duplicate(resourcepacks, &sha256)?;
    if let Some(existing_path) = &existing {
        match choice {
            ImportChoice::Skip => return Ok(ImportOutcome::Skipped),
            ImportChoice::Default => {
                return Ok(ImportOutcome::Duplicate {
                    name,
                    sha256,
                    existing: existing_path.to_string_lossy().to_string(),
                })
            }
            ImportChoice::Replace => remove_any(existing_path)?,
        }
    }

    let file_name = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| PackError::BrokenArchive {
            name: name.clone(),
            reason: "source has no file name".to_string(),
        })?;
    std::fs::create_dir_all(resourcepacks)?;
    let destination = resourcepacks.join(&file_name);
    if destination.exists() {
        match choice {
            ImportChoice::Replace => remove_any(&destination)?,
            _ => return Ok(ImportOutcome::Collision { name }),
        }
    }

    let tmp = resourcepacks.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let _ = remove_any(&tmp);
    let copy_result = if is_dir {
        copy_dir(&source, &tmp)
    } else {
        copy_file(&source, &tmp)
    };
    if let Err(err) = copy_result {
        let _ = remove_any(&tmp);
        return Err(err.into());
    }
    std::fs::rename(&tmp, &destination)?;
    sync_dir(resourcepacks);

    let mut bytes_read = 0;
    let pack = inspect_entry(&destination, location, icons, &mut bytes_read)
        .map_err(|err| match err {
            PackError::BrokenArchive { .. } => err,
            other => other,
        })?;
    Ok(ImportOutcome::Imported { pack })
}

/// Move a pack to the system trash. Only direct children of resourcepacks are
/// eligible, so a stale path cannot delete something outside the game dir.
pub fn delete_pack(path: &Path, resourcepacks: &Path) -> Result<(), AppError> {
    let root = std::fs::canonicalize(resourcepacks)?;
    let target = std::fs::canonicalize(path).map_err(|_| PackError::NotFound {
        path: path.to_path_buf(),
    })?;
    if target.parent() != Some(root.as_path()) {
        return Err(PackError::OutsideRoot { path: target }.into());
    }
    if target != root {
        trash::delete(&target).map_err(|err| PackError::Trash {
            path: target.clone(),
            reason: err.to_string(),
        })?;
    }
    Ok(())
}

fn detect_kind(path: &Path, location: PackLocation) -> Result<PackKind, AppError> {
    match location {
        PackLocation::Directory => {
            let children: Vec<String> = std::fs::read_dir(path)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect();
            Ok(kind_from_names(children.iter().map(String::as_str)))
        }
        PackLocation::Zip => {
            let file = File::open(path)?;
            let archive = zip::ZipArchive::new(file).map_err(|err| {
                PackError::BrokenArchive {
                    name: display_name(path),
                    reason: err.to_string(),
                }
            })?;
            Ok(kind_from_names(archive.file_names()))
        }
    }
}

fn reject_symlinks(root: &Path) -> Result<(), AppError> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(PackError::Symlink {
                path: entry.path(),
            }
            .into());
        }
        if file_type.is_dir() {
            reject_symlinks(&entry.path())?;
        }
    }
    Ok(())
}

fn hash_source(path: &Path, location: PackLocation) -> Result<String, AppError> {
    match location {
        PackLocation::Zip => hash_file(path),
        PackLocation::Directory => hash_dir(path),
    }
}

fn hash_file(path: &Path) -> Result<String, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hash a directory tree by relative path and content. File order is sorted so
/// the same tree always produces the same digest.
fn hash_dir(root: &Path) -> Result<String, AppError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        let mut file = File::open(root.join(&relative))?;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AppError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(PackError::Symlink {
                path: entry.path(),
            }
            .into());
        }
        if file_type.is_dir() {
            collect_files(root, &entry.path(), out)?;
        } else if file_type.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|_| entry.path());
            out.push(relative);
        }
    }
    Ok(())
}

fn find_duplicate(resourcepacks: &Path, sha256: &str) -> Result<Option<PathBuf>, AppError> {
    let entries = match std::fs::read_dir(resourcepacks) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let location = if path.is_dir() {
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
        if hash_source(&path, location).ok().as_deref() == Some(sha256) {
            return Ok(Some(path));
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

fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn sync_dir(path: &Path) {
    if let Ok(handle) = File::open(path) {
        let _ = handle.sync_all();
    }
}

/// Names of the entries in a resourcepacks dir, for tests and tooling.
pub fn pack_names(resourcepacks: &Path) -> Result<BTreeSet<String>, AppError> {
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(resourcepacks)? {
        let entry = entry?;
        names.insert(entry.file_name().to_string_lossy().to_string());
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{FormatVersion, PackKind};
    use std::io::Write as _;

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn mcmeta(min: u32, max: Option<u32>, description: &str) -> Vec<u8> {
        match max {
            Some(max) => format!(
                r#"{{"pack":{{"pack_format":{min},"min_format":{min},"max_format":{max},"description":"{description}"}}}}"#
            ),
            None => format!(
                r#"{{"pack":{{"pack_format":{min},"description":"{description}"}}}}"#
            ),
        }
        .into_bytes()
    }

    fn simple_zip(path: &Path, format: u32, icon: Option<&[u8]>) {
        let meta = mcmeta(format, None, "Demo pack");
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("pack.mcmeta", meta.as_slice()),
            ("assets/minecraft/textures/a.png", b"asset"),
        ];
        if let Some(icon) = icon {
            entries.push(("pack.png", icon));
        }
        write_zip(path, &entries);
    }

    fn cache() -> (tempfile::TempDir, IconCache) {
        let root = tempfile::tempdir().unwrap();
        let cache = IconCache::new(root.path());
        (root, cache)
    }

    #[test]
    fn reads_real_shape_metadata_and_icon() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let zip_path = root.path().join("Demo.zip");
        let meta = br#"{"pack":{"pack_format":55,"supported_formats":{"min_inclusive":34,"max_inclusive":69},"description":{"text":"Hello ","extra":[{"text":"world"}]}}}"#;
        write_zip(
            &zip_path,
            &[
                ("pack.mcmeta", meta),
                ("pack.png", b"png-data"),
                ("assets/minecraft/x", b"x"),
            ],
        );

        let listing = scan_packs(root.path(), &icons).unwrap();
        let pack = &listing.packs[0];
        assert_eq!(pack.name, "Demo");
        assert_eq!(pack.kind, PackKind::Resource);
        assert_eq!(pack.description, "Hello world");
        assert_eq!(pack.format, Some(FormatVersion::from_major(34)));
        assert_eq!(pack.game_label.as_deref(), Some("1.21 to 1.21.10"));
        assert!(pack.icon.is_some());
        assert!(pack.icon.as_ref().unwrap().path.is_file());
        assert!(pack.error.is_none());
        assert!(listing.bytes_read < MAX_ICON_BYTES);
    }

    #[test]
    fn missing_icon_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        simple_zip(&root.path().join("NoIcon.zip"), 34, None);

        let listing = scan_packs(root.path(), &icons).unwrap();
        assert_eq!(listing.packs.len(), 1);
        assert!(listing.packs[0].icon.is_none());
        assert!(listing.packs[0].error.is_none());
        assert_eq!(
            listing.packs[0].game_label.as_deref(),
            Some("1.21 to 1.21.1")
        );
    }

    #[test]
    fn corrupt_archive_is_an_error_card() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        std::fs::write(root.path().join("Broken.zip"), b"not a zip").unwrap();
        simple_zip(&root.path().join("Good.zip"), 34, None);

        let listing = scan_packs(root.path(), &icons).unwrap();
        assert_eq!(listing.count, 2);
        let broken = listing.packs.iter().find(|pack| pack.name == "Broken").unwrap();
        assert_eq!(broken.error.as_ref().unwrap().code, "pack_broken_archive");
        let good = listing.packs.iter().find(|pack| pack.name == "Good").unwrap();
        assert!(good.error.is_none());
        assert!(listing.packs.iter().any(|pack| pack.name == "Good"));
    }

    #[test]
    fn oversized_icon_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let big = vec![7u8; 3 * 1024 * 1024];
        simple_zip(&root.path().join("BigIcon.zip"), 34, Some(&big));

        let listing = scan_packs(root.path(), &icons).unwrap();
        let pack = &listing.packs[0];
        assert!(pack.icon.is_none());
        assert!(pack.error.is_none());
        assert!(
            listing.bytes_read < 1024 * 1024,
            "must not read a 3 MB icon: {}",
            listing.bytes_read
        );
    }

    #[test]
    fn directory_pack_and_both_kinds() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let dir = root.path().join("DirPack");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(
            dir.join("pack.mcmeta"),
            mcmeta(15, Some(22), "Two kinds"),
        )
        .unwrap();
        std::fs::write(dir.join("pack.png"), b"icon").unwrap();

        let listing = scan_packs(root.path(), &icons).unwrap();
        let pack = &listing.packs[0];
        assert_eq!(pack.location, PackLocation::Directory);
        assert_eq!(pack.kind, PackKind::Both);
        assert!(pack.icon.is_some());
        assert_eq!(pack.description, "Two kinds");
    }

    #[test]
    fn only_direct_children_are_scanned() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let nested = root.path().join("nested");
        simple_zip(&nested.join("Inside.zip"), 34, None);

        let listing = scan_packs(root.path(), &icons).unwrap();
        assert_eq!(listing.count, 0, "must not recurse into nested dirs");
    }

    #[test]
    fn hundred_packs_are_sorted_isolated_and_capped() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        for index in 0..100 {
            let name = format!("Pack{index:03}.zip");
            simple_zip(&root.path().join(&name), 34, Some(b"tiny"));
        }
        std::fs::write(root.path().join("ZBroken.zip"), b"nope").unwrap();
        std::fs::write(root.path().join("aaa-notes.txt"), b"ignore me").unwrap();

        let listing = scan_packs(root.path(), &icons).unwrap();
        assert_eq!(listing.count, 101);
        let names: Vec<&str> = listing.packs.iter().map(|pack| pack.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(names, sorted, "listing order is stable");
        assert_eq!(
            listing
                .packs
                .iter()
                .filter(|pack| pack.error.is_some())
                .count(),
            1
        );
        assert!(listing.bytes_read / listing.count as u64 <= 1024 * 1024);
        assert!(listing.elapsed_ms < 1500.0);
    }

    #[test]
    fn imports_zip_and_directory() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let resourcepacks = root.path().join("game/resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();

        let zip_source = root.path().join("incoming/ZipPack.zip");
        simple_zip(&zip_source, 34, Some(b"icon"));
        let outcome = import_pack(&zip_source, &resourcepacks, &icons, ImportChoice::Default).unwrap();
        let pack = match outcome {
            ImportOutcome::Imported { pack } => pack,
            other => panic!("expected import, got {other:?}"),
        };
        assert_eq!(pack.name, "ZipPack");
        assert!(resourcepacks.join("ZipPack.zip").is_file());

        let dir_source = root.path().join("incoming/DirPack");
        std::fs::create_dir_all(dir_source.join("assets")).unwrap();
        std::fs::write(dir_source.join("pack.mcmeta"), mcmeta(7, None, "Dir")).unwrap();
        let outcome = import_pack(&dir_source, &resourcepacks, &icons, ImportChoice::Default).unwrap();
        assert!(matches!(outcome, ImportOutcome::Imported { .. }));
        assert!(resourcepacks.join("DirPack/pack.mcmeta").is_file());
    }

    #[test]
    fn duplicate_offers_skip_and_replace() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);

        import_pack(&source, &resourcepacks, &icons, ImportChoice::Default).unwrap();
        let duplicate =
            import_pack(&source, &resourcepacks, &icons, ImportChoice::Default).unwrap();
        assert!(matches!(duplicate, ImportOutcome::Duplicate { .. }), "{duplicate:?}");

        let skipped = import_pack(&source, &resourcepacks, &icons, ImportChoice::Skip).unwrap();
        assert_eq!(skipped, ImportOutcome::Skipped);
        assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);

        let replaced =
            import_pack(&source, &resourcepacks, &icons, ImportChoice::Replace).unwrap();
        assert!(matches!(replaced, ImportOutcome::Imported { .. }));
        assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);
    }

    #[test]
    fn name_collision_needs_replace() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let first = root.path().join("one/Pack.zip");
        simple_zip(&first, 34, None);
        import_pack(&first, &resourcepacks, &icons, ImportChoice::Default).unwrap();

        let second = root.path().join("two/Pack.zip");
        write_zip(
            &second,
            &[
                ("pack.mcmeta", mcmeta(55, None, "Different").as_slice()),
                ("assets/minecraft/x", b"x"),
            ],
        );
        let collision =
            import_pack(&second, &resourcepacks, &icons, ImportChoice::Default).unwrap();
        assert!(matches!(collision, ImportOutcome::Collision { .. }), "{collision:?}");
        assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);

        let replaced = import_pack(&second, &resourcepacks, &icons, ImportChoice::Replace).unwrap();
        assert!(matches!(replaced, ImportOutcome::Imported { .. }));
        let listing = scan_packs(&resourcepacks, &icons).unwrap();
        assert_eq!(listing.packs[0].description, "Different");
    }

    #[test]
    fn rejects_unknown_kind_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();

        let empty = root.path().join("empty.zip");
        write_zip(&empty, &[("readme.txt", b"nope")]);
        let err = import_pack(&empty, &resourcepacks, &icons, ImportChoice::Default).unwrap_err();
        assert_eq!(err.code(), "pack_unknown_kind");

        let source = root.path().join("linked");
        std::fs::create_dir_all(source.join("assets")).unwrap();
        std::fs::write(source.join("pack.mcmeta"), mcmeta(34, None, "linked")).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", source.join("assets/escape.txt")).unwrap();
        let err = import_pack(&source, &resourcepacks, &icons, ImportChoice::Default).unwrap_err();
        assert_eq!(err.code(), "pack_symlink");
    }

    #[test]
    fn delete_only_accepts_direct_children() {
        let root = tempfile::tempdir().unwrap();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(resourcepacks.join("Pack")).unwrap();
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        let err = delete_pack(&outside, &resourcepacks).unwrap_err();
        assert_eq!(err.code(), "pack_outside_root");
    }
}
