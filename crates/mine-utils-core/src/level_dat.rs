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
    #[error("level.dat holds malformed NBT: {reason}")]
    MalformedNbt { reason: String },
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
        self.error_code().as_str()
    }

    pub fn error_code(&self) -> crate::ErrorCode {
        match self {
            LevelDatError::StaleFile => crate::ErrorCode::StaleStack,
            _ => crate::ErrorCode::LevelDat,
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

pub(crate) const MAX_LEVEL_DAT_BYTES: u64 = 64 * 1024 * 1024;

/// Decompress one `level.dat` file into its raw NBT bytes.
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, LevelDatError> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut raw = Vec::new();
    decoder
        .take(MAX_LEVEL_DAT_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(corrupt)?;
    if raw.len() as u64 > MAX_LEVEL_DAT_BYTES {
        return Err(corrupt("decompressed level.dat is too large"));
    }
    Ok(raw)
}

/// Compress raw NBT bytes the way Minecraft stores them.
pub fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).expect("gzip to memory never fails");
    encoder.finish().expect("gzip to memory never fails")
}

/// Parse decompressed NBT bytes to a value tree. The root must be a compound.
/// A truncated or otherwise unparseable payload is malformed NBT; only a
/// parsed non-compound root is `NotCompound`.
pub fn parse_nbt(raw: &[u8]) -> Result<Value, LevelDatError> {
    let value: Value = fastnbt::from_bytes(raw).map_err(|err| LevelDatError::MalformedNbt {
        reason: err.to_string(),
    })?;
    match &value {
        Value::Compound(_) => Ok(value),
        _ => Err(LevelDatError::NotCompound),
    }
}

/// Parse a whole `level.dat` file into the UI view.
pub fn parse(bytes: &[u8]) -> Result<LevelDatView, LevelDatError> {
    let raw = gunzip(bytes)?;
    let value = parse_nbt(&raw)?;
    view_of(&value, lists_of)
}

/// Like [`parse`], but a file with no `Data.DataPacks` reads as empty lists.
/// Minecraft before 1.13 wrote none, so such a file holds no datapacks. A
/// present but malformed `DataPacks` still fails.
pub fn parse_lenient(bytes: &[u8]) -> Result<LevelDatView, LevelDatError> {
    let raw = gunzip(bytes)?;
    let value = parse_nbt(&raw)?;
    view_of(&value, lists_or_empty)
}

pub(crate) fn world_seed(bytes: &[u8]) -> Option<i64> {
    let raw = gunzip(bytes).ok()?;
    let value = parse_nbt(&raw).ok()?;
    let data = data_of(&value).ok()?;
    let modern = match data.get("WorldGenSettings") {
        Some(Value::Compound(settings)) => settings.get("seed"),
        _ => None,
    };
    match modern {
        Some(Value::Long(seed)) => Some(*seed),
        _ => match data.get("RandomSeed") {
            Some(Value::Long(seed)) => Some(*seed),
            _ => None,
        },
    }
}

fn data_of(value: &Value) -> Result<&std::collections::HashMap<String, Value>, LevelDatError> {
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

type ListsReader = fn(
    &std::collections::HashMap<String, Value>,
) -> Result<(Vec<String>, Vec<String>), LevelDatError>;

fn view_of(value: &Value, read_lists: ListsReader) -> Result<LevelDatView, LevelDatError> {
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
    let (enabled, disabled) = read_lists(data)?;
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

fn lists_or_empty(
    data: &std::collections::HashMap<String, Value>,
) -> Result<(Vec<String>, Vec<String>), LevelDatError> {
    if data.contains_key("DataPacks") {
        lists_of(data)
    } else {
        Ok((Vec::new(), Vec::new()))
    }
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
    let encoded = fastnbt::to_bytes(&value).map_err(|err| LevelDatError::MalformedArrays {
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

/// Fail when any tag outside `Data.DataPacks.Enabled`/`Disabled` differs
/// between two files. Only the two lists are masked (each replaced with a
/// placeholder); every other tag, including any third key inside `DataPacks`,
/// is compared. The comparison runs on the parsed value tree, so gzip bytes
/// may differ.
pub fn assert_outside_datapacks_unchanged(
    before: &[u8],
    after: &[u8],
) -> Result<(), LevelDatError> {
    let before_raw = gunzip(before)?;
    let after_raw = gunzip(after)?;
    let mut before_value = parse_nbt(&before_raw)?;
    let mut after_value = parse_nbt(&after_raw)?;
    mask_datapack_lists(&mut before_value)?;
    mask_datapack_lists(&mut after_value)?;
    if before_value == after_value {
        Ok(())
    } else {
        Err(LevelDatError::MalformedArrays {
            reason: "a tag outside Data.DataPacks.Enabled/Disabled changed".to_string(),
        })
    }
}

fn mask_datapack_lists(value: &mut Value) -> Result<(), LevelDatError> {
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
    for key in ["Enabled", "Disabled"] {
        if packs.contains_key(key) {
            packs.insert(key.to_string(), Value::String("<masked>".to_string()));
        }
    }
    Ok(())
}

/// Write `level.dat` atomically: temp file in the world dir, `sync_all`,
/// rename, directory `fsync`. A failed write leaves the previous file intact,
/// and `level.dat_old` is never touched.
///
/// The temp name carries the pid, a nanosecond timestamp, and a per-process
/// counter, so a crashed write plus pid reuse can never wedge a later write:
/// a leftover temp never collides with a fresh name, and `create_new`
/// collisions are retried with a fresh stamp.
pub fn write_level_dat(target: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let world_dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..64 {
        let tmp = unique_tmp(world_dir);
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, target)?;
            Ok(())
        })();
        match written {
            Ok(()) => {
                if let Ok(handle) = std::fs::File::open(world_dir) {
                    let _ = handle.sync_all();
                }
                return Ok(());
            }
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    last_err = Some(err);
                    continue;
                }
                if let Ok(handle) = std::fs::File::open(world_dir) {
                    let _ = handle.sync_all();
                }
                return Err(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not find a free level.dat temp name",
        )
    }))
}

