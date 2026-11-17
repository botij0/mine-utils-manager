use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::pack::{PackError, PackLocation};

/// A pack.png larger than this is ignored rather than cached.
pub const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// pack.mcmeta larger than this is treated as broken.
pub const MAX_META_BYTES: u64 = 1024 * 1024;
/// A central directory with more entries than this is treated as hostile.
pub const MAX_ZIP_ENTRIES: usize = 65_536;
/// Rough size of one zip central directory record plus its 22 byte end record.
const CENTRAL_DIR_ENTRY_BYTES: u64 = 46;
const CENTRAL_DIR_END_BYTES: u64 = 22;

/// Folder sources reject every symlink, including links below content roots.
/// Zip entries remain archive bytes and are never resolved as filesystem links.
pub(crate) enum PackSource<'a> {
    Zip {
        path: &'a Path,
        reader: RefCell<Option<ZipReader>>,
    },
    Dir {
        path: &'a Path,
        validated: Cell<bool>,
    },
}

pub(crate) struct ZipReader {
    archive: zip::ZipArchive<File>,
    entries: HashMap<String, String>,
}

#[derive(PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Zip(u64, Option<SystemTime>),
    Directory(Vec<(PathBuf, u64, Option<SystemTime>)>),
}

impl<'a> PackSource<'a> {
    pub(crate) fn new(path: &'a Path, location: PackLocation) -> Result<Self, PackError> {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| PackError::NotFound {
            path: path.to_path_buf(),
        })?;
        if metadata.is_symlink() {
            return Err(PackError::Symlink {
                path: path.to_path_buf(),
            });
        }
        Ok(match location {
            PackLocation::Zip => Self::Zip {
                path,
                reader: RefCell::new(None),
            },
            PackLocation::Directory => Self::Dir {
                path,
                validated: Cell::new(false),
            },
        })
    }

    pub(crate) fn entry_names(&self, bytes_read: &mut u64) -> Result<Vec<String>, PackError> {
        match self {
            Self::Zip { path, reader } => with_zip(path, reader, |reader| {
                let names: Vec<_> = reader
                    .archive
                    .file_names()
                    .map(|name| normalize_entry_name(name).to_string())
                    .collect();
                *bytes_read += names
                    .iter()
                    .map(|name| CENTRAL_DIR_ENTRY_BYTES + name.len() as u64)
                    .sum::<u64>()
                    + CENTRAL_DIR_END_BYTES;
                Ok(names)
            }),
            Self::Dir { path, validated } => {
                validate_folder(path, validated)?;
                Ok(std::fs::read_dir(path)
                    .map_err(|err| broken(&display_name(path), err))?
                    .filter_map(|entry| entry.ok())
                    .map(|entry| {
                        let mut name = entry.file_name().to_string_lossy().to_string();
                        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                            name.push('/');
                        }
                        name
                    })
                    .collect())
            }
        }
    }

    pub(crate) fn read_capped(
        &self,
        name: &str,
        cap: u64,
        bytes_read: &mut u64,
    ) -> Result<Option<Vec<u8>>, PackError> {
        match self {
            Self::Zip { path, reader } => with_zip(path, reader, |reader| {
                let pack_name = display_name(path);
                let Some(entry) = reader.entries.get(name) else {
                    return Ok(None);
                };
                let file = reader
                    .archive
                    .by_name(entry)
                    .map_err(|err| broken(&pack_name, err))?;
                read_limited(file, name, &pack_name, cap, bytes_read).map(Some)
            }),
            Self::Dir { path, validated } => {
                validate_folder(path, validated)?;
                let target = path.join(name);
                let metadata = match std::fs::symlink_metadata(&target) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(err) => return Err(broken(&display_name(path), err)),
                };
                if !metadata.is_file() {
                    return Ok(None);
                }
                let file =
                    open_folder_file(&target).map_err(|err| broken(&display_name(path), err))?;
                read_limited(file, name, &display_name(path), cap, bytes_read).map(Some)
            }
        }
    }

    pub(crate) fn fingerprint(&self) -> Result<Fingerprint, AppError> {
        match self {
            Self::Zip { path, .. } => {
                let metadata = std::fs::metadata(path)?;
                Ok(Fingerprint::Zip(metadata.len(), metadata.modified().ok()))
            }
            Self::Dir { path, .. } => {
                let mut files = Vec::new();
                collect_files(path, path, &mut files)?;
                files.sort();
                let files = files
                    .into_iter()
                    .map(|relative| {
                        let metadata = std::fs::metadata(path.join(&relative))?;
                        Ok((relative, metadata.len(), metadata.modified().ok()))
                    })
                    .collect::<Result<Vec<_>, std::io::Error>>()?;
                Ok(Fingerprint::Directory(files))
            }
        }
    }

    pub(crate) fn digest(&self) -> Result<String, AppError> {
        match self {
            Self::Zip { path, .. } => hash_file(path),
            Self::Dir { path, .. } => hash_dir(path),
        }
    }
}

