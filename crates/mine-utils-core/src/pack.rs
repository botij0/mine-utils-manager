use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A pack format number. Minecraft writes integers and, since 1.21.9, dotted
/// numbers such as 75.0 and 94.1. The minor part is always present so the two
/// spellings compare and sort the same way. A `max_format` written as `94` or
/// `[94]` accepts every minor of that major; the game stores that as the minor
/// `ANY_MINOR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct FormatVersion {
    pub major: u32,
    pub minor: u32,
}

impl FormatVersion {
    pub const ANY_MINOR: u32 = 0x7fff_ffff;

    pub fn new(major: u32, minor: u32) -> Self {
        FormatVersion { major, minor }
    }

    pub fn from_major(major: u32) -> Self {
        FormatVersion { major, minor: 0 }
    }

    /// The upper bound that accepts every minor of `major`.
    pub fn any_minor_of(major: u32) -> Self {
        FormatVersion {
            major,
            minor: Self::ANY_MINOR,
        }
    }

    /// The version as pack.mcmeta wrote it: an any-minor max reads back as
    /// its whole major.
    pub fn declared(self) -> Self {
        if self.minor == Self::ANY_MINOR {
            Self::from_major(self.major)
        } else {
            self
        }
    }

    /// "34", "75", or "94.1". A zero or any-minor minor stays hidden.
    pub fn label(&self) -> String {
        let FormatVersion { major, minor } = self.declared();
        if minor == 0 {
            major.to_string()
        } else {
            format!("{major}.{minor}")
        }
    }
}

impl fmt::Display for FormatVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// An inclusive range of pack formats. Either end may be open: a lone
/// `min_format` leaves `max` empty and a lone `max_format` leaves `min` empty,
/// so an open declaration is shown as open instead of being dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct FormatRange {
    pub min: Option<FormatVersion>,
    pub max: Option<FormatVersion>,
}

impl FormatRange {
    pub fn single(version: FormatVersion) -> Self {
        FormatRange {
            min: Some(version),
            max: Some(version),
        }
    }

    /// A closed range, or `None` when the bounds are reversed and the range
    /// therefore declares nothing.
    pub fn closed(min: FormatVersion, max: FormatVersion) -> Option<Self> {
        (min <= max).then_some(FormatRange {
            min: Some(min),
            max: Some(max),
        })
    }

    pub fn label(&self) -> String {
        match (self.min, self.max) {
            (None, None) => "unknown".to_string(),
            (Some(min), None) => format!("{} and up", min.label()),
            (None, Some(max)) => format!("up to {}", max.label()),
            (Some(min), Some(max)) => {
                let (min, max) = (min.label(), max.label());
                if min == max {
                    min
                } else {
                    format!("{min} to {max}")
                }
            }
        }
    }
}

/// The roots a pack exposes. A resource pack has `assets`, a data pack has
/// `data`, and a modded pack may carry both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub enum PackLocation {
    Zip,
    Directory,
}

/// The `pack` object of pack.mcmeta after normalization.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct PackMeta {
    pub pack_format: Option<FormatVersion>,
    pub min_format: Option<FormatVersion>,
    pub max_format: Option<FormatVersion>,
    pub supported_formats: Option<FormatRange>,
    pub description: String,
}

impl PackMeta {
    /// Precedence is min/max, then supported_formats, then pack_format. A lone
    /// min or max stays open instead of being dropped. A reversed range is
    /// unknown instead of falling back to a narrower pack_format number.
    pub fn effective_range(&self) -> Option<FormatRange> {
        match (self.min_format, self.max_format) {
            (Some(min), Some(max)) => return FormatRange::closed(min, max),
            (Some(min), None) => {
                return Some(FormatRange {
                    min: Some(min),
                    max: None,
                })
            }
            (None, Some(max)) => {
                return Some(FormatRange {
                    min: None,
                    max: Some(max),
                })
            }
            (None, None) => {}
        }
        if let Some(range) = self.supported_formats {
            return match (range.min, range.max) {
                (Some(min), Some(max)) => FormatRange::closed(min, max),
                (Some(min), None) => Some(FormatRange {
                    min: Some(min),
                    max: None,
                }),
                (None, Some(max)) => Some(FormatRange {
                    min: None,
                    max: Some(max),
                }),
                (None, None) => None,
            };
        }
        self.pack_format.map(FormatRange::single)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct PackIcon {
    pub sha256: String,
    pub path: PathBuf,
}

/// A broken pack still shows up in a listing, with `error` set and the fields
/// the scan managed to read. One bad archive never hides the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct PackIssue {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "wire-types", ts(optional))]
    pub symlink_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub location: PackLocation,
    pub kind: PackKind,
    pub description: String,
    pub meta: PackMeta,
    /// The lower end of the declared range, or the upper end when the range has
    /// none. Taken from `PackMeta::effective_range`, which reads `min_format` and
    /// `max_format`, then `supported_formats`, then `pack_format`. `format` never
    /// carries the any-minor sentinel: a bare-major upper end reads back as
    /// `major.0`.
    pub format: Option<FormatVersion>,
    /// The full declared range that `format` summarizes. A bare-major upper end,
    /// from `max_format` or `supported_formats`, means any minor of that major:
    /// its `minor` is `FormatVersion::ANY_MINOR` (2147483647 on the wire).
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
    #[error("{name} has a non-UTF-8 filename and cannot be managed")]
    NonUtf8Name { name: String },
    #[error("{name} has no assets or data directory")]
    UnknownKind { name: String },
    #[error("{name} is not a readable pack: {reason}")]
    BrokenArchive { name: String, reason: String },
    #[error("{name} is not an inactive leftover Replace entry in this folder")]
    InvalidHolding { name: String },
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
        self.error_code().as_str()
    }

