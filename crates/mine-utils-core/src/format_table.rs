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

/// Maps a pack format number to the Minecraft versions that read it. Resource
/// and data packs moved to separate numbers in 1.21, so they stay separate.
#[derive(Debug)]
pub struct FormatTable {
    resource: Vec<FormatRangeEntry>,
    data: Vec<FormatRangeEntry>,
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
        Ok(FormatTable {
            resource: convert(raw.resource),
            data: convert(raw.data),
        })
    }

    pub fn label_for(&self, kind: PackKind, version: FormatVersion) -> Option<&str> {
        let entries = match kind {
            PackKind::Data => &self.data,
            _ => &self.resource,
        };
        lookup(entries, version)
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
}
