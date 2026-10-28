//! Public entry points for resolving a world version name such as `1.21.10`
//! to the data pack format its datapacks must declare. The mapping lives in
//! [`FormatTable`], built from the `data` rows of `data/pack_formats.json`.

use crate::format_table::FormatTable;
use crate::pack::FormatVersion;

/// The data format for a world version name such as `1.21.10`, or `None` when
/// the table does not cover it.
pub fn data_format_for_version(version: &str) -> Option<FormatVersion> {
    FormatTable::embedded().data_format_for_version(version)
}

/// The same resolver against an explicit table, so tests can prove a future
/// table row resolves without touching the embedded index.
pub fn data_format_for_version_in(table: &FormatTable, version: &str) -> Option<FormatVersion> {
    table.data_format_for_version(version)
}