    pub fn error_code(&self) -> crate::ErrorCode {
        match self {
            PackError::NonUtf8Name { .. } => crate::ErrorCode::PackNonUtf8Name,
            PackError::UnknownKind { .. } => crate::ErrorCode::PackUnknownKind,
            PackError::BrokenArchive { .. } | PackError::InvalidHolding { .. } => {
                crate::ErrorCode::PackBrokenArchive
            }
            PackError::MissingMeta { .. } => crate::ErrorCode::PackMissingMeta,
            PackError::Symlink { .. } => crate::ErrorCode::PackSymlink,
            PackError::Duplicate { .. } => crate::ErrorCode::PackDuplicate,
            PackError::NameCollision { .. } => crate::ErrorCode::PackNameCollision,
            PackError::NotFound { .. } => crate::ErrorCode::PackNotFound,
            PackError::OutsideRoot { .. } => crate::ErrorCode::PackOutsideRoot,
            PackError::Trash { .. } => crate::ErrorCode::PackTrash,
        }
    }
}

/// The answer to an import attempt. The frontend turns `duplicate` and
/// `collision` into a Skip/Replace choice and retries with that choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub enum ImportOutcome {
    Imported {
        pack: Box<Pack>,
    },
    Duplicate {
        name: String,
        sha256: String,
        existing: String,
    },
    Collision {
        name: String,
    },
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub enum ImportChoice {
    #[default]
    Default,
    Replace,
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(min: u32, max: u32) -> FormatRange {
        FormatRange::closed(
            FormatVersion::from_major(min),
            FormatVersion::from_major(max),
        )
        .unwrap()
    }

    #[test]
    fn format_version_labels() {
        assert_eq!(FormatVersion::from_major(34).label(), "34");
        assert_eq!(FormatVersion::new(69, 0).label(), "69");
        assert_eq!(FormatVersion::new(94, 1).label(), "94.1");
        assert_eq!(closed(34, 42).label(), "34 to 42");
        assert_eq!(
            FormatRange::single(FormatVersion::from_major(34)).label(),
            "34"
        );
    }

    #[test]
    fn open_ranges_label_the_open_end() {
        assert_eq!(
            FormatRange {
                min: Some(FormatVersion::from_major(48)),
                max: None,
            }
            .label(),
            "48 and up"
        );
        assert_eq!(
            FormatRange {
                min: None,
                max: Some(FormatVersion::from_major(42)),
            }
            .label(),
            "up to 42"
        );
        assert_eq!(
            FormatRange {
                min: None,
                max: None,
            }
            .label(),
            "unknown"
        );
    }

    #[test]
    fn effective_range_prefers_min_max() {
        let meta = PackMeta {
            pack_format: Some(FormatVersion::from_major(34)),
            min_format: Some(FormatVersion::from_major(48)),
            max_format: Some(FormatVersion::from_major(70)),
            supported_formats: Some(closed(20, 30)),
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
    fn a_lone_min_or_max_is_not_dropped() {
        let meta = PackMeta {
            pack_format: Some(FormatVersion::from_major(34)),
            min_format: Some(FormatVersion::from_major(48)),
            max_format: None,
            supported_formats: None,
            description: String::new(),
        };
        assert_eq!(meta.effective_range().unwrap().label(), "48 and up");

        let meta = PackMeta {
            min_format: None,
            max_format: Some(FormatVersion::from_major(42)),
            ..meta
        };
        assert_eq!(meta.effective_range().unwrap().label(), "up to 42");
    }

    #[test]
    fn a_reversed_range_is_unknown() {
        let meta = PackMeta {
            pack_format: Some(FormatVersion::from_major(34)),
            min_format: Some(FormatVersion::from_major(50)),
            max_format: Some(FormatVersion::from_major(10)),
            supported_formats: Some(closed(20, 30)),
            description: String::new(),
        };
        assert_eq!(meta.effective_range(), None, "reversed min/max is unknown");

        let meta = PackMeta {
            min_format: None,
            max_format: None,
            supported_formats: Some(FormatRange {
                min: Some(FormatVersion::from_major(50)),
                max: Some(FormatVersion::from_major(10)),
            }),
            ..meta
        };
        assert_eq!(
            meta.effective_range(),
            None,
            "a reversed supported_formats stays unknown, no pack_format fallback"
        );

        assert_eq!(
            FormatRange::closed(FormatVersion::from_major(10), FormatVersion::from_major(50)),
            Some(closed(10, 50))
        );
        assert_eq!(
            FormatRange::closed(FormatVersion::from_major(50), FormatVersion::from_major(10)),
            None
        );
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
