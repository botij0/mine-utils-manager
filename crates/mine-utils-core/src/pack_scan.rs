use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::format_table::FormatTable;
use crate::icon_cache::{icon_within_pixel_cap, IconCache};
use crate::pack::{
    FormatVersion, ImportChoice, ImportOutcome, Pack, PackError, PackIssue, PackKind, PackLocation,
};
use crate::pack_meta::parse_pack_meta;

/// A pack.png larger than this is ignored rather than cached.
pub const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// pack.mcmeta larger than this is treated as broken.
pub const MAX_META_BYTES: u64 = 1024 * 1024;
/// A central directory with more entries than this is treated as hostile.
pub const MAX_ZIP_ENTRIES: usize = 65_536;
/// Rough size of one zip central directory record plus its 22 byte end record.
const CENTRAL_DIR_ENTRY_BYTES: u64 = 46;
const CENTRAL_DIR_END_BYTES: u64 = 22;

#[derive(Debug, Clone, Serialize)]
pub struct PackListing {
    pub packs: Vec<Pack>,
    pub count: usize,
    /// Total bytes read across the listing. Each pack carries its own delta.
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

    let mut children: Vec<(PathBuf, std::fs::FileType)> = match std::fs::read_dir(resourcepacks) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let file_type = entry.file_type().ok();
                (entry.path(), file_type)
            })
            .filter_map(|(path, file_type)| file_type.map(|file_type| (path, file_type)))
            .collect(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err.into()),
    };
    children.sort_by_key(|(path, _)| sort_key(path));

    for (path, file_type) in children {
        if is_temp_name(&path) {
            continue;
        }
        let is_zip = path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        let is_dir = path.is_dir();
        if !is_dir && !is_zip {
            continue;
        }
        let location = if is_zip {
            PackLocation::Zip
        } else {
            PackLocation::Directory
        };
        if file_type.is_symlink() {
            // A symlinked pack is an error card. The link is never followed
            // into the root, so scanning cannot read or later delete a pack
            // that only a link points at.
            packs.push(error_pack(
                &path,
                location,
                PackError::Symlink { path: path.clone() },
                0,
            ));
            continue;
        }
        if is_dir && !path.join("pack.mcmeta").is_file() {
            // A directory without pack.mcmeta is an error card naming the
            // pack, not a silent skip.
            packs.push(error_pack(
                &path,
                location,
                PackError::MissingMeta {
                    name: display_name(&path),
                },
                0,
            ));
            continue;
        }
        let read_before = bytes_read;
        let result = inspect_entry(&path, location, icons, &mut bytes_read);
        let read_delta = bytes_read - read_before;
        let pack = match result {
            Ok(pack) => pack,
            Err(err) => error_pack(&path, location, err, read_delta),
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

/// Stale temp copies from an interrupted import are never listed.
fn is_temp_name(path: &Path) -> bool {
    path.file_name()
        .map(|name| {
            let name = name.to_string_lossy();
            name.starts_with('.') && name.ends_with(".tmp")
        })
        .unwrap_or(false)
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
    let read_before = *bytes_read;
    let name = display_name(path);
    let mut archive = open_zip(path, &name)?;

    let names: Vec<String> = archive
        .file_names()
        .map(|entry| normalize_entry_name(entry).to_string())
        .collect();
    *bytes_read += names
        .iter()
        .map(|entry| CENTRAL_DIR_ENTRY_BYTES + entry.len() as u64)
        .sum::<u64>()
        + CENTRAL_DIR_END_BYTES;

    let kind = kind_from_names(names.iter().map(String::as_str));
    let meta = meta_from_zip(&mut archive, &name, bytes_read)?;

    let icon = find_entry(&mut archive, "pack.png")
        .and_then(|entry| {
            read_zip_entry(&mut archive, &entry, &name, MAX_ICON_BYTES, bytes_read).ok()
        })
        .filter(|bytes| icon_within_pixel_cap(bytes))
        .and_then(|bytes| icons.store(&bytes).ok());

    Ok(finish_pack(
        path,
        PackLocation::Zip,
        kind,
        meta,
        icon,
        None,
        *bytes_read - read_before,
    ))
}

fn inspect_dir(path: &Path, icons: &IconCache, bytes_read: &mut u64) -> Result<Pack, PackError> {
    let read_before = *bytes_read;
    let name = display_name(path);
    let meta = meta_from_dir(path, &name, bytes_read)?;

    let children: Vec<String> = std::fs::read_dir(path)
        .map_err(|err| broken(&name, err))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    let kind = kind_from_names(children.iter().map(String::as_str));

    let icon = read_dir_icon(path, icons, bytes_read);
    Ok(finish_pack(
        path,
        PackLocation::Directory,
        kind,
        meta,
        icon,
        None,
        *bytes_read - read_before,
    ))
}

fn read_dir_icon(
    path: &Path,
    icons: &IconCache,
    bytes_read: &mut u64,
) -> Option<crate::pack::PackIcon> {
    let icon_path = path.join("pack.png");
    let metadata = std::fs::metadata(&icon_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
        return None;
    }
    let bytes = std::fs::read(&icon_path).ok()?;
    *bytes_read += bytes.len() as u64;
    if !icon_within_pixel_cap(&bytes) {
        return None;
    }
    icons.store(&bytes).ok()
}

type Archive = zip::ZipArchive<File>;

fn open_zip(path: &Path, name: &str) -> Result<Archive, PackError> {
    open_zip_capped(path, name, MAX_ZIP_ENTRIES)
}

fn open_zip_capped(path: &Path, name: &str, max_entries: usize) -> Result<Archive, PackError> {
    let file = File::open(path).map_err(|err| broken(name, err))?;
    let archive = zip::ZipArchive::new(file).map_err(|err| broken(name, err))?;
    // The entry count is capped before any name is collected.
    if archive.len() > max_entries {
        return Err(broken(
            name,
            format!("archive has more than {max_entries} entries"),
        ));
    }
    Ok(archive)
}

/// Zip entries may be written as "./pack.mcmeta". Every reader normalizes the
/// same way, so lookup and kind detection never disagree.
fn normalize_entry_name(name: &str) -> &str {
    name.trim_start_matches("./")
}

/// The first entry whose normalized name matches, under its real name.
fn find_entry(archive: &mut Archive, target: &str) -> Option<String> {
    archive
        .file_names()
        .map(str::to_string)
        .find(|name| normalize_entry_name(name) == target)
}

fn read_zip_entry(
    archive: &mut Archive,
    entry: &str,
    pack_name: &str,
    cap: u64,
    bytes_read: &mut u64,
) -> Result<Vec<u8>, PackError> {
    let file = archive
        .by_name(entry)
        .map_err(|err| broken(pack_name, err))?;
    // The declared size is never trusted: capacity is capped and the read
    // stops one byte past the cap so an oversized entry cannot fill memory.
    let mut bytes = Vec::with_capacity(file.size().min(cap) as usize);
    let mut limited = file.take(cap + 1);
    let read = limited
        .read_to_end(&mut bytes)
        .map_err(|err| broken(pack_name, err))?;
    if read as u64 > cap {
        return Err(broken(
            pack_name,
            format!("{entry} is larger than {cap} bytes"),
        ));
    }
    *bytes_read += read as u64;
    Ok(bytes)
}

fn meta_from_zip(
    archive: &mut Archive,
    pack_name: &str,
    bytes_read: &mut u64,
) -> Result<crate::pack::PackMeta, PackError> {
    // The error names the pack, not the metadata file.
    let entry = find_entry(archive, "pack.mcmeta").ok_or_else(|| PackError::MissingMeta {
        name: pack_name.to_string(),
    })?;
    let bytes = read_zip_entry(archive, &entry, pack_name, MAX_META_BYTES, bytes_read)?;
    parse_pack_meta(&bytes).map_err(|err| named_meta_error(err, pack_name))
}

fn meta_from_dir(
    path: &Path,
    pack_name: &str,
    bytes_read: &mut u64,
) -> Result<crate::pack::PackMeta, PackError> {
    let meta_path = path.join("pack.mcmeta");
    let size = std::fs::metadata(&meta_path)
        .map_err(|_| PackError::MissingMeta {
            name: pack_name.to_string(),
        })?
        .len();
    if size > MAX_META_BYTES {
        return Err(broken(pack_name, "pack.mcmeta is larger than 1 MB"));
    }
    let bytes = std::fs::read(&meta_path).map_err(|err| broken(pack_name, err))?;
    *bytes_read += bytes.len() as u64;
    parse_pack_meta(&bytes).map_err(|err| named_meta_error(err, pack_name))
}

fn named_meta_error(err: PackError, name: &str) -> PackError {
    match err {
        PackError::BrokenArchive { reason, .. } => PackError::BrokenArchive {
            name: name.to_string(),
            reason,
        },
        other => other,
    }
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
    // A range that does not fully map shows no game label at all. Falling back
    // to a narrower pack_format number would claim compatibility the range
    // does not have.
    let game_label = range
        .as_ref()
        .and_then(|range| table.game_label(kind, range));
    Pack {
        id: path.to_string_lossy().to_string(),
        name: display_name(path),
        path: path.to_path_buf(),
        location,
        kind,
        description: meta.description.clone(),
        format: range
            .and_then(|range| range.min.or(range.max))
            .map(FormatVersion::declared),
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

pub(crate) fn display_name(path: &Path) -> String {
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
        let root = normalize_entry_name(name).split('/').next().unwrap_or("");
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

/// Memoizes pack hashes by path, size, and mtime, so a second import into a
/// full resourcepacks folder does not re-read every pack on disk.
#[derive(Default)]
pub struct HashCache {
    entries: Mutex<HashMap<PathBuf, HashEntry>>,
}

struct HashEntry {
    len: u64,
    modified: Option<SystemTime>,
    sha256: String,
}

impl HashCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hash(&self, path: &Path, location: PackLocation) -> Result<String, AppError> {
        let metadata = std::fs::metadata(path)?;
        let len = metadata.len();
        let modified = metadata.modified().ok();
        let cached = {
            let entries = self.entries.lock().unwrap_or_else(|err| err.into_inner());
            entries.get(path).and_then(|entry| {
                (entry.len == len && entry.modified == modified).then(|| entry.sha256.clone())
            })
        };
        if let Some(sha256) = cached {
            return Ok(sha256);
        }
        let sha256 = hash_source(path, location)?;
        self.entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                path.to_path_buf(),
                HashEntry {
                    len,
                    modified,
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
    // A symlinked source is rejected instead of followed, so no later step can
    // remove a pack that only a link points at.
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
    // Kind and pack.mcmeta are validated before anything is copied or removed,
    // so a broken pack can never replace a working one.
    validate_meta(&source, location, &name)?;

    std::fs::create_dir_all(resourcepacks)?;
    let root = std::fs::canonicalize(resourcepacks)?;
    let sha256 = hashes.hash(&source, location)?;
    let existing = find_duplicate(&root, &sha256, hashes)?;

    // Re-importing a pack that already lives in resourcepacks is a no-op. The
    // source is never deleted, whatever the caller chose.
    if source.parent() == Some(root.as_path()) || existing.as_deref() == Some(source.as_path()) {
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
    let destination_taken = std::fs::symlink_metadata(&destination).is_ok();

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
            ImportChoice::Replace => {}
        }
    } else if destination_taken && choice != ImportChoice::Replace {
        return Ok(ImportOutcome::Collision { name });
    }

    // Copy and validate first. Until the temp copy is good, nothing old has
    // been touched, so every error below is a rollback.
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

    // One Replace removes at most one pack. When the exact duplicate lives
    // under a different name than a colliding destination, only the duplicate
    // goes to the trash and the destination is reported as a collision without
    // being deleted.
    if let Some(existing_path) = &existing {
        if existing_path != &destination && destination_taken {
            let result = trash.move_to_trash(existing_path).map_err(AppError::from);
            let _ = remove_any(&tmp);
            result?;
            return Ok(ImportOutcome::Collision { name });
        }
    }

    let removal = existing
        .clone()
        .or_else(|| destination_taken.then(|| destination.clone()));
    if let Some(target) = &removal {
        if let Err(err) = trash.move_to_trash(target) {
            let _ = remove_any(&tmp);
            return Err(err.into());
        }
    }

    if let Err(err) = std::fs::rename(&tmp, &destination) {
        let _ = remove_any(&tmp);
        return Err(err.into());
    }
    sync_dir(&root);

    let mut bytes_read = 0;
    let pack = inspect_entry(&destination, location, icons, &mut bytes_read)?;
    Ok(ImportOutcome::Imported {
        pack: Box::new(pack),
    })
}

/// Validate kind and pack.mcmeta for an import source or its temp copy.
fn validate_meta(path: &Path, location: PackLocation, name: &str) -> Result<(), PackError> {
    match location {
        PackLocation::Zip => {
            let mut archive = open_zip(path, name)?;
            let mut bytes_read = 0;
            meta_from_zip(&mut archive, name, &mut bytes_read).map(|_| ())
        }
        PackLocation::Directory => {
            let mut bytes_read = 0;
            meta_from_dir(path, name, &mut bytes_read).map(|_| ())
        }
    }
}

/// Move a pack to the system trash. Only direct children of resourcepacks are
/// eligible, so a stale path cannot delete something outside the game dir. The
/// entry itself is never canonicalized, so a symlinked pack trashes the link
/// and never the target it points at.
pub fn delete_pack(path: &Path, resourcepacks: &Path) -> Result<(), AppError> {
    delete_pack_with(path, resourcepacks, &SystemTrash)
}

pub fn delete_pack_with(
    path: &Path,
    resourcepacks: &Path,
    trash: &dyn TrashService,
) -> Result<(), AppError> {
    validate_delete_target(path, resourcepacks)?;
    trash.move_to_trash(path)?;
    Ok(())
}

/// Validate that `path` is an existing direct child (file or folder, never a
/// symlink) of the resolved `resourcepacks`. A path outside the root, nested
/// under it, missing, or a symlink is refused before any caller touches
/// `options.txt`. The entry itself is never canonicalized, so the check cannot
/// redirect a delete elsewhere.
pub fn validate_delete_target(path: &Path, resourcepacks: &Path) -> Result<(), AppError> {
    let root = std::fs::canonicalize(resourcepacks)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| PackError::NotFound {
        path: path.to_path_buf(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(PackError::Symlink {
            path: path.to_path_buf(),
        }
        .into());
    }
    if path.file_name().is_none() {
        return Err(PackError::OutsideRoot {
            path: path.to_path_buf(),
        }
        .into());
    }
    let parent = path.parent().ok_or_else(|| PackError::OutsideRoot {
        path: path.to_path_buf(),
    })?;
    let parent = std::fs::canonicalize(parent).map_err(|_| PackError::NotFound {
        path: path.to_path_buf(),
    })?;
    if parent != root {
        return Err(PackError::OutsideRoot {
            path: path.to_path_buf(),
        }
        .into());
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
            let name = display_name(path);
            let archive = open_zip(path, &name)?;
            Ok(kind_from_names(archive.file_names()))
        }
    }
}

pub(crate) fn reject_symlinks(root: &Path) -> Result<(), AppError> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(PackError::Symlink { path: entry.path() }.into());
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

/// Chunk size for pack content hashing. Shared by `hash_file` and `hash_dir`
/// via `feed_hasher`.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

fn hash_file(path: &Path) -> Result<String, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    feed_hasher(&mut file, &mut hasher)?;
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
        feed_hasher(&mut file, &mut hasher)?;
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The one chunked-read loop for pack content hashing. Both `hash_file` and
/// `hash_dir` pump through here, so the 64KiB pattern has a single owner.
/// `install::hash_file` stays separate: it hashes SHA-512 and SHA-256 in one
/// pass for the install commit. `download` stays separate: it hashes while
/// writing and enforcing the byte cap.
fn feed_hasher(reader: &mut impl Read, hasher: &mut Sha256) -> Result<(), AppError> {
    let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AppError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(PackError::Symlink { path: entry.path() }.into());
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

/// The installed pack with the same content, if any. I/O errors propagate:
/// silently skipping an unreadable pack would hide a real duplicate.
fn find_duplicate(
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
        if hashes.hash(&path, location)? == sha256 {
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

/// Permanent removal, used only for temp paths this module created itself.
/// Packs always go through `TrashService`.
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
    use std::os::unix::fs::PermissionsExt as _;

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
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

    /// Records trash calls and keeps the trashed path, like the real trash.
    struct RecordingTrash {
        dir: PathBuf,
        calls: Mutex<Vec<PathBuf>>,
        counter: AtomicU64,
    }

    impl RecordingTrash {
        fn new(dir: &Path) -> Self {
            std::fs::create_dir_all(dir).unwrap();
            RecordingTrash {
                dir: dir.to_path_buf(),
                calls: Mutex::new(Vec::new()),
                counter: AtomicU64::new(0),
            }
        }

        fn calls(&self) -> Vec<PathBuf> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl TrashService for RecordingTrash {
        fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
            self.calls.lock().unwrap().push(path.to_path_buf());
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "item".to_string());
            let index = self.counter.fetch_add(1, Ordering::Relaxed);
            let target = self.dir.join(format!("{index}-{name}"));
            std::fs::rename(path, &target).map_err(|err| PackError::Trash {
                path: path.to_path_buf(),
                reason: err.to_string(),
            })
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        IconCache,
        HashCache,
        RecordingTrash,
    ) {
        let root = tempfile::tempdir().unwrap();
        let cache_root = tempfile::tempdir().unwrap();
        let icons = IconCache::new(cache_root.path());
        let trash = RecordingTrash::new(&root.path().join("trash-box"));
        let hashes = HashCache::new();
        (root, cache_root, icons, hashes, trash)
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
        assert_eq!(
            pack.bytes_read, listing.bytes_read,
            "one pack reports its own delta"
        );
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
        let broken = listing
            .packs
            .iter()
            .find(|pack| pack.name == "Broken")
            .unwrap();
        assert_eq!(broken.error.as_ref().unwrap().code, "pack_broken_archive");
        let good = listing
            .packs
            .iter()
            .find(|pack| pack.name == "Good")
            .unwrap();
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
    fn an_icon_claiming_huge_pixels_is_skipped() {
        let mut header = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        header.extend_from_slice(&13u32.to_be_bytes());
        header.extend_from_slice(b"IHDR");
        header.extend_from_slice(&65_535u32.to_be_bytes());
        header.extend_from_slice(&65_535u32.to_be_bytes());
        header.extend_from_slice(&[0; 5]);

        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        simple_zip(&root.path().join("Lying.zip"), 34, Some(&header));

        let listing = scan_packs(root.path(), &icons).unwrap();
        let pack = &listing.packs[0];
        assert!(pack.icon.is_none(), "a hostile IHDR is never cached");
        assert!(pack.error.is_none());
    }

    #[test]
    fn directory_pack_and_both_kinds() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let dir = root.path().join("DirPack");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("pack.mcmeta"), mcmeta(15, Some(22), "Two kinds")).unwrap();
        std::fs::write(dir.join("pack.png"), b"icon").unwrap();

        let listing = scan_packs(root.path(), &icons).unwrap();
        let pack = &listing.packs[0];
        assert_eq!(pack.location, PackLocation::Directory);
        assert_eq!(pack.kind, PackKind::Both);
        assert!(pack.icon.is_some());
        assert_eq!(pack.description, "Two kinds");
        assert_eq!(
            pack.game_label.as_deref(),
            Some("Resource 1.20 to 1.20.4, data unknown"),
            "each half is labelled from its own table"
        );
    }

    #[test]
    fn a_directory_without_meta_is_an_error_card_naming_the_pack() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let nested = root.path().join("nested");
        std::fs::create_dir_all(nested.join("assets")).unwrap();
        simple_zip(&nested.join("Inside.zip"), 34, None);

        let listing = scan_packs(root.path(), &icons).unwrap();
        assert_eq!(listing.count, 1, "no recursion into nested dirs");
        let card = &listing.packs[0];
        assert_eq!(card.name, "nested");
        assert_eq!(card.error.as_ref().unwrap().code, "pack_missing_meta");
        assert!(
            card.error.as_ref().unwrap().message.contains("nested"),
            "the error names the pack: {}",
            card.error.as_ref().unwrap().message
        );
    }

    #[test]
    fn missing_meta_errors_name_the_pack() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();

        write_zip(
            &root.path().join("incoming/NoMeta.zip"),
            &[("assets/minecraft/x", b"x")],
        );
        let err = import_pack(
            &root.path().join("incoming/NoMeta.zip"),
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &HashCache::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "pack_missing_meta");
        assert!(
            err.to_string().contains("NoMeta"),
            "the error names the pack: {err}"
        );

        std::fs::create_dir_all(root.path().join("incoming/NoMetaDir/assets")).unwrap();
        let err = import_pack(
            &root.path().join("incoming/NoMetaDir"),
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &HashCache::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "pack_missing_meta");
        assert!(
            err.to_string().contains("NoMetaDir"),
            "the error names the pack: {err}"
        );
    }

    #[test]
    fn dot_prefixed_zip_entries_are_normalized() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let meta = mcmeta(34, None, "Dotted");
        write_zip(
            &root.path().join("Dotted.zip"),
            &[
                ("./pack.mcmeta", meta.as_slice()),
                ("./pack.png", b"icon"),
                ("./assets/minecraft/x", b"x"),
            ],
        );

        let listing = scan_packs(root.path(), &icons).unwrap();
        let pack = &listing.packs[0];
        assert!(pack.error.is_none(), "{:?}", pack.error);
        assert_eq!(pack.kind, PackKind::Resource);
        assert_eq!(pack.description, "Dotted");
        assert!(pack.icon.is_some());

        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let outcome = import_pack(
            &root.path().join("Dotted.zip"),
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &HashCache::new(),
        )
        .unwrap();
        assert!(
            matches!(outcome, ImportOutcome::Imported { .. }),
            "{outcome:?}"
        );
    }

    #[test]
    fn stale_temp_entries_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        simple_zip(&root.path().join("Good.zip"), 34, None);
        std::fs::create_dir_all(root.path().join(".DirPack.123-1.tmp")).unwrap();
        std::fs::write(root.path().join(".Pack.zip.123-2.tmp"), b"leftover").unwrap();

        let listing = scan_packs(root.path(), &icons).unwrap();
        assert_eq!(listing.count, 1);
        assert_eq!(listing.packs[0].name, "Good");
    }

    #[test]
    fn symlinked_packs_are_error_cards() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let target = root.path().join("Real");
        std::fs::create_dir_all(target.join("assets")).unwrap();
        std::fs::write(target.join("pack.mcmeta"), mcmeta(34, None, "real")).unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("Linked")).unwrap();

        let listing = scan_packs(root.path(), &icons).unwrap();
        let linked = listing
            .packs
            .iter()
            .find(|pack| pack.name == "Linked")
            .unwrap();
        assert_eq!(linked.error.as_ref().unwrap().code, "pack_symlink");
        let real = listing
            .packs
            .iter()
            .find(|pack| pack.name == "Real")
            .unwrap();
        assert!(real.error.is_none());
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
        let names: Vec<&str> = listing
            .packs
            .iter()
            .map(|pack| pack.name.as_str())
            .collect();
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
        let max_per_pack = listing
            .packs
            .iter()
            .map(|pack| pack.bytes_read)
            .max()
            .unwrap();
        assert!(
            max_per_pack <= 1024 * 1024,
            "per-pack cap, not the mean: {max_per_pack}"
        );
        assert!(
            max_per_pack * 50 < listing.bytes_read,
            "each pack reports its own delta, not a running total: max {max_per_pack} of {}",
            listing.bytes_read
        );
        assert!(listing.elapsed_ms < 1500.0);
    }

    #[test]
    fn a_zip_entry_over_the_cap_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        let big = vec![0u8; 2 * 1024 * 1024];
        let meta = mcmeta(34, None, "Huge meta");
        let mut entries: Vec<(&str, &[u8])> = vec![("pack.mcmeta", big.as_slice())];
        entries.push(("assets/x", meta.as_slice()));
        write_zip(&root.path().join("HugeMeta.zip"), &entries);

        let listing = scan_packs(root.path(), &icons).unwrap();
        let card = &listing.packs[0];
        assert_eq!(card.error.as_ref().unwrap().code, "pack_broken_archive");
        assert!(
            card.error.as_ref().unwrap().message.contains("larger than"),
            "{:?}",
            card.error
        );
    }

    #[test]
    fn the_zip_entry_count_is_capped() {
        let root = tempfile::tempdir().unwrap();
        let mut names: Vec<(String, &[u8])> = Vec::new();
        let meta = mcmeta(34, None, "Many");
        names.push(("pack.mcmeta".to_string(), meta.as_slice()));
        for index in 0..5 {
            names.push((format!("assets/{index}"), b"x"));
        }
        let owned: Vec<(&str, &[u8])> = names
            .iter()
            .map(|(name, bytes)| (name.as_str(), *bytes))
            .collect();
        write_zip(&root.path().join("Many.zip"), &owned);

        let name = display_name(&root.path().join("Many.zip"));
        let err = open_zip_capped(&root.path().join("Many.zip"), &name, 3).unwrap_err();
        assert_eq!(err.code(), "pack_broken_archive");
        assert!(err.to_string().contains("more than 3 entries"), "{err}");
        assert!(open_zip_capped(&root.path().join("Many.zip"), &name, 6).is_ok());
    }

    #[test]
    fn imports_zip_and_directory() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("game/resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();

        let zip_source = root.path().join("incoming/ZipPack.zip");
        simple_zip(&zip_source, 34, Some(b"icon"));
        let outcome = import_pack_with(
            &zip_source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        let pack = match outcome {
            ImportOutcome::Imported { pack } => pack,
            other => panic!("expected import, got {other:?}"),
        };
        assert_eq!(pack.name, "ZipPack");
        assert!(resourcepacks.join("ZipPack.zip").is_file());

        let dir_source = root.path().join("incoming/DirPack");
        std::fs::create_dir_all(dir_source.join("assets")).unwrap();
        std::fs::write(dir_source.join("pack.mcmeta"), mcmeta(7, None, "Dir")).unwrap();
        let outcome = import_pack_with(
            &dir_source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(matches!(outcome, ImportOutcome::Imported { .. }));
        assert!(resourcepacks.join("DirPack/pack.mcmeta").is_file());
        assert!(trash.calls().is_empty());
    }

    #[test]
    fn duplicate_offers_skip_and_replace() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);

        import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        let duplicate = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(
            matches!(duplicate, ImportOutcome::Duplicate { .. }),
            "{duplicate:?}"
        );

        let skipped = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Skip,
            &hashes,
            &trash,
        )
        .unwrap();
        assert_eq!(skipped, ImportOutcome::Skipped);
        assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);
        assert!(trash.calls().is_empty());

        let replaced = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(matches!(replaced, ImportOutcome::Imported { .. }));
        assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);
        assert_eq!(
            trash.calls().len(),
            1,
            "one Replace moves exactly one pack to the trash"
        );
    }

    #[test]
    fn name_collision_needs_replace() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let first = root.path().join("one/Pack.zip");
        simple_zip(&first, 34, None);
        import_pack_with(
            &first,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();

        let second = root.path().join("two/Pack.zip");
        write_zip(
            &second,
            &[
                ("pack.mcmeta", mcmeta(55, None, "Different").as_slice()),
                ("assets/minecraft/x", b"x"),
            ],
        );
        let collision = import_pack_with(
            &second,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(
            matches!(collision, ImportOutcome::Collision { .. }),
            "{collision:?}"
        );
        assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);
        assert!(trash.calls().is_empty(), "a collision removes nothing");

        let replaced = import_pack_with(
            &second,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(matches!(replaced, ImportOutcome::Imported { .. }));
        assert_eq!(trash.calls().len(), 1);
        let listing = scan_packs(&resourcepacks, &icons).unwrap();
        assert_eq!(listing.packs[0].description, "Different");
    }

    #[test]
    fn duplicate_plus_collision_removes_at_most_one_pack() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();

        let alpha = root.path().join("alpha/Alpha.zip");
        simple_zip(&alpha, 34, None);
        import_pack_with(
            &alpha,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        let other = root.path().join("other/Pack.zip");
        write_zip(
            &other,
            &[
                ("pack.mcmeta", mcmeta(55, None, "Colliding").as_slice()),
                ("assets/minecraft/x", b"x"),
            ],
        );
        import_pack_with(
            &other,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();

        // The source has Alpha's content but Pack.zip's name.
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);
        let first = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(
            matches!(first, ImportOutcome::Collision { .. }),
            "{first:?}"
        );
        assert_eq!(
            pack_names(&resourcepacks).unwrap().len(),
            1,
            "one Replace removes at most one pack"
        );
        assert_eq!(trash.calls().len(), 1);
        assert_eq!(
            trash.calls()[0],
            resourcepacks.join("Alpha.zip"),
            "only the exact duplicate goes; the colliding destination stays"
        );
        assert!(resourcepacks.join("Pack.zip").is_file());
        assert!(source.is_file(), "the source is never deleted");

        let second = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(
            matches!(second, ImportOutcome::Imported { .. }),
            "{second:?}"
        );
        assert_eq!(trash.calls().len(), 2);
        let listing = scan_packs(&resourcepacks, &icons).unwrap();
        assert_eq!(listing.count, 1);
        assert_eq!(listing.packs[0].description, "Demo pack");
    }

    #[test]
    fn reimporting_a_pack_inside_resourcepacks_keeps_it() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);
        import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();

        let installed = resourcepacks.join("Pack.zip");
        for choice in [
            ImportChoice::Default,
            ImportChoice::Replace,
            ImportChoice::Skip,
        ] {
            let outcome =
                import_pack_with(&installed, &resourcepacks, &icons, choice, &hashes, &trash)
                    .unwrap();
            assert!(
                matches!(outcome, ImportOutcome::Imported { .. }),
                "re-importing from the destination is a no-op success: {outcome:?}"
            );
            assert!(installed.is_file(), "{choice:?} must not delete the source");
            assert!(source.is_file());
            assert!(
                trash.calls().is_empty(),
                "{choice:?} must not trash anything"
            );
            assert_eq!(pack_names(&resourcepacks).unwrap().len(), 1);
        }
    }

    #[test]
    fn replace_rolls_back_before_touching_the_old_pack() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);
        import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();

        // Make the copy fail: the destination dir refuses new files.
        let mut perms = std::fs::metadata(&resourcepacks).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(&resourcepacks, perms.clone()).unwrap();
        let err = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
        )
        .unwrap_err();
        perms.set_mode(0o755);
        std::fs::set_permissions(&resourcepacks, perms).unwrap();

        assert_eq!(err.code(), "io");
        assert!(
            trash.calls().is_empty(),
            "nothing is trashed before the new copy is safe"
        );
        assert!(resourcepacks.join("Pack.zip").is_file());
        assert!(source.is_file());
    }

    #[test]
    fn an_unreadable_installed_pack_fails_instead_of_hiding_a_duplicate() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let locked = resourcepacks.join("Locked.zip");
        simple_zip(&locked, 34, None);
        let mut perms = std::fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&locked, perms.clone()).unwrap();

        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);
        let err = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap_err();
        perms.set_mode(0o644);
        std::fs::set_permissions(&locked, perms).unwrap();

        assert_eq!(err.code(), "io", "I/O errors propagate: {err}");
        assert!(trash.calls().is_empty());
    }

    #[test]
    fn rejects_unknown_kind_and_symlinks() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();

        let empty = root.path().join("empty.zip");
        write_zip(&empty, &[("readme.txt", b"nope")]);
        let err = import_pack_with(
            &empty,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap_err();
        assert_eq!(err.code(), "pack_unknown_kind");

        let source = root.path().join("linked");
        std::fs::create_dir_all(source.join("assets")).unwrap();
        std::fs::write(source.join("pack.mcmeta"), mcmeta(34, None, "linked")).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", source.join("assets/escape.txt")).unwrap();
        let err = import_pack_with(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap_err();
        assert_eq!(err.code(), "pack_symlink");

        let zip_source = root.path().join("incoming/Real.zip");
        simple_zip(&zip_source, 34, None);
        let link = root.path().join("incoming/Link.zip");
        std::os::unix::fs::symlink(&zip_source, &link).unwrap();
        let err = import_pack_with(
            &link,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap_err();
        assert_eq!(
            err.code(),
            "pack_symlink",
            "a symlinked source is not followed"
        );
    }

    #[test]
    fn delete_moves_a_direct_child_to_the_freedesktop_trash() {
        let root = tempfile::tempdir().unwrap();
        let resourcepacks = root.path().join("resourcepacks");
        let pack = resourcepacks.join("TrashMe");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), mcmeta(34, None, "trash")).unwrap();

        // The freedesktop trash lives under $XDG_DATA_HOME/Trash. Point it at a
        // temp home so the test proves the trash call, not just the absence.
        let data_home = root.path().join("xdg-data");
        std::fs::create_dir_all(data_home.join("Trash/files")).unwrap();
        std::env::set_var("XDG_DATA_HOME", &data_home);
        delete_pack(&pack, &resourcepacks).unwrap();
        std::env::remove_var("XDG_DATA_HOME");

        assert!(!pack.exists());
        assert!(
            data_home.join("Trash/files/TrashMe").exists(),
            "the pack must land in the freedesktop trash, not be unlinked"
        );
    }

    #[test]
    fn delete_refuses_a_symlinked_pack() {
        let (root, _cache_root, _icons, _hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        let target = resourcepacks.join("Target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("pack.mcmeta"), mcmeta(34, None, "target")).unwrap();
        let link = resourcepacks.join("Link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = delete_pack_with(&link, &resourcepacks, &trash).unwrap_err();
        assert_eq!(err.code(), "pack_symlink");
        assert!(trash.calls().is_empty(), "nothing is trashed");
        assert!(link.exists(), "the link is untouched");
        assert!(
            target.join("pack.mcmeta").is_file(),
            "the target is untouched"
        );
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
