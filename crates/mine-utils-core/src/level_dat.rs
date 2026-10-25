//! Read and modify `level.dat`, the gzip-compressed NBT file that owns the world
//! name, the world version, and the datapack lists.
//!
//! The file is parsed to a `fastnbt::Value` tree. Only the two string lists at
//! `Data.DataPacks.Enabled` and `Data.DataPacks.Disabled` are ever mutated;
//! every other tag is carried through untouched. Tag order is preserved as far
//! as `fastnbt` allows: tests compare the parsed value tree outside
//! `Data.DataPacks`, never gzip byte identity.

use std::io::{Read, Write};
use std::path::Path;

use fastnbt::Value;

/// Everything that can go wrong while reading or rewriting `level.dat`. The
/// app never repairs the file, so a malformed file is a read-only error the
/// user resolves by launching Minecraft once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LevelDatError {
    #[error("level.dat is not valid gzip: {reason}")]
    CorruptGzip { reason: String },
    #[error("level.dat does not hold an NBT compound at its root")]
    NotCompound,
    #[error("level.dat has no Data.Version.Name; launch Minecraft once so it writes the file")]
    MissingVersion,
    #[error("level.dat has no usable Data.DataPacks: {reason}")]
    MissingDataPacks { reason: String },
    #[error("level.dat has malformed datapack lists: {reason}")]
    MalformedArrays { reason: String },
    #[error("level.dat changed on disk; reload it and try again")]
    StaleFile,
}

impl LevelDatError {
    pub fn code(&self) -> &'static str {
        match self {
            LevelDatError::StaleFile => "stale_stack",
            _ => "level_dat",
        }
    }
}

/// What the UI needs from `level.dat`: the display name, the game version, and
/// the two datapack lists in load order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelDatView {
    pub level_name: Option<String>,
    pub version_name: String,
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
}

fn corrupt(err: impl std::fmt::Display) -> LevelDatError {
    LevelDatError::CorruptGzip {
        reason: err.to_string(),
    }
}

/// Decompress one `level.dat` file into its raw NBT bytes.
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, LevelDatError> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).map_err(corrupt)?;
    Ok(raw)
}

/// Compress raw NBT bytes the way Minecraft stores them.
pub fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).expect("gzip to memory never fails");
    encoder.finish().expect("gzip to memory never fails")
}

/// Parse decompressed NBT bytes to a value tree. The root must be a compound.
pub fn parse_nbt(raw: &[u8]) -> Result<Value, LevelDatError> {
    let value: Value =
        fastnbt::from_bytes(raw).map_err(|_| LevelDatError::NotCompound)?;
    match &value {
        Value::Compound(_) => Ok(value),
        _ => Err(LevelDatError::NotCompound),
    }
}

/// Parse a whole `level.dat` file into the UI view.
pub fn parse(bytes: &[u8]) -> Result<LevelDatView, LevelDatError> {
    let raw = gunzip(bytes)?;
    let value = parse_nbt(&raw)?;
    view_of(&value)
}

fn data_of<'a>(value: &'a Value) -> Result<&'a std::collections::HashMap<String, Value>, LevelDatError> {
    match value {
        Value::Compound(root) => match root.get("Data") {
            Some(Value::Compound(data)) => Ok(data),
            _ => Err(LevelDatError::MissingDataPacks {
                reason: "Data is not a compound".to_string(),
            }),
        },
        _ => Err(LevelDatError::NotCompound),
    }
}

fn view_of(value: &Value) -> Result<LevelDatView, LevelDatError> {
    let data = data_of(value)?;
    let version_name = match data.get("Version") {
        Some(Value::Compound(version)) => match version.get("Name") {
            Some(Value::String(name)) => name.clone(),
            _ => return Err(LevelDatError::MissingVersion),
        },
        _ => return Err(LevelDatError::MissingVersion),
    };
    let level_name = match data.get("LevelName") {
        Some(Value::String(name)) => Some(name.clone()),
        _ => None,
    };
    let (enabled, disabled) = lists_of(data)?;
    Ok(LevelDatView {
        level_name,
        version_name,
        enabled,
        disabled,
    })
}

fn strings_of(list: &Value, key: &str) -> Result<Vec<String>, LevelDatError> {
    match list {
        Value::List(entries) => entries
            .iter()
            .map(|entry| match entry {
                Value::String(text) => Ok(text.clone()),
                _ => Err(LevelDatError::MalformedArrays {
                    reason: format!("{key} holds a non-string entry"),
                }),
            })
            .collect(),
        _ => Err(LevelDatError::MissingDataPacks {
            reason: format!("{key} is not a list"),
        }),
    }
}

