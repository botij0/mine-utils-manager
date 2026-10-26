//! Provenance: which Modrinth project a local file came from.
//!
//! The record lives at `<data-dir>/provenance.json`, keyed by the SHA-256 of
//! the installed file, so a future update check can map a file back to its
//! project and version. Writes are atomic and preserve other entries; a
//! corrupt file is a clean error, never a silent overwrite.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where an installed file came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            ProvenanceError::Corrupt { .. } => "provenance_corrupt",
            ProvenanceError::Io { .. } => "provenance_io",
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
    let bytes = match std::fs::read(&path) {
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
pub fn record(
    data_dir: &Path,
    sha256: &str,
    entry: ProvenanceEntry,
) -> Result<(), ProvenanceError> {
    let mut all = read_all(data_dir)?;
    all.insert(sha256.to_string(), entry);
    let bytes = serde_json::to_vec_pretty(&all).map_err(|err| ProvenanceError::Io {
        reason: err.to_string(),
    })?;
    let path = provenance_file(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ProvenanceError::Io {
            reason: format!("cannot use {}: {err}", parent.display()),
        })?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &bytes).map_err(|err| ProvenanceError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    std::fs::rename(&temp, &path).map_err(|err| ProvenanceError::Io {
        reason: format!("cannot replace {}: {err}", path.display()),
    })?;
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
