use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::pack::{FormatRange, FormatVersion, PackKind};

const PACK_FORMATS_JSON: &str = include_str!("../data/pack_formats.json");

#[derive(Debug, Deserialize)]
struct RawEntry {
    format: [u32; 2],
    label: String,
}

#[derive(Debug, Deserialize)]
struct RawTable {
    resource: Vec<RawEntry>,
    data: Vec<RawEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRangeEntry {
    pub format: FormatVersion,
    pub label: String,
}

/// A game version as `(major, minor, patch)`. `1.21.10` is `(1, 21, 10)` and
/// the year-based `26.1.2` is `(26, 1, 2)`; a missing patch is 0. Tuples compare
/// numerically, so `1.21.10` sorts after `1.21.1` and `26.1` after `1.21.11`.
type GameVersion = (u32, u32, u32);

/// Maps a pack format number to the Minecraft versions that read it. Resource
/// and data packs moved to separate numbers in 1.21, so they stay separate.
#[derive(Debug)]
pub struct FormatTable {
    resource: Vec<FormatRangeEntry>,
    data: Vec<FormatRangeEntry>,
    data_formats: HashMap<String, FormatVersion>,
    newest_data_version: Option<GameVersion>,
}

/// The data format for a world version name such as `1.21.10` or `26.1.2`, or
/// `None` when the table does not cover it.
pub fn data_format_for_version(version: &str) -> Option<FormatVersion> {
    FormatTable::embedded().data_format_for_version(version)
}

/// The newest version the embedded table covers, but only when `version` is
/// newer than that. `None` means the version is covered, older, or unparsable.
pub fn newest_known_version_if_newer(version: &str) -> Option<String> {
    FormatTable::embedded().newest_known_version_if_newer(version)
}

impl FormatTable {
    pub fn embedded() -> &'static FormatTable {
        static TABLE: OnceLock<FormatTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            FormatTable::from_json(PACK_FORMATS_JSON).expect("pack_formats.json is valid")
        })
    }

    pub fn from_json(json: &str) -> Result<FormatTable, String> {
        let raw: RawTable =
            serde_json::from_str(json).map_err(|err| format!("invalid format table: {err}"))?;
        let data = convert(raw.data);
        let data_formats = data_versions(&data);
        let newest_data_version = newest_version(&data);
        Ok(FormatTable {
            resource: convert(raw.resource),
            data,
            data_formats,
            newest_data_version,
        })
    }

    pub fn label_for(&self, kind: PackKind, version: FormatVersion) -> Option<&str> {
        let entries = match kind {
            PackKind::Data => &self.data,
            _ => &self.resource,
        };
        lookup(entries, version)
    }

    /// The data format for a world version name such as `1.21.10`, or `None`
    /// when the data rows do not cover it. Labels name the versions, so a
    /// future table row resolves without a probe list.
    pub fn data_format_for_version(&self, version: &str) -> Option<FormatVersion> {
        self.data_formats.get(version.trim()).copied()
    }

    /// The newest data-row version, named like the labels (`26.3`), when
    /// `version` sorts after it. A world past this cannot have its datapacks
    /// checked, which the UI must say instead of reporting compatible.
    pub fn newest_known_version_if_newer(&self, version: &str) -> Option<String> {
        let newest = self.newest_data_version?;
        (version_parts(version.trim())? > newest).then(|| format_version(newest))
    }

    /// Game versions for the declared range. A range must map at both ends and
    /// must not be reversed; an open end is named as open instead of guessed,
    /// and a `Both` pack is labelled for each half from its own table.
    pub fn game_label(&self, kind: PackKind, range: &FormatRange) -> Option<String> {
        match kind {
            PackKind::Resource => half_label(&self.resource, range),
            PackKind::Data => half_label(&self.data, range),
            PackKind::Both => {
                let resource = half_label(&self.resource, range);
                let data = half_label(&self.data, range);
                match (resource, data) {
                    (Some(resource), Some(data)) if resource == data => Some(resource),
                    (resource, data) => {
                        if resource.is_none() && data.is_none() {
                            return None;
                        }
                        Some(format!(
                            "Resource {}, data {}",
                            resource.unwrap_or_else(|| "unknown".to_string()),
                            data.unwrap_or_else(|| "unknown".to_string()),
                        ))
                    }
                }
            }
            PackKind::Unknown => None,
        }
    }
}

