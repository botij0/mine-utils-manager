//! Resolve a world version name such as `1.21.10` to the data pack format the
//! world's datapacks must declare. The mapping comes from the `data` rows of
//! `data/pack_formats.json`, whose labels cover `1.13` through `1.21.11`.
//!
//! Labels name one version (`1.21.4`) or a span (`1.21.9 to 1.21.10`,
//! `1.19 to 1.19.3`). Spans expand over the named endpoints: two-part spans
//! cover every patch between the endpoints, so `1.19 to 1.19.3` means 1.19,
//! 1.19.1, 1.19.2, and 1.19.3.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::format_table::FormatTable;
use crate::pack::{FormatVersion, PackKind};

/// A world version name and the data format its datapacks target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionFormat {
    pub version: String,
    pub format: FormatVersion,
}

fn table_versions(table: &FormatTable) -> Vec<VersionFormat> {
    // The data rows are the source of truth; the labels are parsed, not the
    // raw JSON, so a future table edit flows through automatically.
    let mut out = Vec::new();
    let mut seen = HashMap::new();
    for format in data_formats(table) {
        let Some(label) = table.label_for(PackKind::Data, format) else {
            continue;
        };
        for version in expand_label(label) {
            // The first row naming a version wins; rows are disjoint.
            if seen.insert(version.clone(), format).is_none() {
                out.push(VersionFormat { version, format });
            }
        }
    }
    out
}

fn data_formats(table: &FormatTable) -> Vec<FormatVersion> {
    // Probe the known data format space: single digits from the early eras
    // plus every format the embedded table is known to carry.
    let mut formats: Vec<FormatVersion> = (1..=30).map(FormatVersion::from_major).collect();
    for major in [32, 41, 48, 55, 57, 61, 71, 80, 81, 88] {
        formats.push(FormatVersion::from_major(major));
    }
    formats.push(FormatVersion::new(94, 1));
    formats
        .into_iter()
        .filter(|format| table.label_for(PackKind::Data, *format).is_some())
        .collect()
}

fn expand_label(label: &str) -> Vec<String> {
    let (start, end) = match label.split_once(" to ") {
        Some((start, end)) => (start.trim(), end.trim()),
        None => return vec![label.trim().to_string()],
    };
    let Some((major, minor_start, minor_end)) = span_parts(start, end) else {
        return vec![label.trim().to_string()];
    };
    (minor_start..=minor_end)
        .map(|minor| {
            if minor == 0 {
                format!("1.{major}")
            } else {
                format!("1.{major}.{minor}")
            }
        })
        .collect()
}

fn span_parts(start: &str, end: &str) -> Option<(u32, u32, u32)> {
    let (major_start, minor_start) = version_parts(start)?;
    let (major_end, minor_end) = version_parts(end)?;
    if major_start != major_end || minor_start > minor_end {
        return None;
    }
    Some((major_start, minor_start, minor_end))
}

fn version_parts(text: &str) -> Option<(u32, u32)> {
    // Minecraft versions are `1.<series>[.<patch>]`: `1.21.10` is series 21,
    // patch 10, and a bare `1.21` is series 21, patch 0.
    let rest = text.strip_prefix("1.")?;
    let mut parts = rest.split('.');
    let series: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = match parts.next() {
        Some(patch) => patch.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((series, patch))
}

fn index() -> &'static HashMap<String, FormatVersion> {
    static INDEX: OnceLock<HashMap<String, FormatVersion>> = OnceLock::new();
    INDEX.get_or_init(|| {
        table_versions(FormatTable::embedded())
            .into_iter()
            .map(|entry| (entry.version, entry.format))
            .collect()
    })
}

/// The data format for a world version name such as `1.21.10`, or `None` when
/// the table does not cover it.
pub fn data_format_for_version(version: &str) -> Option<FormatVersion> {
    index().get(version.trim()).copied()
}

/// True when the pack's declared range accepts the world's data format. An
/// undeclared (open) end never blocks, and a pack with no range at all is
/// treated as unknown rather than incompatible.
pub fn range_accepts(
    min: Option<FormatVersion>,
    max: Option<FormatVersion>,
    format: FormatVersion,
) -> bool {
    if let Some(min) = min {
        if format < min {
            return false;
        }
    }
    if let Some(max) = max {
        if max < format {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::FormatRange;

    #[test]
    fn resolves_the_1_21_series() {
        assert_eq!(
            data_format_for_version("1.21.10"),
            Some(FormatVersion::from_major(88))
        );
        assert_eq!(
            data_format_for_version("1.21.9"),
            Some(FormatVersion::from_major(88))
        );
        assert_eq!(
            data_format_for_version("1.21.4"),
            Some(FormatVersion::from_major(61))
        );
        assert_eq!(
            data_format_for_version("1.21.11"),
            Some(FormatVersion::new(94, 1))
        );
    }

    #[test]
    fn resolves_older_eras() {
        assert_eq!(
            data_format_for_version("1.17.1"),
            Some(FormatVersion::from_major(7))
        );
        assert_eq!(
            data_format_for_version("1.19.2"),
            Some(FormatVersion::from_major(10))
        );
        assert_eq!(data_format_for_version("9.9.9"), None);
    }

    #[test]
    fn unknown_versions_resolve_to_none() {
        assert_eq!(data_format_for_version(""), None);
        assert_eq!(data_format_for_version("9.9.9"), None);
        assert_eq!(
            data_format_for_version("1.21"),
            Some(FormatVersion::from_major(48))
        );
    }

    #[test]
    fn range_check_covers_the_1_17_packs_on_1_21_10() {
        let world = FormatVersion::from_major(88);
        let old = FormatRange {
            min: Some(FormatVersion::from_major(7)),
            max: Some(FormatVersion::from_major(7)),
        };
        assert!(!range_accepts(old.min, old.max, world));
        let wide = FormatRange {
            min: Some(FormatVersion::from_major(7)),
            max: None,
        };
        assert!(range_accepts(wide.min, wide.max, world));
    }
}
