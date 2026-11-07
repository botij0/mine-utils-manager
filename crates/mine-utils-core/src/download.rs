//! Verified pack downloads from the Modrinth CDN.
//!
//! A file streams into a unique temp file inside the target directory, never
//! held fully in memory. The SHA-512 from the version metadata is checked
//! while streaming, the zip root (`assets` for resource packs, `data` for
//! datapacks) is validated, and only then may the caller commit the temp
//! into place. The temp is deleted on every failure.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256, Sha512};

use crate::modrinth::{ModrinthClient, Transport, MAX_DOWNLOAD_BYTES};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// What the download verifier reports. Codes are the wire codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// The CDN could not be reached. The UI shows offline.
    Offline { reason: String },
    /// The CDN answered with a non-2xx status after retries.
    Status { status: u16, url: String },
    /// A redirect left the Modrinth hosts. Refused, not retried.
    BadHost { host: String },
    /// More than 500 MB arrived. The temp is deleted, nothing is kept.
    TooLarge { url: String },
    /// The stream failed or the target dir was unusable.
    Io { reason: String },
    /// The bytes do not match the version metadata. Not retried.
    HashMismatch { expected: String, actual: String },
    /// The bytes are not a zip. Not retried.
    NotZip { reason: String },
    /// The zip has no `assets` (resource) or `data` (datapack) root.
    WrongRoot {
        expected: String,
        found: Vec<String>,
    },
    /// The file name is unsafe for the target directory.
    BadFilename { name: String },
    /// The destination already exists; the installer routes this through the
    /// duplicate/collision prompt instead of overwriting.
    Exists { name: String },
}

impl DownloadError {
    pub fn code(&self) -> &'static str {
        match self {
            DownloadError::Offline { .. } => "download_offline",
            DownloadError::Status { .. } => "download_status",
            DownloadError::BadHost { .. } => "download_host",
            DownloadError::TooLarge { .. } => "download_too_large",
            DownloadError::Io { .. } => "download_io",
            DownloadError::HashMismatch { .. } => "download_hash_mismatch",
            DownloadError::NotZip { .. } => "download_not_zip",
            DownloadError::WrongRoot { .. } => "download_wrong_root",
            DownloadError::BadFilename { .. } => "download_filename",
            DownloadError::Exists { .. } => "download_exists",
        }
    }
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Offline { reason } => {
                write!(f, "download failed: CDN unreachable ({reason})")
            }
            DownloadError::Status { status, url } => {
                write!(f, "download failed: status {status} for {url}")
            }
            DownloadError::BadHost { host } => {
                write!(f, "download refused: redirect to unexpected host {host}")
            }
            DownloadError::TooLarge { url } => {
                write!(f, "download refused: {url} exceeds the 500 MB cap")
            }
            DownloadError::Io { reason } => write!(f, "download failed: {reason}"),
            DownloadError::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "download failed: SHA-512 mismatch (expected {expected}, got {actual})"
                )
            }
            DownloadError::NotZip { reason } => {
                write!(f, "download failed: not a zip archive ({reason})")
            }
            DownloadError::WrongRoot { expected, found } => {
                write!(
                    f,
                    "download failed: zip has no `{expected}/` root (saw {})",
                    summarize_roots(found)
                )
            }
            DownloadError::BadFilename { name } => {
                write!(f, "download refused: unsafe file name {name:?}")
            }
            DownloadError::Exists { name } => {
                write!(f, "download stopped: {name} already exists")
            }
        }
    }
}

fn summarize_roots(found: &[String]) -> String {
    if found.is_empty() {
        return "an empty archive".to_string();
    }
    let mut roots: Vec<&str> = found
        .iter()
        .map(|name| name.split('/').next().unwrap_or(name))
        .collect();
    roots.sort();
    roots.dedup();
    roots.truncate(5);
    roots.join(", ")
}

impl std::error::Error for DownloadError {}

impl From<crate::modrinth::ModrinthError> for DownloadError {
    fn from(err: crate::modrinth::ModrinthError) -> Self {
        use crate::modrinth::ModrinthError as E;
        match err {
            E::Offline { reason } => DownloadError::Offline { reason },
            E::Status { status, url } | E::RetryExhausted { status, url } => {
                DownloadError::Status { status, url }
            }
            E::Redirect { host }
            | E::TooManyRedirects { url: host }
            | E::BadUrl { url: host, .. } => DownloadError::BadHost { host },
            E::BodyTooLarge { url } => DownloadError::TooLarge { url },
            E::Parse { reason } => DownloadError::Io { reason },
        }
    }
}

/// Which archive root the downloaded zip must contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipRoot {
    /// Resource packs carry `assets/`.
    Assets,
    /// Datapacks carry `data/`.
    Data,
}

impl ZipRoot {
    fn dir(self) -> &'static str {
        match self {
            ZipRoot::Assets => "assets",
            ZipRoot::Data => "data",
        }
    }
}