fn lookup(entries: &[FormatRangeEntry], version: FormatVersion) -> Option<&str> {
    entries
        .iter()
        .find(|entry| entry.format == version)
        .map(|entry| entry.label.as_str())
}

fn start_of(label: &str) -> &str {
    label.split(" to ").next().unwrap_or(label)
}

fn end_of(label: &str) -> &str {
    label.rsplit(" to ").next().unwrap_or(label)
}

fn half_label(entries: &[FormatRangeEntry], range: &FormatRange) -> Option<String> {
    match (range.min, range.max) {
        (Some(min), Some(max)) => {
            if min > max {
                return None;
            }
            let start = lookup(entries, min)?;
            if min == max {
                return Some(start.to_string());
            }
            let end = end_of(lookup(entries, max)?);
            let start = start_of(start);
            if start == end {
                Some(start.to_string())
            } else {
                Some(format!("{start} to {end}"))
            }
        }
        (Some(min), None) => {
            lookup(entries, min).map(|label| format!("{} and newer", start_of(label)))
        }
        (None, Some(max)) => lookup(entries, max).map(|label| format!("up to {}", end_of(label))),
        (None, None) => None,
    }
}

/// The first data row naming a version wins; rows are disjoint.
fn data_versions(rows: &[FormatRangeEntry]) -> HashMap<String, FormatVersion> {
    let mut versions = HashMap::new();
    for row in rows {
        for version in expand_label(&row.label) {
            versions.entry(version).or_insert(row.format);
        }
    }
    versions
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
    if start.0 != end.0 {
        // A span across a renumbering (`1.x` to `26.x`) has no countable
        // middle, so only the named endpoints resolve.
        return vec![format_version(start), format_version(end)];
    }
    // Numeric expansion inside one major: every (minor, patch) between the
    // endpoints. Patches cap at 30, which every real span stays under;
    // nonexistent versions inside the range map harmlessly and rows stay
    // disjoint because spans never overlap.
    let mut out = Vec::new();
    for minor in start.1..=end.1 {
        for patch in 0..=30 {
            let version = (start.0, minor, patch);
            if version < start || version > end {
                continue;
            }
            out.push(format_version(version));
        }
    }
    out
}

/// `1.21`, `1.21.10`, `26.1`, and `26.1.2` all follow one rule: the patch is
/// shown only when it is not 0, so a label stays in its endpoints' scheme.
fn format_version((major, minor, patch): GameVersion) -> String {
    if patch == 0 {
        format!("{major}.{minor}")
    } else {
        format!("{major}.{minor}.{patch}")
    }
}

