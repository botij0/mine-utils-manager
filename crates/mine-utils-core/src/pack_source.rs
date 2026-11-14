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
    Zip(&'a Path),
    Dir(&'a Path),
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
            PackLocation::Zip => Self::Zip(path),
            PackLocation::Directory => Self::Dir(path),
        })
    }

    pub(crate) fn entry_names(&self, bytes_read: &mut u64) -> Result<Vec<String>, PackError> {
        match self {
            Self::Zip(path) => {
                let archive = open_zip_capped(path, &display_name(path), MAX_ZIP_ENTRIES)?;
                let names: Vec<_> = archive
                    .file_names()
                    .map(|name| normalize_entry_name(name).to_string())
                    .collect();
                *bytes_read += names
                    .iter()
                    .map(|name| CENTRAL_DIR_ENTRY_BYTES + name.len() as u64)
                    .sum::<u64>()
                    + CENTRAL_DIR_END_BYTES;
                Ok(names)
            }
            Self::Dir(path) => {
                reject_symlinks(path).map_err(|err| pack_error(path, err))?;
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
            Self::Zip(path) => {
                let pack_name = display_name(path);
                let mut archive = open_zip_capped(path, &pack_name, MAX_ZIP_ENTRIES)?;
                let entry = archive
                    .file_names()
                    .find(|entry| normalize_entry_name(entry) == name)
                    .map(str::to_string);
                let Some(entry) = entry else { return Ok(None) };
                let file = archive
                    .by_name(&entry)
                    .map_err(|err| broken(&pack_name, err))?;
                read_limited(file, name, &pack_name, cap, bytes_read).map(Some)
            }
            Self::Dir(path) => {
                reject_symlinks(path).map_err(|err| pack_error(path, err))?;
                let target = path.join(name);
                let metadata = match std::fs::symlink_metadata(&target) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(err) => return Err(broken(&display_name(path), err)),
                };
                if !metadata.is_file() {
                    return Ok(None);
                }
                let file = File::open(target).map_err(|err| broken(&display_name(path), err))?;
                read_limited(file, name, &display_name(path), cap, bytes_read).map(Some)
            }
        }
    }

    pub(crate) fn fingerprint(&self) -> Result<Fingerprint, AppError> {
        match self {
            Self::Zip(path) => {
                let metadata = std::fs::metadata(path)?;
                Ok(Fingerprint::Zip(metadata.len(), metadata.modified().ok()))
            }
            Self::Dir(path) => {
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
            Self::Zip(path) => hash_file(path),
            Self::Dir(path) => hash_dir(path),
        }
    }
}

fn read_limited(
    reader: impl Read,
    entry: &str,
    pack_name: &str,
    cap: u64,
    bytes_read: &mut u64,
) -> Result<Vec<u8>, PackError> {
    let mut bytes = Vec::new();
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
    name.trim_start_matches("./")
}

pub(crate) fn reject_symlinks(root: &Path) -> Result<(), AppError> {
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
        let mut file = File::open(root.join(&relative))?;
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
