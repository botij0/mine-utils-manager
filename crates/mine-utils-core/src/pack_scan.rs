use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::error::AppError;
use crate::format_table::FormatTable;
use crate::icon_cache::{icon_within_pixel_cap, IconCache};
use crate::pack::{FormatVersion, Pack, PackError, PackIssue, PackKind, PackLocation};
use crate::pack_meta::parse_pack_meta;

use crate::pack_import::{SystemTrash, TrashService};
pub(crate) use crate::pack_source::display_name;
#[cfg(test)]
use crate::pack_source::open_zip_capped;
use crate::pack_source::PackSource;
pub use crate::pack_source::{MAX_ICON_BYTES, MAX_META_BYTES, MAX_ZIP_ENTRIES};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct PackListing {
    pub packs: Vec<Pack>,
    pub count: usize,
    pub holding_entries: Vec<crate::holding_recovery::HoldingEntry>,
    /// Total bytes read across the listing. Each pack carries its own delta.
    pub bytes_read: u64,
    pub elapsed_ms: f64,
}

/// List only direct children of resourcepacks. A zip or a directory with
/// pack.mcmeta is a pack. Folders missing pack.mcmeta also get error cards.
/// Broken entries become error cards so one bad pack
/// never hides the rest.
pub fn scan_packs(resourcepacks: &Path, icons: &IconCache) -> Result<PackListing, AppError> {
    let holding_entries = crate::holding_recovery::list(resourcepacks)?;
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
        if path.file_name().is_some_and(|name| name.to_str().is_none()) {
            let display_path = PathBuf::from(path.to_string_lossy().into_owned());
            let mut pack = error_pack(
                &display_path,
                location,
                PackError::NonUtf8Name {
                    name: display_name(&display_path),
                },
                0,
            );
            pack.id = format!("non-utf8:{:?}", path.as_os_str());
            packs.push(pack);
            continue;
        }
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
        holding_entries,
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
pub(crate) fn is_temp_name(path: &Path) -> bool {
    path.file_name()
        .map(|name| {
            let name = name.to_string_lossy();
            name.starts_with('.') && name.ends_with(".tmp")
        })
        .unwrap_or(false)
}

pub(crate) fn inspect_entry(
    path: &Path,
    location: PackLocation,
    icons: &IconCache,
    bytes_read: &mut u64,
) -> Result<Pack, PackError> {
    let read_before = *bytes_read;
    let source = PackSource::new(path, location)?;
    let names = source.entry_names(bytes_read)?;
    let kind = kind_from_names(names.iter().map(String::as_str));
    let name = display_name(path);
    let meta = read_meta(&source, &name, bytes_read)?;
    let icon = source
        .read_capped("pack.png", MAX_ICON_BYTES, bytes_read)
        .ok()
        .flatten()
        .filter(|bytes| icon_within_pixel_cap(bytes))
        .and_then(|bytes| icons.store(&bytes).ok());
    Ok(finish_pack(
        path,
        location,
        kind,
        meta,
        icon,
        None,
        *bytes_read - read_before,
    ))
}

pub(crate) fn read_meta(
    source: &PackSource<'_>,
    name: &str,
    bytes_read: &mut u64,
) -> Result<crate::pack::PackMeta, PackError> {
    let bytes = source
        .read_capped("pack.mcmeta", MAX_META_BYTES, bytes_read)?
        .ok_or_else(|| PackError::MissingMeta {
            name: name.to_string(),
        })?;
    parse_pack_meta(&bytes).map_err(|err| named_meta_error(err, name))
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
            symlink_path: match &err {
                PackError::Symlink { path } => Some(path.clone()),
                _ => None,
            },
        }),
        bytes_read,
    )
}