fn lists_of(
    data: &std::collections::HashMap<String, Value>,
) -> Result<(Vec<String>, Vec<String>), LevelDatError> {
    let packs = match data.get("DataPacks") {
        Some(Value::Compound(packs)) => packs,
        _ => {
            return Err(LevelDatError::MissingDataPacks {
                reason: "Data.DataPacks is not a compound".to_string(),
            });
        }
    };
    let enabled = match packs.get("Enabled") {
        Some(list) => strings_of(list, "Enabled")?,
        None => {
            return Err(LevelDatError::MissingDataPacks {
                reason: "Data.DataPacks.Enabled is missing".to_string(),
            });
        }
    };
    let disabled = match packs.get("Disabled") {
        Some(list) => strings_of(list, "Disabled")?,
        None => {
            return Err(LevelDatError::MissingDataPacks {
                reason: "Data.DataPacks.Disabled is missing".to_string(),
            });
        }
    };
    Ok((enabled, disabled))
}

/// Replace both datapack lists and re-encode. Every other tag is preserved.
pub fn render_with(
    bytes: &[u8],
    enabled: &[String],
    disabled: &[String],
) -> Result<Vec<u8>, LevelDatError> {
    let raw = gunzip(bytes)?;
    let mut value = parse_nbt(&raw)?;
    replace_lists(&mut value, enabled, disabled)?;
    let encoded =
        fastnbt::to_bytes(&value).map_err(|err| LevelDatError::MalformedArrays {
            reason: format!("could not re-encode level.dat: {err}"),
        })?;
    Ok(gzip(&encoded))
}

fn replace_lists(
    value: &mut Value,
    enabled: &[String],
    disabled: &[String],
) -> Result<(), LevelDatError> {
    let Value::Compound(root) = value else {
        return Err(LevelDatError::NotCompound);
    };
    let Some(Value::Compound(data)) = root.get_mut("Data") else {
        return Err(LevelDatError::MissingDataPacks {
            reason: "Data is not a compound".to_string(),
        });
    };
    let Some(Value::Compound(packs)) = data.get_mut("DataPacks") else {
        return Err(LevelDatError::MissingDataPacks {
            reason: "Data.DataPacks is not a compound".to_string(),
        });
    };
    packs.insert(
        "Enabled".to_string(),
        Value::List(enabled.iter().cloned().map(Value::String).collect()),
    );
    packs.insert(
        "Disabled".to_string(),
        Value::List(disabled.iter().cloned().map(Value::String).collect()),
    );
    Ok(())
}

/// Move one datapack between the two lists and re-encode. Enabling removes
/// `file/<filename>` from `Disabled` and appends it to `Enabled`; disabling
/// does the reverse. Duplicates are removed on disable; enabling is
/// idempotent.
pub fn set_enabled(
    bytes: &[u8],
    filename: &str,
    enabled: bool,
) -> Result<Vec<u8>, LevelDatError> {
    let view = parse(bytes)?;
    let value = format!("file/{filename}");
    let (mut next_enabled, mut next_disabled) = (view.enabled, view.disabled);
    if enabled {
        next_disabled.retain(|entry| entry != &value);
        if !next_enabled.contains(&value) {
            next_enabled.push(value);
        }
    } else {
        next_enabled.retain(|entry| entry != &value);
        next_disabled.retain(|entry| entry != &value);
        next_disabled.push(value);
    }
    render_with(bytes, &next_enabled, &next_disabled)
}

/// Fail when any tag outside `Data.DataPacks` differs between two files. The
/// comparison runs on the parsed value tree, so gzip bytes may differ.
pub fn assert_outside_datapacks_unchanged(
    before: &[u8],
    after: &[u8],
) -> Result<(), LevelDatError> {
    let before_raw = gunzip(before)?;
    let after_raw = gunzip(after)?;
    let mut before_value = parse_nbt(&before_raw)?;
    let mut after_value = parse_nbt(&after_raw)?;
    strip_datapacks(&mut before_value)?;
    strip_datapacks(&mut after_value)?;
    if before_value == after_value {
        Ok(())
    } else {
        Err(LevelDatError::MalformedArrays {
            reason: "a tag outside Data.DataPacks changed".to_string(),
        })
    }
}

fn strip_datapacks(value: &mut Value) -> Result<(), LevelDatError> {
    let Value::Compound(root) = value else {
        return Err(LevelDatError::NotCompound);
    };
    let Some(Value::Compound(data)) = root.get_mut("Data") else {
        return Err(LevelDatError::MissingDataPacks {
            reason: "Data is not a compound".to_string(),
        });
    };
    data.remove("DataPacks");
    Ok(())
}