fn version_parts(text: &str) -> Option<GameVersion> {
    let mut parts = text.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = match parts.next() {
        Some(patch) => patch.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// The highest version any row's label names at either end.
fn newest_version(rows: &[FormatRangeEntry]) -> Option<GameVersion> {
    rows.iter()
        .filter_map(|row| version_parts(end_of(&row.label).trim()))
        .max()
}

fn convert(entries: Vec<RawEntry>) -> Vec<FormatRangeEntry> {
    entries
        .into_iter()
        .map(|entry| FormatRangeEntry {
            format: FormatVersion::new(entry.format[0], entry.format[1]),
            label: entry.label,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_has_the_1_21_series() {
        let table = FormatTable::embedded();
        let resource = |major, minor| {
            table
                .label_for(PackKind::Resource, FormatVersion::new(major, minor))
                .map(str::to_string)
        };
        assert_eq!(resource(34, 0).as_deref(), Some("1.21 to 1.21.1"));
        assert_eq!(resource(42, 0).as_deref(), Some("1.21.2 to 1.21.3"));
        assert_eq!(resource(46, 0).as_deref(), Some("1.21.4"));
        assert_eq!(resource(55, 0).as_deref(), Some("1.21.5"));
        assert_eq!(resource(63, 0).as_deref(), Some("1.21.6"));
        assert_eq!(resource(64, 0).as_deref(), Some("1.21.7 to 1.21.8"));
        assert_eq!(resource(69, 0).as_deref(), Some("1.21.9 to 1.21.10"));
        assert_eq!(resource(75, 0).as_deref(), Some("1.21.11"));
        assert_eq!(resource(7, 0).as_deref(), Some("1.17 to 1.17.1"));
        assert_eq!(resource(99, 0), None);
    }

    #[test]
    fn data_table_is_separate() {
        let table = FormatTable::embedded();
        let label = |major, minor| {
            table
                .label_for(PackKind::Data, FormatVersion::new(major, minor))
                .map(str::to_string)
        };
        assert_eq!(label(48, 0).as_deref(), Some("1.21 to 1.21.1"));
        assert_eq!(label(57, 0).as_deref(), Some("1.21.2 to 1.21.3"));
        assert_eq!(label(61, 0).as_deref(), Some("1.21.4"));
        assert_eq!(label(71, 0).as_deref(), Some("1.21.5"));
        assert_eq!(label(80, 0).as_deref(), Some("1.21.6"));
        assert_eq!(label(81, 0).as_deref(), Some("1.21.7 to 1.21.8"));
        assert_eq!(label(88, 0).as_deref(), Some("1.21.9 to 1.21.10"));
        assert_eq!(label(94, 1).as_deref(), Some("1.21.11"));
        assert_eq!(label(34, 0), None);
    }

    #[test]
    fn range_label_spans_both_ends() {
        let table = FormatTable::embedded();
        let label = table.game_label(
            PackKind::Resource,
            &FormatRange {
                min: Some(FormatVersion::from_major(34)),
                max: Some(FormatVersion::from_major(69)),
            },
        );
        assert_eq!(label.as_deref(), Some("1.21 to 1.21.10"));
        assert_eq!(
            table.game_label(
                PackKind::Data,
                &FormatRange {
                    min: Some(FormatVersion::from_major(94)),
                    max: Some(FormatVersion::from_major(1)),
                },
            ),
            None
        );
    }

    #[test]
    fn pre_1_21_data_rows_are_corrected() {
        let table = FormatTable::embedded();
        let label = |major| {
            table
                .label_for(PackKind::Data, FormatVersion::from_major(major))
                .map(str::to_string)
        };
        assert_eq!(label(4).as_deref(), Some("1.13 to 1.14.4"));
        assert_eq!(label(5).as_deref(), Some("1.15 to 1.16.1"));
        assert_eq!(label(6).as_deref(), Some("1.16.2 to 1.16.5"));
        assert_eq!(label(7).as_deref(), Some("1.17 to 1.17.1"));
        assert_eq!(label(8).as_deref(), Some("1.18 to 1.18.1"));
        assert_eq!(label(9).as_deref(), Some("1.18.2"));
        assert_eq!(label(10).as_deref(), Some("1.19 to 1.19.3"));
        assert_eq!(label(12).as_deref(), Some("1.19.4"));
        assert_eq!(label(1), None, "format 1 is not a data pack format");
    }

    #[test]
    fn a_range_with_an_unknown_end_shows_no_narrower_label() {
        let table = FormatTable::embedded();
        assert_eq!(
            table.game_label(
                PackKind::Resource,
                &FormatRange {
                    min: Some(FormatVersion::from_major(34)),
                    max: Some(FormatVersion::from_major(99)),
                },
            ),
            None,
            "an unknown end must not fall back to the 34 label"
        );
        assert_eq!(
            table.game_label(
                PackKind::Resource,
                &FormatRange {
                    min: Some(FormatVersion::from_major(50)),
                    max: Some(FormatVersion::from_major(10)),
                },
            ),
            None,
            "a reversed range is unknown"
        );
    }

    #[test]
    fn an_any_minor_max_labels_the_highest_row_of_that_major() {
        let table = FormatTable::embedded();
        let range = FormatRange {
            min: Some(FormatVersion::from_major(88)),
            max: Some(FormatVersion::new(94, 0x7fff_ffff)),
        };
        assert_eq!(
            table.game_label(PackKind::Data, &range).as_deref(),
            Some("1.21.9 to 1.21.11")
        );
        let lone_max = FormatRange {
            min: None,
            max: Some(FormatVersion::new(101, 0x7fff_ffff)),
        };
        assert_eq!(
            table.game_label(PackKind::Data, &lone_max).as_deref(),
            Some("up to 26.1.2")
        );
    }

    #[test]
    fn a_whole_number_min_matches_the_lowest_row_of_its_major() {
        let table = FormatTable::embedded();
        let range = FormatRange {
            min: Some(FormatVersion::from_major(101)),
            max: Some(FormatVersion::new(101, 0x7fff_ffff)),
        };
        assert_eq!(
            table.game_label(PackKind::Data, &range).as_deref(),
            Some("26.1 to 26.1.2")
        );
    }

    #[test]
    fn open_ranges_name_the_open_end() {
        let table = FormatTable::embedded();
        assert_eq!(
            table
                .game_label(
                    PackKind::Resource,
                    &FormatRange {
                        min: Some(FormatVersion::from_major(34)),
                        max: None,
                    },
                )
                .as_deref(),
            Some("1.21 and newer")
        );
        assert_eq!(
            table
                .game_label(
                    PackKind::Data,
                    &FormatRange {
                        min: None,
                        max: Some(FormatVersion::from_major(48)),
                    },
                )
                .as_deref(),
            Some("up to 1.21.1")
        );
    }

    #[test]
    fn a_both_pack_is_labelled_for_each_half() {
        let table = FormatTable::embedded();
        let range = |min, max| FormatRange {
            min: Some(FormatVersion::from_major(min)),
            max: Some(FormatVersion::from_major(max)),
        };
        assert_eq!(
            table.game_label(PackKind::Both, &range(48, 57)).as_deref(),
            Some("Resource unknown, data 1.21 to 1.21.3"),
            "the data half must use the data table"
        );
        assert_eq!(
            table.game_label(PackKind::Both, &range(34, 42)).as_deref(),
            Some("Resource 1.21 to 1.21.3, data unknown"),
            "the resource half must not be reused for the data half"
        );
        assert_eq!(
            table.game_label(PackKind::Both, &range(15, 15)).as_deref(),
            Some("1.20 to 1.20.1"),
            "both halves agreeing shows one label"
        );
        assert_eq!(table.game_label(PackKind::Both, &range(99, 99)), None);
    }

    #[test]
    fn resolves_the_1_21_series() {
        let table = FormatTable::embedded();
        assert_eq!(
            table.data_format_for_version("1.21.10"),
            Some(FormatVersion::from_major(88))
        );
        assert_eq!(
            table.data_format_for_version("1.21.9"),
            Some(FormatVersion::from_major(88))
        );
        assert_eq!(
            table.data_format_for_version("1.21.4"),
            Some(FormatVersion::from_major(61))
        );
        assert_eq!(
            table.data_format_for_version("1.21.11"),
            Some(FormatVersion::new(94, 1))
        );
    }

    #[test]
    fn resolves_older_eras() {
        let table = FormatTable::embedded();
        assert_eq!(
            table.data_format_for_version("1.17.1"),
            Some(FormatVersion::from_major(7))
        );
        assert_eq!(
            table.data_format_for_version("1.19.2"),
            Some(FormatVersion::from_major(10))
        );
        assert_eq!(table.data_format_for_version("9.9.9"), None);
    }

    #[test]
    fn cross_major_spans_resolve_every_real_version() {
        let table = FormatTable::embedded();
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
                table.data_format_for_version(version),
                Some(FormatVersion::from_major(format)),
                "{version} must resolve"
            );
        }
        // Numeric order, not string order: 1.21.10 sorts after 1.21.1.
        assert!(version_parts("1.21.10") > version_parts("1.21.1"));
    }

    #[test]
    fn unknown_versions_resolve_to_none() {
        let table = FormatTable::embedded();
        assert_eq!(table.data_format_for_version(""), None);
        assert_eq!(table.data_format_for_version("9.9.9"), None);
        assert_eq!(
            table.data_format_for_version("1.21"),
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
            table.data_format_for_version("1.99"),
            Some(FormatVersion::from_major(99))
        );
        assert_eq!(table.data_format_for_version("1.98"), None);
    }

    #[test]
    fn a_year_based_row_resolves_every_version_in_its_span() {
        let table = FormatTable::from_json(
            r#"{"resource": [], "data": [{"format": [101, 1], "label": "26.1 to 26.1.2"}]}"#,
        )
        .unwrap();
        for version in ["26.1", "26.1.1", "26.1.2"] {
            assert_eq!(
                table.data_format_for_version(version),
                Some(FormatVersion::new(101, 1)),
                "{version} must resolve"
            );
        }
        assert_eq!(table.data_format_for_version("26.2"), None);
        assert_eq!(table.data_format_for_version("26.1.3"), None);
    }

    #[test]
    fn the_embedded_table_resolves_the_26_x_releases() {
        let table = FormatTable::embedded();
        for (version, format) in [
            ("26.1", FormatVersion::new(101, 1)),
            ("26.1.1", FormatVersion::new(101, 1)),
            ("26.1.2", FormatVersion::new(101, 1)),
            ("26.2", FormatVersion::new(107, 1)),
            ("26.3", FormatVersion::from_major(121)),
        ] {
            assert_eq!(
                table.data_format_for_version(version),
                Some(format),
                "{version} must resolve"
            );
        }
        let label = |kind, format| table.label_for(kind, format).map(str::to_string);
        assert_eq!(
            label(PackKind::Resource, FormatVersion::from_major(84)).as_deref(),
            Some("26.1 to 26.1.2")
        );
        assert_eq!(
            label(PackKind::Resource, FormatVersion::from_major(88)).as_deref(),
            Some("26.2")
        );
        assert_eq!(
            label(PackKind::Resource, FormatVersion::new(97, 1)).as_deref(),
            Some("26.3")
        );
        assert_eq!(
            label(PackKind::Data, FormatVersion::new(101, 1)).as_deref(),
            Some("26.1 to 26.1.2")
        );
    }

    #[test]
    fn every_embedded_data_row_resolves() {
        let table = FormatTable::embedded();
        for row in &table.data {
            // Named endpoints always count, even a future high patch: only
            // over-generated interior patches above every real release are
            // exempt, so a future high-patch row stays covered.
            let endpoints = row_endpoints(&row.label);
            for version in expand_label(&row.label) {
                let is_endpoint =
                    version_parts(version.as_str()).is_some_and(|parts| endpoints.contains(&parts));
                if !is_endpoint && version_patch_above_known(version.as_str()) {
                    continue;
                }
                assert_eq!(
                    table.data_format_for_version(version.as_str()),
                    Some(row.format),
                    "{version} must resolve to {}",
                    row.format.label()
                );
            }
        }
        assert!(!table.data.is_empty(), "the data table must carry rows");
    }

    /// The parsed endpoints of one row label: a single version, or both ends
    /// of a span. Compared numerically, so `1.21` and `1.21.0` are the same.
    fn row_endpoints(label: &str) -> Vec<GameVersion> {
        let (start, end) = match label.split_once(" to ") {
            Some((start, end)) => (start.trim(), end.trim()),
            None => return version_parts(label.trim()).into_iter().collect(),
        };
        [start, end].into_iter().filter_map(version_parts).collect()
    }

    /// Over-generated patches (no real release) are exempt from coverage;
    /// endpoints always count.
    fn version_patch_above_known(version: &str) -> bool {
        let Some((_, _, patch)) = version_parts(version) else {
            return false;
        };
        patch > 11
    }
}