pub(crate) fn kind_from_names<'a>(names: impl Iterator<Item = &'a str>) -> PackKind {
    let mut assets = false;
    let mut data = false;
    for name in names {
        let root = name.split('/').next().unwrap_or("");
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

pub(crate) fn reject_ambiguous_name(path: &Path, root: &Path) -> Result<(), AppError> {
    if let Some(name) = path.file_name() {
        let mut ambiguous = name.to_str().is_none();
        if name.to_string_lossy().contains('�') {
            let entries = match std::fs::read_dir(root) {
                Ok(entries) => Some(entries),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                Err(err) => return Err(err.into()),
            };
            for entry in entries.into_iter().flatten() {
                let sibling = entry?.file_name();
                if sibling.to_str().is_none() && sibling.to_string_lossy() == name.to_string_lossy()
                {
                    ambiguous = true;
                    break;
                }
            }
        }
        if ambiguous {
            return Err(PackError::NonUtf8Name {
                name: display_name(path),
            }
            .into());
        }
    }
    Ok(())
}

/// Validate that `path` is an existing direct child (file or folder, never a
/// symlink) of the resolved `resourcepacks`. A path outside the root, nested
/// under it, missing, or a symlink is refused before any caller touches
/// `options.txt`. The entry itself is never canonicalized, so the check cannot
/// redirect a delete elsewhere.
pub fn validate_delete_target(path: &Path, resourcepacks: &Path) -> Result<(), AppError> {
    reject_ambiguous_name(path, resourcepacks)?;
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

pub(crate) fn detect_kind(path: &Path, location: PackLocation) -> Result<PackKind, AppError> {
    let source = PackSource::new(path, location)?;
    let names = source.entry_names(&mut 0)?;
    Ok(kind_from_names(names.iter().map(String::as_str)))
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
    use crate::fs_commit::{atomic_no_replace, no_replace_fallback};
    use crate::pack::{FormatVersion, PackKind};
    use crate::pack::{ImportChoice, ImportOutcome};
    use crate::pack_import::{
        find_duplicate, import_pack, import_pack_with, import_pack_with_commit, HashCache,
    };
    use std::fs::File;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

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

    #[cfg(unix)]
    #[test]
    fn non_utf8_name_is_serializable_and_cannot_delete_collision() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let (_cache_root, icons) = cache();
        simple_zip(&root.path().join("Good.zip"), 55, None);
        let bad = root
            .path()
            .join(std::ffi::OsStr::from_bytes(b"Caf\xe9.zip"));
        std::fs::write(&bad, b"bad").unwrap();
        let listing = scan_packs(root.path(), &icons).unwrap();
        let json = serde_json::to_value(&listing).unwrap();
        let packs = json["packs"].as_array().unwrap();
        assert_eq!(packs.len(), 2);
        let good = packs.iter().find(|p| p["name"] == "Good").unwrap();
        assert_eq!(good["error"], serde_json::Value::Null);
        let card = packs.iter().find(|p| p["name"] == "Caf�").unwrap();
        assert_eq!(card["error"]["code"], "pack_non_utf8_name");
        let display_path = PathBuf::from(card["path"].as_str().unwrap());
        std::fs::write(&display_path, b"collision").unwrap();
        let trash = RecordingTrash::new(&root.path().join("trash"));
        assert!(delete_pack_with(&display_path, root.path(), &trash).is_err());
        assert_eq!(std::fs::read(&display_path).unwrap(), b"collision");
        assert_eq!(std::fs::read(&bad).unwrap(), b"bad");
        std::fs::remove_file(&bad).unwrap();
        delete_pack_with(&display_path, root.path(), &trash).unwrap();
        assert!(!display_path.exists());
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
    fn a_rival_created_after_the_collision_check_keeps_its_bytes() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("game/resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let zip_source = root.path().join("incoming/ZipPack.zip");
        simple_zip(&zip_source, 34, None);
        let dir_source = root.path().join("incoming/DirPack");
        std::fs::create_dir_all(dir_source.join("assets")).unwrap();
        std::fs::write(dir_source.join("pack.mcmeta"), mcmeta(7, None, "Dir")).unwrap();

        for (source, file_name, name) in [
            (&zip_source, "ZipPack.zip", "ZipPack"),
            (&dir_source, "DirPack", "DirPack"),
        ] {
            let rival = resourcepacks.join(file_name);
            let outcome = import_pack_with_commit(
                source,
                &resourcepacks,
                &icons,
                ImportChoice::Default,
                &hashes,
                &trash,
                |tmp, dest| {
                    std::fs::write(&rival, b"rival").unwrap();
                    atomic_no_replace(tmp, dest)
                },
            )
            .unwrap();
            assert_eq!(
                outcome,
                ImportOutcome::Collision {
                    name: name.to_string()
                }
            );
            assert_eq!(std::fs::read(&rival).unwrap(), b"rival");
        }
        let mut left: Vec<String> = std::fs::read_dir(&resourcepacks)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(left, ["DirPack", "ZipPack.zip"]);
        assert!(trash.calls().is_empty());
    }

    fn entry_summary(path: &Path) -> String {
        let entry = std::fs::symlink_metadata(path).unwrap();
        if entry.is_symlink() {
            format!("symlink to {}", std::fs::read_link(path).unwrap().display())
        } else if entry.is_dir() {
            format!(
                "dir with {} entries",
                std::fs::read_dir(path).unwrap().count()
            )
        } else {
            format!("file {}", std::fs::read_to_string(path).unwrap())
        }
    }

    #[test]
    fn a_folder_import_through_the_link_fallback_lands_or_reports_the_rival() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let source = root.path().join("incoming/DirPack");
        std::fs::create_dir_all(source.join("assets")).unwrap();
        std::fs::write(source.join("pack.mcmeta"), mcmeta(7, None, "Dir")).unwrap();
        let destination = resourcepacks.join("DirPack");
        type MakeRival = fn(&Path);
        let rivals: [(&str, MakeRival); 4] = [
            ("no rival", |_| {}),
            ("dangling symlink", |path| {
                std::os::unix::fs::symlink("missing", path).unwrap()
            }),
            ("empty dir", |path| std::fs::create_dir(path).unwrap()),
            ("regular file", |path| {
                std::fs::write(path, b"rival").unwrap()
            }),
        ];

        let mut seen = Vec::new();
        for (label, make_rival) in rivals {
            let outcome = import_pack_with_commit(
                &source,
                &resourcepacks,
                &icons,
                ImportChoice::Default,
                &hashes,
                &trash,
                |tmp, dest| {
                    make_rival(dest);
                    no_replace_fallback(tmp, dest)
                },
            );
            let outcome = match outcome {
                Ok(ImportOutcome::Imported { .. }) => "imported".to_string(),
                Ok(ImportOutcome::Collision { name }) => format!("collision {name}"),
                other => format!("{other:?}"),
            };
            seen.push(format!(
                "{label}: {outcome}; destination {}; resourcepacks [{}]",
                entry_summary(&destination),
                Vec::from_iter(pack_names(&resourcepacks).unwrap()).join(", ")
            ));
            if std::fs::symlink_metadata(&destination).unwrap().is_dir() {
                std::fs::remove_dir_all(&destination).unwrap();
            } else {
                std::fs::remove_file(&destination).unwrap();
            }
        }

        assert_eq!(
            seen,
            [
                "no rival: imported; destination dir with 2 entries; resourcepacks [DirPack]",
                "dangling symlink: collision DirPack; destination symlink to missing; resourcepacks [DirPack]",
                "empty dir: collision DirPack; destination dir with 0 entries; resourcepacks [DirPack]",
                "regular file: collision DirPack; destination file rival; resourcepacks [DirPack]",
            ]
        );
        assert!(trash.calls().is_empty());
    }

    #[test]
    fn replace_names_the_saved_original_when_a_rival_takes_its_name() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let old = root.path().join("one/Pack.zip");
        simple_zip(&old, 34, None);
        import_pack_with(
            &old,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        let new = root.path().join("two/Pack.zip");
        write_zip(
            &new,
            &[
                ("pack.mcmeta", mcmeta(55, None, "Different").as_slice()),
                ("assets/minecraft/x", b"x"),
            ],
        );
        let rival = resourcepacks.join("Pack.zip");

        let err = import_pack_with_commit(
            &new,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
            |tmp, dest| {
                std::fs::write(&rival, b"rival").unwrap();
                atomic_no_replace(tmp, dest)
            },
        )
        .unwrap_err();
        let holding = std::fs::read_dir(&resourcepacks)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| is_temp_name(path))
            .unwrap();
        assert_eq!(
            std::fs::read(&holding).unwrap(),
            std::fs::read(&old).unwrap()
        );
        assert_eq!(std::fs::read(&rival).unwrap(), b"rival");
        assert_eq!(trash.calls(), Vec::<PathBuf>::new());
        assert_eq!(err.code(), "io");
        assert!(err.to_string().contains(&holding.display().to_string()));
        assert!(err.to_string().contains("Restart Mine Utils to restore"));
        assert_eq!(scan_packs(&resourcepacks, &icons).unwrap().count, 1);
        assert_eq!(
            find_duplicate(
                &resourcepacks,
                &hashes.hash(&old, PackLocation::Zip).unwrap(),
                &hashes
            )
            .unwrap(),
            None
        );
        println!(
            "rival preserved; original bytes retained at {}; error {err}",
            holding.display()
        );
    }

    #[test]
    fn replace_commit_failure_restores_the_original_without_trash() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        std::fs::create_dir_all(&resourcepacks).unwrap();
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);
        let original = resourcepacks.join("Pack.zip");
        std::fs::copy(&source, &original).unwrap();
        let before = std::fs::read(&original).unwrap();

        let err = import_pack_with_commit(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
            |_, dest| {
                assert!(!dest.exists(), "original is held before committing");
                Err(std::io::Error::other("injected commit failure"))
            },
        )
        .unwrap_err();

        assert_eq!(err.code(), "io");
        assert_eq!(std::fs::read(&original).ok(), Some(before));
        assert_eq!(trash.calls(), Vec::<PathBuf>::new());
        assert_eq!(
            Vec::from_iter(pack_names(&resourcepacks).unwrap()),
            ["Pack.zip"]
        );
        println!(
            "injected commit failure; original bytes restored at {}; trash calls 0",
            original.display()
        );
    }

    #[test]
    fn folder_replace_commit_failure_restores_the_original() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        let old = resourcepacks.join("Faithful");
        let source = root.path().join("incoming/Faithful-1.21");
        for path in [&old, &source] {
            std::fs::create_dir_all(path.join("assets")).unwrap();
            std::fs::write(path.join("pack.mcmeta"), mcmeta(34, None, "Faithful")).unwrap();
        }
        let err = import_pack_with_commit(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Replace,
            &hashes,
            &trash,
            |_, _| {
                assert!(!old.exists());
                Err(std::io::Error::other("injected folder commit failure"))
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "io");
        assert_eq!(
            std::fs::read(old.join("pack.mcmeta")).unwrap(),
            mcmeta(34, None, "Faithful")
        );
        assert_eq!(
            Vec::from_iter(pack_names(&resourcepacks).unwrap()),
            ["Faithful"]
        );
        assert_eq!(trash.calls(), Vec::<PathBuf>::new());
        println!("folder commit failed after holding rename; original restored at {}; listing [Faithful]; trash calls 0", old.display());
    }

    #[test]
    fn failed_trash_reports_where_the_original_remains_after_install() {
        struct FailingTrash;
        impl TrashService for FailingTrash {
            fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
                Err(PackError::Trash {
                    path: path.to_path_buf(),
                    reason: "injected trash failure".to_string(),
                })
            }
        }
        for filename in ["Faithful", "Faithful-1.21"] {
            let (root, _cache_root, icons, hashes, _) = setup();
            let resourcepacks = root.path().join("resourcepacks");
            let old = resourcepacks.join("Faithful");
            let source = root.path().join("incoming").join(filename);
            for path in [&old, &source] {
                std::fs::create_dir_all(path.join("assets")).unwrap();
                std::fs::write(path.join("pack.mcmeta"), mcmeta(34, None, "Faithful")).unwrap();
            }
            let err = import_pack_with(
                &source,
                &resourcepacks,
                &icons,
                ImportChoice::Replace,
                &hashes,
                &FailingTrash,
            )
            .unwrap_err();
            let original = if filename == "Faithful" {
                std::fs::read_dir(&resourcepacks)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| is_temp_name(p))
                    .unwrap()
            } else {
                old
            };
            assert!(err.to_string().contains(&original.display().to_string()));
            assert_eq!(
                std::fs::read(original.join("pack.mcmeta")).unwrap(),
                mcmeta(34, None, "Faithful")
            );
            assert_eq!(
                std::fs::read(resourcepacks.join(filename).join("pack.mcmeta")).unwrap(),
                mcmeta(34, None, "Faithful")
            );
        }
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
    fn duplicate_plus_collision_preserves_both_installed_packs() {
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
            Vec::from_iter(pack_names(&resourcepacks).unwrap()),
            ["Alpha.zip", "Pack.zip"]
        );
        assert_eq!(trash.calls(), Vec::<PathBuf>::new());
        let listing = scan_packs(&resourcepacks, &icons).unwrap();
        assert_eq!(listing.packs[0].description, "Demo pack");
        assert_eq!(listing.packs[1].description, "Colliding");
        assert!(source.is_file());
    }

    #[test]
    fn in_place_import_does_not_hash_the_pack_or_siblings() {
        let (root, _cache_root, icons, hashes, trash) = setup();
        let packs = root.path().join("resourcepacks");
        std::fs::create_dir_all(&packs).unwrap();
        simple_zip(&packs.join("Sibling.zip"), 34, None);
        simple_zip(&packs.join("Pack.zip"), 34, None);
        let outcome = import_pack_with(
            &packs.join("Pack.zip"),
            &packs,
            &icons,
            ImportChoice::Default,
            &hashes,
            &trash,
        )
        .unwrap();
        assert!(matches!(outcome, ImportOutcome::Imported { .. }));
        assert_eq!(hashes.entries.lock().unwrap().len(), 0);
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
    fn import_succeeds_next_to_installed_pack_with_nested_symlink() {
        let (root, _cache_root, icons, hashes, _trash) = setup();
        let resourcepacks = root.path().join("resourcepacks");
        let assets = resourcepacks.join("Linked/assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(
            resourcepacks.join("Linked/pack.mcmeta"),
            mcmeta(34, None, "linked"),
        )
        .unwrap();
        std::os::unix::fs::symlink("/etc/hostname", assets.join("link.txt")).unwrap();
        let source = root.path().join("incoming/Pack.zip");
        simple_zip(&source, 34, None);

        let result = import_pack(
            &source,
            &resourcepacks,
            &icons,
            ImportChoice::Default,
            &hashes,
        );
        assert!(
            matches!(result, Ok(ImportOutcome::Imported { .. })),
            "{result:?}"
        );
        assert_eq!(
            std::fs::read(resourcepacks.join("Pack.zip")).unwrap(),
            std::fs::read(source).unwrap()
        );
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