/// Read `saves/<world>/level.dat` into the UI view.
pub fn read_level_dat(world_dir: &Path) -> Result<LevelDatView, LevelDatError> {
    let path = world_dir.join("level.dat");
    let bytes = std::fs::read(&path).map_err(corrupt)?;
    parse(&bytes)
}

/// Write `level.dat` atomically: temp file in the world dir, `sync_all`,
/// rename, directory `fsync`. A failed write leaves the previous file intact,
/// and `level.dat_old` is never touched.
pub fn write_level_dat(world_dir: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let target = world_dir.join("level.dat");
    let tmp = world_dir.join(format!(".level.dat.{}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &target)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    if let Ok(handle) = std::fs::File::open(world_dir) {
        let _ = handle.sync_all();
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compound(pairs: Vec<(&str, Value)>) -> Value {
        Value::Compound(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn fixture_lists(enabled: &[&str], disabled: &[&str]) -> Vec<u8> {
        let value = compound(vec![(
            "Data",
            compound(vec![
                ("LevelName", Value::String("Test World".to_string())),
                (
                    "Version",
                    compound(vec![("Name", Value::String("1.21.10".to_string()))]),
                ),
                (
                    "DataPacks",
                    compound(vec![
                        (
                            "Enabled",
                            Value::List(
                                enabled.iter().map(|e| Value::String(e.to_string())).collect(),
                            ),
                        ),
                        (
                            "Disabled",
                            Value::List(
                                disabled.iter().map(|e| Value::String(e.to_string())).collect(),
                            ),
                        ),
                    ]),
                ),
                ("Seed", Value::Long(12345)),
            ]),
        )]);
        gzip(&fastnbt::to_bytes(&value).unwrap())
    }

    #[test]
    fn parses_the_view() {
        let bytes = fixture_lists(&["vanilla", "file/a.zip"], &["trade_rebalance"]);
        let view = parse(&bytes).unwrap();
        assert_eq!(view.version_name, "1.21.10");
        assert_eq!(view.level_name.as_deref(), Some("Test World"));
        assert_eq!(view.enabled, vec!["vanilla", "file/a.zip"]);
        assert_eq!(view.disabled, vec!["trade_rebalance"]);
    }

    #[test]
    fn toggle_moves_between_lists() {
        let bytes = fixture_lists(&["vanilla", "file/a.zip"], &[]);
        let off = set_enabled(&bytes, "a.zip", false).unwrap();
        let view = parse(&off).unwrap();
        assert_eq!(view.enabled, vec!["vanilla"]);
        assert_eq!(view.disabled, vec!["file/a.zip"]);
        let on = set_enabled(&off, "a.zip", true).unwrap();
        let view = parse(&on).unwrap();
        assert_eq!(view.enabled, vec!["vanilla", "file/a.zip"]);
        assert!(view.disabled.is_empty());
    }

    #[test]
    fn disable_removes_every_duplicate_and_enable_is_idempotent() {
        let bytes = fixture_lists(&["file/a.zip", "vanilla", "file/a.zip"], &["file/a.zip"]);
        let off = set_enabled(&bytes, "a.zip", false).unwrap();
        let view = parse(&off).unwrap();
        assert_eq!(view.enabled, vec!["vanilla"]);
        assert_eq!(view.disabled, vec!["file/a.zip"]);
        let on = set_enabled(&off, "a.zip", true).unwrap();
        let again = set_enabled(&on, "a.zip", true).unwrap();
        assert_eq!(parse(&again).unwrap().enabled.len(), 2);
    }

    #[test]
    fn unknown_tags_survive_a_toggle() {
        let bytes = fixture_lists(&["vanilla"], &[]);
        let toggled = set_enabled(&bytes, "a.zip", true).unwrap();
        assert_outside_datapacks_unchanged(&bytes, &toggled).unwrap();
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            parse(b"not gzip"),
            Err(LevelDatError::CorruptGzip { .. })
        ));
        let not_compound = gzip(&[0x01, 0x00, 0x00, 0x2a]);
        assert_eq!(parse(&not_compound), Err(LevelDatError::NotCompound));
        let no_version = gzip(
            &fastnbt::to_bytes(&compound(vec![("Data", compound(vec![]))])).unwrap(),
        );
        assert_eq!(parse(&no_version), Err(LevelDatError::MissingVersion));
        let no_packs = gzip(
            &fastnbt::to_bytes(&compound(vec![(
                "Data",
                compound(vec![(
                    "Version",
                    compound(vec![("Name", Value::String("1.21.10".to_string()))]),
                )]),
            )]))
            .unwrap(),
        );
        assert!(matches!(
            parse(&no_packs),
            Err(LevelDatError::MissingDataPacks { .. })
        ));
    }
}
