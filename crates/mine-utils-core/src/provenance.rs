//! Provenance: which Modrinth project a local file came from.
//!
//! The record lives at `<data-dir>/provenance.json`, keyed by the SHA-256 of
//! the installed file, so a future update check can map a file back to its
//! project and version. Writes are atomic and preserve other entries; a
//! corrupt file is a clean error, never a silent overwrite.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Serializes concurrent installs in this process: the record is a global
/// read-modify-write, so two installs in different game dirs must not
/// interleave.
static PROVENANCE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const MAX_PROVENANCE_BYTES: u64 = 16 * 1024 * 1024;

static PROVENANCE_TEMPS: AtomicU64 = AtomicU64::new(0);

/// Where an installed file came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "wire-types", ts(rename = "ModrinthProvenanceEntry"))]
pub struct ProvenanceEntry {
    pub project_id: String,
    pub version_id: String,
    pub file_name: String,
}

/// What the provenance store reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceError {
    /// The JSON exists but cannot be read. Delete or repair it by hand; the
    /// app never overwrites it silently.
    Corrupt { reason: String },
    /// The store could not be read or written.
    Io { reason: String },
}

impl ProvenanceError {
    pub fn code(&self) -> &'static str {
        self.error_code().as_str()
    }

    pub fn error_code(&self) -> crate::ErrorCode {
        match self {
            ProvenanceError::Corrupt { .. } => crate::ErrorCode::ProvenanceCorrupt,
            ProvenanceError::Io { .. } => crate::ErrorCode::ProvenanceIo,
        }
    }
}

impl std::fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProvenanceError::Corrupt { reason } => {
                write!(
                    f,
                    "provenance record is corrupt ({reason}); not overwritten"
                )
            }
            ProvenanceError::Io { reason } => {
                write!(f, "provenance record unavailable: {reason}")
            }
        }
    }
}

impl std::error::Error for ProvenanceError {}

/// The record file inside the app data dir.
pub fn provenance_file(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("provenance.json")
}

fn read_all(data_dir: &Path) -> Result<BTreeMap<String, ProvenanceEntry>, ProvenanceError> {
    let path = provenance_file(data_dir);
    let bytes = match crate::read_capped::read_capped_follow(&path, MAX_PROVENANCE_BYTES) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => {
            return Err(ProvenanceError::Io {
                reason: format!("cannot read {}: {err}", path.display()),
            });
        }
    };
    serde_json::from_slice(&bytes).map_err(|err| ProvenanceError::Corrupt {
        reason: err.to_string(),
    })
}

/// Look up where the file with this SHA-256 came from, if recorded.
pub fn lookup(data_dir: &Path, sha256: &str) -> Result<Option<ProvenanceEntry>, ProvenanceError> {
    Ok(read_all(data_dir)?.remove(sha256))
}

/// Record where an installed file came from. Other entries are preserved.
/// Concurrent installs in this process are serialized; the temp name is
/// unique per record and both the file and its parent dir are fsynced, so a
/// crash cannot leave a half-written record behind.
pub fn record(
    data_dir: &Path,
    sha256: &str,
    entry: ProvenanceEntry,
) -> Result<(), ProvenanceError> {
    let _guard = PROVENANCE_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let mut all = read_all(data_dir)?;
    all.insert(sha256.to_string(), entry);
    let bytes = serde_json::to_vec_pretty(&all).map_err(|err| ProvenanceError::Io {
        reason: err.to_string(),
    })?;
    if bytes.len() as u64 > MAX_PROVENANCE_BYTES {
        return Err(ProvenanceError::Io {
            reason: "provenance record is too large".into(),
        });
    }
    let path = provenance_file(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ProvenanceError::Io {
            reason: format!("cannot use {}: {err}", parent.display()),
        })?;
    }
    let id = PROVENANCE_TEMPS.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp = path.with_extension(format!("json.{}.{}.{}.tmp", std::process::id(), stamp, id));
    std::fs::write(&temp, &bytes).map_err(|err| ProvenanceError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    // fsync the file before it becomes visible under the real name.
    if let Ok(handle) = crate::read_capped::open_regular(&temp) {
        let _ = handle.sync_all();
    }
    if let Err(err) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(ProvenanceError::Io {
            reason: format!("cannot replace {}: {err}", path.display()),
        });
    }
    if let Some(parent) = path.parent() {
        if let Ok(handle) = std::fs::File::open(parent) {
            let _ = handle.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_preserves_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        record(
            dir.path(),
            "aaa",
            ProvenanceEntry {
                project_id: "p1".into(),
                version_id: "v1".into(),
                file_name: "a.zip".into(),
            },
        )
        .expect("record");
        record(
            dir.path(),
            "bbb",
            ProvenanceEntry {
                project_id: "p2".into(),
                version_id: "v2".into(),
                file_name: "b.zip".into(),
            },
        )
        .expect("record");
        let first = lookup(dir.path(), "aaa").expect("lookup");
        assert_eq!(first.map(|e| e.project_id), Some("p1".into()));
        assert!(lookup(dir.path(), "zzz").expect("lookup").is_none());
    }

    #[test]
    fn concurrent_records_do_not_lose_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    record(
                        &path,
                        &format!("key-{i}"),
                        ProvenanceEntry {
                            project_id: format!("p{i}"),
                            version_id: format!("v{i}"),
                            file_name: format!("{i}.zip"),
                        },
                    )
                    .expect("record");
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread");
        }
        for i in 0..8 {
            let entry = lookup(dir.path(), &format!("key-{i}"))
                .expect("lookup")
                .expect("entry survives concurrent records");
            assert_eq!(entry.project_id, format!("p{i}"));
        }
    }

    #[test]
    fn corrupt_file_is_an_error_not_an_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(provenance_file(dir.path()), b"{not json").expect("write");
        let err = lookup(dir.path(), "aaa").expect_err("corrupt must fail");
        assert_eq!(err.code(), "provenance_corrupt");
        assert!(record(
            dir.path(),
            "aaa",
            ProvenanceEntry {
                project_id: "p".into(),
                version_id: "v".into(),
                file_name: "a.zip".into(),
            }
        )
        .is_err());
    }
}