fn unique_tmp(world_dir: &Path) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::SeqCst);
    world_dir.join(format!(
        ".level.dat.{}.{}.{}.tmp",
        std::process::id(),
        nanos,
        count
    ))
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
                                enabled
                                    .iter()
                                    .map(|e| Value::String(e.to_string()))
                                    .collect(),
                            ),
                        ),
                        (
                            "Disabled",
                            Value::List(
                                disabled
                                    .iter()
                                    .map(|e| Value::String(e.to_string()))
                                    .collect(),
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
    fn render_with_replaces_only_the_lists() {
        let bytes = fixture_lists(&["vanilla", "file/a.zip"], &[]);
        let off = render_with(
            &bytes,
            &["vanilla".to_string()],
            &["file/a.zip".to_string()],
        )
        .unwrap();
        let view = parse(&off).unwrap();
        assert_eq!(view.enabled, vec!["vanilla"]);
        assert_eq!(view.disabled, vec!["file/a.zip"]);
        assert_outside_datapacks_unchanged(&bytes, &off).unwrap();
    }

    #[test]
    fn unknown_tags_survive_a_render() {
        let bytes = fixture_lists(&["vanilla"], &[]);
        let toggled = render_with(
            &bytes,
            &["vanilla".to_string(), "file/a.zip".to_string()],
            &[],
        )
        .unwrap();
        assert_outside_datapacks_unchanged(&bytes, &toggled).unwrap();
    }

    #[test]
    fn third_datapack_key_is_compared_not_erased() {
        let with_extra = |extra: i32| {
            let value = compound(vec![(
                "Data",
                compound(vec![
                    (
                        "Version",
                        compound(vec![("Name", Value::String("1.21.10".to_string()))]),
                    ),
                    (
                        "DataPacks",
                        compound(vec![
                            ("Enabled", Value::List(vec![])),
                            ("Disabled", Value::List(vec![])),
                            ("Third", Value::Int(extra)),
                        ]),
                    ),
                ]),
            )]);
            gzip(&fastnbt::to_bytes(&value).unwrap())
        };
        let before = with_extra(1);
        let same = with_extra(1);
        assert_outside_datapacks_unchanged(&before, &same).unwrap();
        let changed = with_extra(2);
        assert!(assert_outside_datapacks_unchanged(&before, &changed).is_err());
    }

    #[test]
    fn truncated_nbt_is_malformed_not_not_compound() {
        let bytes = fixture_lists(&["vanilla"], &[]);
        let raw = gunzip(&bytes).unwrap();
        let truncated = gzip(&raw[..raw.len() / 2]);
        let err = parse(&truncated).unwrap_err();
        assert!(
            matches!(err, LevelDatError::MalformedNbt { .. }),
            "truncated payload must be malformed NBT, got {err:?}"
        );
        // fastnbt refuses to encode a non-compound root, so the NotCompound
        // arm stays defensive: a bare Byte tag decodes as malformed NBT.
        let bare_tag = gzip(&[0x01, 0x00, 0x00, 0x2a]);
        assert!(matches!(
            parse(&bare_tag),
            Err(LevelDatError::MalformedNbt { .. })
        ));
    }

    #[test]
    fn write_uses_the_supplied_target_path() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("chosen.dat");
        write_level_dat(&target, b"chosen bytes").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"chosen bytes");
        assert!(!dir.path().join("level.dat").exists());
    }

    #[test]
    fn write_is_atomic_and_leaves_old_temp_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("world");
        std::fs::create_dir_all(&world).unwrap();
        // A crashed write with pid reuse leaves this behind; it must not block.
        std::fs::write(
            world.join(format!(".level.dat.{}.tmp", std::process::id())),
            b"stale",
        )
        .unwrap();
        std::fs::write(world.join("level.dat_old"), b"old stays").unwrap();
        let first = fixture_lists(&["vanilla"], &[]);
        write_level_dat(&world.join("level.dat"), &first).unwrap();
        assert_eq!(std::fs::read(world.join("level.dat")).unwrap(), first);
        assert_eq!(
            std::fs::read(world.join("level.dat_old")).unwrap(),
            b"old stays"
        );
        let second = fixture_lists(&["vanilla", "file/a.zip"], &["x"]);
        write_level_dat(&world.join("level.dat"), &second).unwrap();
        assert_eq!(std::fs::read(world.join("level.dat")).unwrap(), second);
        assert_eq!(
            std::fs::read(world.join("level.dat_old")).unwrap(),
            b"old stays"
        );
        assert_outside_datapacks_unchanged(&first, &second).unwrap();
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            parse(b"not gzip"),
            Err(LevelDatError::CorruptGzip { .. })
        ));
        let bare_tag = gzip(&[0x01, 0x00, 0x00, 0x2a]);
        assert!(matches!(
            parse(&bare_tag),
            Err(LevelDatError::MalformedNbt { .. })
        ));
        let no_version =
            gzip(&fastnbt::to_bytes(&compound(vec![("Data", compound(vec![]))])).unwrap());
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
