use std::collections::HashMap;
use std::path::Path;

use fastnbt::Value;
use mine_utils_core::level_dat;

fn real_level_path() -> Option<String> {
    std::env::var("MINE_UTILS_REAL_LEVEL").ok()
}

fn compound(pairs: Vec<(&str, Value)>) -> Value {
    Value::Compound(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn strings(entries: &[&str]) -> Value {
    Value::List(
        entries
            .iter()
            .map(|e| Value::String(e.to_string()))
            .collect(),
    )
}

fn fixture(
    version: &str,
    enabled: &[&str],
    disabled: &[&str],
    extra_data: Vec<(&str, Value)>,
) -> Vec<u8> {
    let mut data: HashMap<String, Value> = HashMap::new();
    data.insert(
        "Version".to_string(),
        compound(vec![("Name", Value::String(version.to_string()))]),
    );
    data.insert(
        "DataPacks".to_string(),
        compound(vec![
            ("Enabled", strings(enabled)),
            ("Disabled", strings(disabled)),
        ]),
    );
    data.insert(
        "LevelName".to_string(),
        Value::String("Fixture".to_string()),
    );
    // Unknown tags the write must preserve: a long seed, a nested compound,
    // and a list of ints.
    data.insert("Seed".to_string(), Value::Long(8675309));
    data.insert(
        "WorldGenSettings".to_string(),
        compound(vec![
            ("Seed", Value::Long(42)),
            ("GenerateFeatures", Value::Byte(1)),
        ]),
    );
    data.insert(
        "PlayerPos".to_string(),
        Value::List(vec![Value::Double(1.5), Value::Double(64.0)]),
    );
    for (key, value) in extra_data {
        data.insert(key.to_string(), value);
    }
    let root = Value::Compound(
        [("Data".to_string(), Value::Compound(data))]
            .into_iter()
            .collect(),
    );
    level_dat::gzip(&fastnbt::to_bytes(&root).unwrap())
}

fn lists(version: &str) -> Vec<u8> {
    fixture(
        version,
        &["vanilla", "file/a.zip"],
        &["trade_rebalance"],
        Vec::new(),
    )
}

#[test]
fn toggle_preserves_every_tag_outside_datapacks() {
    for version in ["1.17.1", "1.21.10"] {
        let bytes = lists(version);
        let off = level_dat::set_enabled(&bytes, "a.zip", false).unwrap();
        let view = level_dat::parse(&off).unwrap();
        assert_eq!(view.enabled, vec!["vanilla".to_string()]);
        assert_eq!(
            view.disabled,
            vec!["trade_rebalance".to_string(), "file/a.zip".to_string()]
        );
        level_dat::assert_outside_datapacks_unchanged(&bytes, &off).unwrap();

        let on = level_dat::set_enabled(&off, "a.zip", true).unwrap();
        let view = level_dat::parse(&on).unwrap();
        assert_eq!(
            view.enabled,
            vec!["vanilla".to_string(), "file/a.zip".to_string()]
        );
        assert!(view.disabled == vec!["trade_rebalance".to_string()]);
        level_dat::assert_outside_datapacks_unchanged(&bytes, &on).unwrap();
    }
}

#[test]
fn changed_tags_are_detected() {
    let before = lists("1.21.10");
    let view = level_dat::parse(&before).unwrap();
    assert_eq!(view.version_name, "1.21.10");
    let tampered = {
        let raw = level_dat::gunzip(&before).unwrap();
        let mut value = level_dat::parse_nbt(&raw).unwrap();
        let Value::Compound(root) = &mut value else {
            panic!("root")
        };
        let Some(Value::Compound(data)) = root.get_mut("Data") else {
            panic!("data")
        };
        data.insert("Seed".to_string(), Value::Long(1));
        level_dat::gzip(&fastnbt::to_bytes(&value).unwrap())
    };
    assert!(level_dat::assert_outside_datapacks_unchanged(&before, &tampered).is_err());
}

#[test]
fn corrupt_gzip_is_refused() {
    let err = level_dat::parse(b"not gzip at all").unwrap_err();
    assert!(matches!(err, level_dat::LevelDatError::CorruptGzip { .. }));
}

#[test]
fn non_compound_root_is_refused() {
    let bytes = level_dat::gzip(&[0x01, 0x00, 0x00, 0x2a]);
    assert_eq!(
        level_dat::parse(&bytes).unwrap_err(),
        level_dat::LevelDatError::NotCompound
    );
}

#[test]
fn missing_version_is_an_error() {
    let mut data: HashMap<String, Value> = HashMap::new();
    data.insert(
        "DataPacks".to_string(),
        compound(vec![("Enabled", strings(&[])), ("Disabled", strings(&[]))]),
    );
    let root = Value::Compound(
        [("Data".to_string(), Value::Compound(data))]
            .into_iter()
            .collect(),
    );
    let bytes = level_dat::gzip(&fastnbt::to_bytes(&root).unwrap());
    assert_eq!(
        level_dat::parse(&bytes).unwrap_err(),
        level_dat::LevelDatError::MissingVersion
    );
}

#[test]
fn missing_datapacks_is_an_error() {
    let bytes = fixture("1.21.10", &[], &[], Vec::new());
    let raw = level_dat::gunzip(&bytes).unwrap();
    let mut value = level_dat::parse_nbt(&raw).unwrap();
    let Value::Compound(root) = &mut value else {
        panic!("root")
    };
    let Some(Value::Compound(data)) = root.get_mut("Data") else {
        panic!("data")
    };
    data.remove("DataPacks");
    let bytes = level_dat::gzip(&fastnbt::to_bytes(&value).unwrap());
    assert!(matches!(
        level_dat::parse(&bytes).unwrap_err(),
        level_dat::LevelDatError::MissingDataPacks { .. }
    ));
}

#[test]
fn missing_enabled_or_disabled_is_an_error() {
    for missing in ["Enabled", "Disabled"] {
        let bytes = fixture("1.21.10", &["vanilla"], &[], Vec::new());
        let raw = level_dat::gunzip(&bytes).unwrap();
        let mut value = level_dat::parse_nbt(&raw).unwrap();
        let Value::Compound(root) = &mut value else {
            panic!("root")
        };
        let Some(Value::Compound(data)) = root.get_mut("Data") else {
            panic!("data")
        };
        let Some(Value::Compound(packs)) = data.get_mut("DataPacks") else {
            panic!("packs")
        };
        packs.remove(missing);
        let bytes = level_dat::gzip(&fastnbt::to_bytes(&value).unwrap());
        assert!(
            matches!(
                level_dat::parse(&bytes).unwrap_err(),
                level_dat::LevelDatError::MissingDataPacks { .. }
            ),
            "{missing} must be required"
        );
    }
}

#[test]
fn duplicates_are_preserved_on_read_and_removed_on_disable() {
    let bytes = fixture(
        "1.21.10",
        &["vanilla", "file/a.zip", "file/a.zip"],
        &[],
        Vec::new(),
    );
    let view = level_dat::parse(&bytes).unwrap();
    assert_eq!(view.enabled.len(), 3, "duplicates survive a read");
    let off = level_dat::set_enabled(&bytes, "a.zip", false).unwrap();
    let view = level_dat::parse(&off).unwrap();
    assert_eq!(view.enabled, vec!["vanilla".to_string()]);
    level_dat::assert_outside_datapacks_unchanged(&bytes, &off).unwrap();
}

#[test]
fn foreign_entries_keep_their_relative_order() {
    let bytes = fixture(
        "1.21.10",
        &[
            "vanilla",
            "file/a.zip",
            "fabric",
            "fabric-convention-tags-v2",
        ],
        &["minecart_improvements"],
        Vec::new(),
    );
    let on = level_dat::set_enabled(&bytes, "b.zip", true).unwrap();
    let view = level_dat::parse(&on).unwrap();
    assert_eq!(
        view.enabled,
        vec![
            "vanilla",
            "file/a.zip",
            "fabric",
            "fabric-convention-tags-v2",
            "file/b.zip"
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
    assert_eq!(view.disabled, vec!["minecart_improvements".to_string()]);
    level_dat::assert_outside_datapacks_unchanged(&bytes, &on).unwrap();
}

#[test]
fn real_copy_fail_first_opt_in() {
    let Some(path) = real_level_path() else {
        return;
    };
    let bytes = std::fs::read(Path::new(&path)).unwrap_or_else(|_| {
        panic!(
            "real level.dat copy missing at {path}; set MINE_UTILS_REAL_LEVEL or copy the fixture"
        )
    });
    let view = level_dat::parse(&bytes).expect("real level.dat must parse");
    assert_eq!(view.version_name, "1.21.10");
    assert_eq!(
        view.enabled,
        vec![
            "vanilla",
            "file/armored elytra v1.0.4 (MC 1.17.x).zip",
            "file/double shulker shells v1.3.1 (MC 1.17.x).zip",
            "file/dragon drops v1.3.1 (MC 1.17.x).zip",
            "file/fast leaf decay v2.0.4 (MC 1.17.x).zip",
            "file/graves v2.8.5 (MC 1.17.x).zip",
            "fabric",
            "fabric-convention-tags-v2",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
    assert_eq!(
        view.disabled,
        vec![
            "minecart_improvements",
            "redstone_experiments",
            "trade_rebalance"
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );

    let toggled = level_dat::set_enabled(&bytes, "graves v2.8.5 (MC 1.17.x).zip", false)
        .expect("toggle off must succeed");
    let after = level_dat::parse(&toggled).expect("toggled bytes must parse");
    assert!(!after.enabled.iter().any(|e| e.contains("graves")));
    assert!(after
        .disabled
        .contains(&"file/graves v2.8.5 (MC 1.17.x).zip".to_string()));

    level_dat::assert_outside_datapacks_unchanged(&bytes, &toggled)
        .expect("every tag outside Data.DataPacks must be unchanged");
}