/// A verified temp file inside the target directory. Commit it with
/// [`VerifiedDownload::commit`] (or [`commit_verified`]); dropping it
/// without committing deletes the temp, so a failed install never leaves a
/// `.tmp` behind. The temp ends in `.tmp` so pack scans skip it.
#[derive(Debug)]
pub struct VerifiedDownload {
    /// The temp file. Lives in the target dir, so the commit is atomic.
    pub temp_path: PathBuf,
    /// SHA-256 of the downloaded bytes, for the provenance record.
    pub sha256: String,
    /// Bytes written.
    pub bytes: u64,
    committed: bool,
}

impl Drop for VerifiedDownload {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

impl VerifiedDownload {
    /// Atomically move the verified temp to `target_dir/filename` without
    /// replacing an existing file. Marks the download committed so `Drop`
    /// no longer deletes it.
    pub fn commit(mut self, target_dir: &Path, filename: &str) -> Result<PathBuf, DownloadError> {
        let dest = commit_path(&self.temp_path, target_dir, filename)?;
        self.committed = true;
        Ok(dest)
    }
}

/// The file name must be a plain name: no separators, no escapes.
pub fn validate_filename(name: &str) -> Result<(), DownloadError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(DownloadError::BadFilename {
            name: name.to_string(),
        });
    }
    Ok(())
}

fn temp_path_for(target_dir: &Path, filename: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    target_dir.join(format!(
        ".{filename}.mine-utils-download-{pid}-{stamp}-{id}.tmp",
        pid = std::process::id()
    ))
}

/// Stream a CDN file into a temp file in `target_dir`, checking the 500 MB
/// cap and the SHA-512 while streaming, then validating the zip root.
/// The URL must be `https://cdn.modrinth.com/...` and every redirect hop
/// must stay on the Modrinth hosts. The temp is deleted on every error.
pub fn download_verified<T: Transport>(
    client: &ModrinthClient<T>,
    url: &str,
    target_dir: &Path,
    filename: &str,
    expected_sha512: &str,
    root: ZipRoot,
) -> Result<VerifiedDownload, DownloadError> {
    download_verified_capped(
        client,
        url,
        target_dir,
        filename,
        expected_sha512,
        root,
        MAX_DOWNLOAD_BYTES,
    )
}

/// [`download_verified`] with an explicit byte cap. Tests use a small cap to
/// prove the limit without staging 500 MB; production passes
/// [`MAX_DOWNLOAD_BYTES`].
pub fn download_verified_capped<T: Transport>(
    client: &ModrinthClient<T>,
    url: &str,
    target_dir: &Path,
    filename: &str,
    expected_sha512: &str,
    root: ZipRoot,
    max_bytes: u64,
) -> Result<VerifiedDownload, DownloadError> {
    validate_filename(filename)?;
    std::fs::create_dir_all(target_dir).map_err(|err| DownloadError::Io {
        reason: format!("cannot use {}: {err}", target_dir.display()),
    })?;
    let temp = temp_path_for(target_dir, filename);
    let result = download_into(client, url, &temp, expected_sha512, root, max_bytes);
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map(|(sha256, bytes)| VerifiedDownload {
        temp_path: temp,
        sha256,
        bytes,
        committed: false,
    })
}

fn download_into<T: Transport>(
    client: &ModrinthClient<T>,
    url: &str,
    temp: &Path,
    expected_sha512: &str,
    root: ZipRoot,
    max_bytes: u64,
) -> Result<(String, u64), DownloadError> {
    let fetched = client.file_stream(url)?;
    let mut out = std::fs::File::create(temp).map_err(|err| DownloadError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    let mut sha512 = Sha512::new();
    let mut sha256 = Sha256::new();
    let mut bytes: u64 = 0;
    let mut reader = fetched.body.take(max_bytes + 1);
    // Same 64KiB size as the pack/install hash loops, but not shared: this
    // loop hashes two digests while writing and enforcing the byte cap.
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|err| DownloadError::Io {
            reason: format!("download interrupted: {err}"),
        })?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > max_bytes {
            return Err(DownloadError::TooLarge {
                url: fetched.url.clone(),
            });
        }
        sha512.update(&chunk[..n]);
        sha256.update(&chunk[..n]);
        out.write_all(&chunk[..n])
            .map_err(|err| DownloadError::Io {
                reason: format!("cannot write {}: {err}", temp.display()),
            })?;
    }
    out.flush().map_err(|err| DownloadError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    // Fsync the temp before the atomic rename so a crash cannot leave a
    // zero/partial destination that no later check re-hashes. The parent-dir
    // sync in `commit_path` then makes the rename itself durable.
    out.sync_all().map_err(|err| DownloadError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    drop(out);
    let actual_sha512 = hex::encode(sha512.finalize());
    if !constant_time_eq_hex(&actual_sha512, expected_sha512) {
        return Err(DownloadError::HashMismatch {
            expected: expected_sha512.to_ascii_lowercase(),
            actual: actual_sha512,
        });
    }
    validate_zip_root(temp, root)?;
    Ok((hex::encode(sha256.finalize()), bytes))
}

fn constant_time_eq_hex(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in actual.bytes().zip(expected.to_ascii_lowercase().bytes()) {
        diff |= a ^ b;
    }
    diff == 0 && !actual.is_empty()
}

/// The temp must be a zip containing the expected root. Not retried. Only
/// a bounded number of entry names are kept for the error summary, so a
/// hostile archive with millions of entries cannot blow up the message.
fn validate_zip_root(temp: &Path, root: ZipRoot) -> Result<(), DownloadError> {
    const MAX_LISTED_ENTRIES: usize = 64;
    let file = std::fs::File::open(temp).map_err(|err| DownloadError::Io {
        reason: err.to_string(),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| DownloadError::NotZip {
        reason: err.to_string(),
    })?;
    if archive.is_empty() {
        return Err(DownloadError::WrongRoot {
            expected: root.dir().to_string(),
            found: Vec::new(),
        });
    }
    let want = format!("{}/", root.dir());
    let mut found = Vec::new();
    let mut truncated = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| DownloadError::NotZip {
                reason: err.to_string(),
            })?;
        let name = entry.name().to_string();
        if found.len() < MAX_LISTED_ENTRIES {
            found.push(name.clone());
        } else {
            truncated = true;
        }
        if name == root.dir() || name.starts_with(&want) {
            return Ok(());
        }
    }
    if truncated {
        found.push("…".to_string());
    }
    Err(DownloadError::WrongRoot {
        expected: root.dir().to_string(),
        found,
    })
}