fn with_zip<T>(
    path: &Path,
    cached: &RefCell<Option<ZipReader>>,
    read: impl FnOnce(&mut ZipReader) -> Result<T, PackError>,
) -> Result<T, PackError> {
    let mut cached = cached.borrow_mut();
    let reader = match cached.as_mut() {
        Some(reader) => reader,
        None => {
            let archive = open_zip_capped(path, &display_name(path), MAX_ZIP_ENTRIES)?;
            let mut entries = HashMap::with_capacity(archive.len());
            for name in archive.file_names() {
                entries
                    .entry(normalize_entry_name(name).to_string())
                    .or_insert_with(|| name.to_string());
            }
            cached.insert(ZipReader { archive, entries })
        }
    };
    read(reader)
}

fn validate_folder(path: &Path, validated: &Cell<bool>) -> Result<(), PackError> {
    if !validated.get() {
        reject_symlinks(path).map_err(|err| pack_error(path, err))?;
        validated.set(true);
    }
    Ok(())
}

fn open_folder_file(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn read_limited(
    reader: impl Read,
    entry: &str,
    pack_name: &str,
    cap: u64,
    bytes_read: &mut u64,
) -> Result<Vec<u8>, PackError> {
    let mut bytes = Vec::new();
    // Cap actual bytes because zip headers can understate the expanded size.
    let read = reader
        .take(cap.saturating_add(1))
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

pub(crate) fn open_zip_capped(
    path: &Path,
    name: &str,
    max_entries: usize,
) -> Result<zip::ZipArchive<File>, PackError> {
    #[cfg(test)]
    SOURCE_PASSES.with(|counts| counts.borrow_mut().0 += 1);
    let file = File::open(path).map_err(|err| broken(name, err))?;
    let archive = zip::ZipArchive::new(file).map_err(|err| broken(name, err))?;
    if archive.len() > max_entries {
        return Err(broken(
            name,
            format!("archive has more than {max_entries} entries"),
        ));
    }
    Ok(archive)
}

fn normalize_entry_name(name: &str) -> &str {
    // Lookup and kind detection must see the same root for dotted zip entries.
    name.trim_start_matches("./")
}

pub(crate) fn reject_symlinks(root: &Path) -> Result<(), AppError> {
    #[cfg(test)]
    SOURCE_PASSES.with(|counts| counts.borrow_mut().1 += 1);
    collect_files(root, root, &mut Vec::new())
}

fn pack_error(path: &Path, err: AppError) -> PackError {
    match err {
        AppError::Pack(err) => err,
        err => broken(&display_name(path), err),
    }
}

fn broken(name: &str, reason: impl std::fmt::Display) -> PackError {
    PackError::BrokenArchive {
        name: name.to_string(),
        reason: reason.to_string(),
    }
}

const HASH_CHUNK_BYTES: usize = 64 * 1024;

fn hash_file(path: &Path) -> Result<String, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    feed_hasher(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

fn hash_dir(root: &Path) -> Result<String, AppError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        let mut file = open_folder_file(&root.join(&relative))?;
        feed_hasher(&mut file, &mut hasher)?;
    }
    Ok(hex::encode(hasher.finalize()))
}

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

#[cfg(test)]
thread_local! {
    static SOURCE_PASSES: std::cell::RefCell<(usize, usize)> = const { std::cell::RefCell::new((0, 0)) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[cfg(unix)]
    #[test]
    fn folder_file_open_refuses_a_swapped_leaf_symlink() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("pack.mcmeta");
        std::fs::write(&target, b"original").unwrap();
        assert!(std::fs::symlink_metadata(&target).unwrap().is_file());
        std::fs::remove_file(&target).unwrap();
        std::fs::write(root.path().join("outside"), b"outside").unwrap();
        std::os::unix::fs::symlink("outside", &target).unwrap();
        assert_eq!(
            open_folder_file(&target).unwrap_err().raw_os_error(),
            Some(libc::ELOOP)
        );
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, b"real").unwrap();
        let mut bytes = String::new();
        open_folder_file(&target)
            .unwrap()
            .read_to_string(&mut bytes)
            .unwrap();
        assert_eq!(bytes, "real");
    }

    #[cfg(unix)]
    #[test]
    fn folder_file_open_refuses_a_swapped_fifo_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("pack.mcmeta");
        let c_path = std::ffi::CString::new(target.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_folder_file(&target).is_err());
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2)),
            Ok(true),
            "opening a FIFO must fail instead of blocking"
        );
    }

    #[test]
    fn scan_opens_each_archive_and_walks_each_folder_once() {
        let root = tempfile::tempdir().unwrap();
        let packs = root.path().join("packs");
        let folder = packs.join("Dir");
        std::fs::create_dir_all(folder.join("data/test")).unwrap();
        let meta = br#"{"pack":{"pack_format":61,"description":"one pass"}}"#;
        std::fs::write(folder.join("pack.mcmeta"), meta).unwrap();
        let mut archive = zip::ZipWriter::new(File::create(packs.join("Zip.zip")).unwrap());
        archive
            .start_file("./pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(meta).unwrap();
        archive
            .add_directory("data/", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.finish().unwrap();
        SOURCE_PASSES.with(|counts| *counts.borrow_mut() = (0, 0));
        let listing =
            crate::scan_packs(&packs, &crate::IconCache::new(&root.path().join("icons"))).unwrap();
        assert_eq!(
            listing
                .packs
                .iter()
                .map(|p| p.description.as_str())
                .collect::<Vec<_>>(),
            vec!["one pass", "one pass"]
        );
        SOURCE_PASSES.with(|counts| assert_eq!(*counts.borrow(), (1, 1)));
    }
}
