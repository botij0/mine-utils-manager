use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A pack format number. Minecraft writes integers and, since 1.21.9, dotted
/// numbers such as 75.0 and 94.1. The minor part is always present so the two
/// spellings compare and sort the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FormatVersion {
    pub major: u32,
    pub minor: u32,
}

impl FormatVersion {
    pub fn new(major: u32, minor: u32) -> Self {
        FormatVersion { major, minor }
    }

    pub fn from_major(major: u32) -> Self {
        FormatVersion { major, minor: 0 }
    }

    /// "34", "75", or "94.1". A zero minor stays hidden.
    pub fn label(&self) -> String {
        if self.minor == 0 {
            self.major.to_string()
        } else {
            format!("{}.{}", self.major, self.minor)
        }
    }
}

impl fmt::Display for FormatVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// An inclusive range of pack formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatRange {
    pub min: FormatVersion,
    pub max: FormatVersion,
}

impl FormatRange {
    pub fn single(version: FormatVersion) -> Self {
        FormatRange {
            min: version,
            max: version,
        }
    }

    pub fn label(&self) -> String {
        if self.min == self.max {
            self.min.label()
        } else {
            format!("{} to {}", self.min.label(), self.max.label())
        }
    }
}

/// The roots a pack exposes. A resource pack has `assets`, a data pack has
/// `data`, and a modded pack may carry both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    Resource,
    Data,
    Both,
    Unknown,
}

impl PackKind {
    pub fn label(self) -> &'static str {
        match self {
            PackKind::Resource => "Resource",
            PackKind::Data => "Data",
            PackKind::Both => "Both",
            PackKind::Unknown => "Unknown",
        }
    }
}

/// How the pack is stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackLocation {
    Zip,
    Directory,
}

/// The `pack` object of pack.mcmeta after normalization.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PackMeta {
    pub pack_format: Option<FormatVersion>,
    pub min_format: Option<FormatVersion>,
    pub max_format: Option<FormatVersion>,
    pub supported_formats: Option<FormatRange>,
    pub description: String,
}

impl PackMeta {
    /// Precedence is min/max, then supported_formats, then pack_format.
    pub fn effective_range(&self) -> Option<FormatRange> {
        match (self.min_format, self.max_format) {
            (Some(min), Some(max)) => Some(FormatRange { min, max }),
            _ => self
                .supported_formats
                .or_else(|| self.pack_format.map(FormatRange::single)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIcon {
    pub sha256: String,
    pub path: PathBuf,
}

/// A broken pack still shows up in a listing, with `error` set and the fields
/// the scan managed to read. One bad archive never hides the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub location: PackLocation,
    pub kind: PackKind,
    pub description: String,
    pub meta: PackMeta,
    pub format: Option<FormatVersion>,
    pub format_range: Option<FormatRange>,
    pub game_label: Option<String>,
    pub icon: Option<PackIcon>,
    pub error: Option<PackIssue>,
    pub bytes_read: u64,
}

impl Pack {
    pub fn issue(&self) -> Option<&PackIssue> {
        self.error.as_ref()
    }
}

/// Everything that can go wrong while reading or importing one pack.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackError {
    #[error("{name} has no assets or data directory")]
    UnknownKind { name: String },
    #[error("{name} is not a readable pack: {reason}")]
    BrokenArchive { name: String, reason: String },
    #[error("{name} is missing pack.mcmeta")]
    MissingMeta { name: String },
    #[error("{path} is a symlink; pack sources must be real files and dirs")]
    Symlink { path: PathBuf },
    #[error("{name} is already installed as {existing}")]
    Duplicate {
        name: String,
        sha256: String,
        existing: String,
    },
    #[error("{name} already exists in resourcepacks")]
    NameCollision { name: String },
    #[error("{path} does not exist")]
    NotFound { path: PathBuf },
    #[error("{path} is not a direct child of resourcepacks")]
    OutsideRoot { path: PathBuf },
    #[error("cannot move {path} to the trash: {reason}")]
    Trash { path: PathBuf, reason: String },
}

impl PackError {
    pub fn code(&self) -> &'static str {
        match self {
            PackError::UnknownKind { .. } => "pack_unknown_kind",
            PackError::BrokenArchive { .. } => "pack_broken_archive",
            PackError::MissingMeta { .. } => "pack_missing_meta",
            PackError::Symlink { .. } => "pack_symlink",
            PackError::Duplicate { .. } => "pack_duplicate",
            PackError::NameCollision { .. } => "pack_name_collision",
            PackError::NotFound { .. } => "pack_not_found",
            PackError::OutsideRoot { .. } => "pack_outside_root",
            PackError::Trash { .. } => "pack_trash",
        }
    }
}

/// The answer to an import attempt. The frontend turns `duplicate` and
/// `collision` into a Skip/Replace choice and retries with that choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImportOutcome {
    Imported { pack: Pack },
    Duplicate {
        name: String,
        sha256: String,
        existing: String,
    },
    Collision { name: String },
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportChoice {
    #[default]
    Default,
    Replace,
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_labels() {
        assert_eq!(FormatVersion::from_major(34).label(), "34");
        assert_eq!(FormatVersion::new(69, 0).label(), "69");
        assert_eq!(FormatVersion::new(94, 1).label(), "94.1");
        assert_eq!(
            FormatRange {
                min: FormatVersion::from_major(34),
                max: FormatVersion::from_major(42),
            }
            .label(),
            "34 to 42"
        );
    }

    #[test]
    fn effective_range_prefers_min_max() {
        let meta = PackMeta {
            pack_format: Some(FormatVersion::from_major(34)),
            min_format: Some(FormatVersion::from_major(48)),
            max_format: Some(FormatVersion::from_major(70)),
            supported_formats: Some(FormatRange {
                min: FormatVersion::from_major(20),
                max: FormatVersion::from_major(30),
            }),
            description: String::new(),
        };
        assert_eq!(meta.effective_range().unwrap().label(), "48 to 70");

        let meta = PackMeta {
            min_format: None,
            max_format: None,
            ..meta
        };
        assert_eq!(meta.effective_range().unwrap().label(), "20 to 30");

        let meta = PackMeta {
            supported_formats: None,
            ..meta
        };
        assert_eq!(meta.effective_range().unwrap().label(), "34");
    }

    #[test]
    fn import_outcome_wire_shape() {
        let json = serde_json::to_string(&ImportOutcome::Skipped).unwrap();
        assert_eq!(json, r#"{"status":"skipped"}"#);
        let json = serde_json::to_string(&ImportOutcome::Collision {
            name: "pack.zip".to_string(),
        })
        .unwrap();
        assert_eq!(json, r#"{"status":"collision","name":"pack.zip"}"#);
    }
}
