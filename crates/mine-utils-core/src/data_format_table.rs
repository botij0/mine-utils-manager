//! Resolve a world version name such as `1.21.10` to the data pack format the
//! world's datapacks must declare. The mapping comes from the `data` rows of
//! `data/pack_formats.json`, whose labels cover `1.13` through `1.21.11`.
//!
//! Labels name one version (`1.21.4`) or a span (`1.21.9 to 1.21.10`,
//! `1.19 to 1.19.3`). Spans expand over the named endpoints, so `1.19 to
//! 1.19.3` means 1.19, 1.19.1, 1.19.2, and 1.19.3. Cross-major spans such as
//! `1.13 to 1.14.4` expand numerically across the series boundary, so every
//! real world version in `1.13`–`1.16.5` resolves.

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
    // The data rows are the source of truth; the labels are parsed, not a
    // probe list, so a future table edit flows through automatically and no
    // covered format keeps a silent blind spot.
    let mut out = Vec::new();
    let mut seen = HashMap::new();
    for format in table.formats(PackKind::Data) {
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

fn expand_label(label: &str) -> Vec<String> {
    let (start, end) = match label.split_once(" to ") {
        Some((start, end)) => (start.trim(), end.trim()),
        None => return vec![label.trim().to_string()],
    };
    let (Some(start), Some(end)) = (version_parts(start), version_parts(end)) else {
        return vec![label.trim().to_string()];
    };
    if end < start {
        return Vec::new();
    }
    // Numeric expansion across the series boundary: every (series, patch)
    // between the endpoints. Patches cap at 30, which every real span stays
    // under; nonexistent versions inside the range map harmlessly and rows
    // stay disjoint because spans never overlap.
    let mut out = Vec::new();
    for series in start.0..=end.0 {
        for patch in 0..=30 {
            let version = (series, patch);
            if version < start || version > end {
                continue;
            }
            out.push(format_version(series, patch));
        }
    }
    out
}

fn format_version(series: u32, patch: u32) -> String {
    if patch == 0 {
        format!("1.{series}")
    } else {
        format!("1.{series}.{patch}")
    }
}

fn version_parts(text: &str) -> Option<(u32, u32)> {
    // Minecraft versions are `1.<series>[.<patch>]`: `1.21.10` is series 21,
    // patch 10, and a bare `1.21` is series 21, patch 0. Tuples compare
    // numerically, so `1.21.10` sorts after `1.21.1` without string tricks.
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

fn index_for(table: &FormatTable) -> HashMap<String, FormatVersion> {
    table_versions(table)
        .into_iter()
        .map(|entry| (entry.version, entry.format))
        .collect()
}

fn index() -> &'static HashMap<String, FormatVersion> {
    static INDEX: OnceLock<HashMap<String, FormatVersion>> = OnceLock::new();
    INDEX.get_or_init(|| index_for(FormatTable::embedded()))
}

/// The data format for a world version name such as `1.21.10`, or `None` when
/// the table does not cover it.
pub fn data_format_for_version(version: &str) -> Option<FormatVersion> {
    index().get(version.trim()).copied()
}

/// The same resolver against an explicit table, so tests can prove a future
/// table row resolves without touching the embedded index.
pub fn data_format_for_version_in(table: &FormatTable, version: &str) -> Option<FormatVersion> {
    index_for(table).get(version.trim()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cross_major_spans_resolve_every_real_version() {
        // `1.13 to 1.14.4`, `1.15 to 1.16.1`, `1.16.2 to 1.16.5`.
        for (version, format) in [
            ("1.13", 4),
            ("1.13.2", 4),
            ("1.14", 4),
            ("1.14.4", 4),
            ("1.15", 5),
            ("1.15.2", 5),
            ("1.16", 5),
            ("1.16.1", 5),
            ("1.16.2", 6),
            ("1.16.5", 6),
            ("1.21.1", 48),
            ("1.21.10", 88),
        ] {
            assert_eq!(
                data_format_for_version(version),
                Some(FormatVersion::from_major(format)),
                "{version} must resolve"
            );
        }
        // Numeric order, not string order: 1.21.10 sorts after 1.21.1.
        assert!(version_parts("1.21.10") > version_parts("1.21.1"));
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
    fn a_future_table_row_resolves_without_an_embedded_change() {
        let table = FormatTable::from_json(
            r#"{"resource": [], "data": [{"format": [99, 0], "label": "1.99"}]}"#,
        )
        .unwrap();
        assert_eq!(
            data_format_for_version_in(&table, "1.99"),
            Some(FormatVersion::from_major(99))
        );
        assert_eq!(data_format_for_version_in(&table, "1.98"), None);
    }

    #[test]
    fn every_embedded_data_row_resolves() {
        let table = FormatTable::embedded();
        for row in table.data_rows() {
            for version in expand_label(&row.label) {
                // Skip synthetic over-generated patches; every named endpoint
                // and every real version must resolve to this row's format.
                if version_patch_above_known(version.as_str()) {
                    continue;
                }
                assert_eq!(
                    data_format_for_version(version.as_str()),
                    Some(row.format),
                    "{version} must resolve to {}",
                    row.format.label()
                );
            }
        }
        assert!(
            !table.data_rows().is_empty(),
            "the data table must carry rows"
        );
    }

    /// Over-generated patches (no real release) are exempt from coverage;
    /// endpoints always count.
    fn version_patch_above_known(version: &str) -> bool {
        let Some((_, patch)) = version_parts(version) else {
            return false;
        };
        patch > 11
    }
}
