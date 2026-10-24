use std::sync::OnceLock;

use serde::Deserialize;

use crate::pack::{FormatVersion, PackKind};

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
        entries
            .iter()
            .find(|entry| entry.format == version)
            .map(|entry| entry.label.as_str())
    }

    pub fn game_label(
        &self,
        kind: PackKind,
        min: FormatVersion,
        max: FormatVersion,
    ) -> Option<String> {
        let start = self.label_for(kind, min)?;
        if min == max {
            return Some(start.to_string());
        }
        let end = self.label_for(kind, max)?;
        let start = start.split(" to ").next().unwrap_or(start);
        let end = end.rsplit(" to ").next().unwrap_or(end);
        if start == end {
            Some(start.to_string())
        } else {
            Some(format!("{start} to {end}"))
        }
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
            FormatVersion::from_major(34),
            FormatVersion::from_major(69),
        );
        assert_eq!(label.as_deref(), Some("1.21 to 1.21.10"));
        assert_eq!(
            table.game_label(
                PackKind::Data,
                FormatVersion::from_major(94),
                FormatVersion::from_major(1),
            ),
            None
        );
    }
}