fn commit_path(temp: &Path, target_dir: &Path, filename: &str) -> Result<PathBuf, DownloadError> {
    validate_filename(filename)?;
    let dest = target_dir.join(filename);
    atomic_no_replace(temp, &dest).map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            DownloadError::Exists {
                name: filename.to_string(),
            }
        } else {
            DownloadError::Io {
                reason: format!("cannot move download into place: {err}"),
            }
        }
    })?;
    sync_parent(&dest);
    Ok(dest)
}

/// Move `temp` to `dest` without replacing an existing file. On Linux the
/// rename is a single `renameat2` with `RENAME_NOREPLACE`, so the
/// exists-then-rename race cannot happen; elsewhere a hard link (same
/// directory, so same filesystem) provides the atomic no-replace, with an
/// exists-checked rename as the last resort.
fn atomic_no_replace(temp: &Path, dest: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // renameat2 with AT_FDCWD renames by path. RENAME_NOREPLACE makes
        // an existing destination fail with EEXIST instead of replacing it.
        const RENAME_NOREPLACE: libc::c_uint = 1;
        let temp_c = std::ffi::CString::new(temp.as_os_str().as_encoded_bytes());
        let dest_c = std::ffi::CString::new(dest.as_os_str().as_encoded_bytes());
        if let (Ok(temp_c), Ok(dest_c)) = (temp_c, dest_c) {
            // SAFETY: plain integers and NUL-terminated path pointers; no
            // memory is retained by the kernel after the call returns.
            let rc = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    temp_c.as_ptr(),
                    libc::AT_FDCWD,
                    dest_c.as_ptr(),
                    RENAME_NOREPLACE,
                )
            };
            if rc == 0 {
                return Ok(());
            }
            let err = std::io::Error::last_os_error();
            if !rename_noreplace_unsupported(&err) {
                return Err(err);
            }
        }
    }
    match std::fs::hard_link(temp, dest) {
        Ok(()) => std::fs::remove_file(temp),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(err),
        Err(_) => {
            if dest.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "destination exists",
                ));
            }
            std::fs::rename(temp, dest)
        }
    }
}

/// Whether a failed `renameat2(RENAME_NOREPLACE)` means the kernel or
/// filesystem cannot do it here (seccomp, exotic fs), so the commit falls
/// through to the hard-link path rather than failing the install.
#[cfg(target_os = "linux")]
fn rename_noreplace_unsupported(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::PermissionDenied || err.raw_os_error() == Some(libc::ENOSYS)
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(handle) = std::fs::File::open(parent) {
            let _ = handle.sync_all();
        }
    }
}

/// Atomically rename a verified temp into `target_dir/filename`. Refuses to
/// overwrite: an existing name routes through the duplicate prompt instead.
pub fn commit_verified(
    temp: &Path,
    target_dir: &Path,
    filename: &str,
) -> Result<PathBuf, DownloadError> {
    commit_path(temp, target_dir, filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_filenames() {
        for bad in ["", ".", "..", "a/b.zip", "a\\b.zip", "a\0b.zip"] {
            assert!(validate_filename(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(validate_filename("Fresh Animations v1.9.zip").is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn renameat2_errors_that_mean_unsupported_take_the_link_fallback() {
        let decisions: Vec<(&str, bool)> = [
            ("EINVAL", libc::EINVAL),
            ("ENOSYS", libc::ENOSYS),
            ("EPERM", libc::EPERM),
            ("EACCES", libc::EACCES),
            ("EEXIST", libc::EEXIST),
        ]
        .into_iter()
        .map(|(name, errno)| {
            let err = std::io::Error::from_raw_os_error(errno);
            (name, rename_noreplace_unsupported(&err))
        })
        .collect();
        assert_eq!(
            decisions,
            vec![
                ("EINVAL", true),
                ("ENOSYS", true),
                ("EPERM", true),
                ("EACCES", true),
                ("EEXIST", false),
            ]
        );
    }
}
