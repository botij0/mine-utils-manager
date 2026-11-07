use std::io::Write as _;
use std::path::Path;

use mine_utils_core::{data_format_for_version, scan_packs, FormatRange, IconCache, Pack};

fn write_zip(path: &Path, mcmeta: &str, root: &str) {
    let file = std::fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("pack.mcmeta", options).unwrap();
    writer.write_all(mcmeta.as_bytes()).unwrap();
    writer
        .start_file(format!("{root}/x.json"), options)
        .unwrap();
    writer.write_all(b"{}").unwrap();
    writer.finish().unwrap();
}

fn scan_listing(packs: &[(&str, &str, &str)]) -> Vec<Pack> {
    let dir = tempfile::tempdir().unwrap();
    let resourcepacks = dir.path().join("resourcepacks");
    std::fs::create_dir_all(&resourcepacks).unwrap();
    for (name, mcmeta, root) in packs {
        write_zip(&resourcepacks.join(name), mcmeta, root);
    }
    let icons = IconCache::new(&dir.path().join("cache"));
    scan_packs(&resourcepacks, &icons).unwrap().packs
}

fn scan(packs: &[(&str, &str, &str)]) -> Vec<(String, serde_json::Value, Option<String>)> {
    scan_listing(packs)
        .iter()
        .map(|pack| {
            (
                pack.name.clone(),
                serde_json::to_value(pack.format_range).unwrap(),
                pack.game_label.clone(),
            )
        })
        .collect()
}

#[test]
fn integer_min_and_max_formats_label_the_whole_data_range() {
    let packs = scan(&[(
        "ints.zip",
        r#"{"pack":{"description":"d","min_format":88,"max_format":94}}"#,
        "data",
    )]);
    assert_eq!(
        packs,
        vec![(
            "ints".to_string(),
            serde_json::json!({
                "min": { "major": 88, "minor": 0 },
                "max": { "major": 94, "minor": 2147483647 },
            }),
            Some("1.21.9 to 1.21.11".to_string()),
        )]
    );
}

#[test]
fn one_element_min_format_keeps_its_minimum() {
    let packs = scan(&[(
        "one.zip",
        r#"{"pack":{"description":"d","min_format":[82],"max_format":[94]}}"#,
        "data",
    )]);
    assert_eq!(
        packs[0].1,
        serde_json::json!({
            "min": { "major": 82, "minor": 0 },
            "max": { "major": 94, "minor": 2147483647 },
        })
    );
}

#[test]
fn integer_supported_formats_is_one_major_covering_its_minors() {
    let packs = scan(&[(
        "supported.zip",
        r#"{"pack":{"description":"d","supported_formats":94}}"#,
        "data",
    )]);
    assert_eq!(
        packs[0].1,
        serde_json::json!({
            "min": { "major": 94, "minor": 0 },
            "max": { "major": 94, "minor": 2147483647 },
        })
    );
    assert_eq!(packs[0].2.as_deref(), Some("1.21.11"));
}

fn frontend_admits(range: Option<FormatRange>, world: &str) -> bool {
    let world = data_format_for_version(world).unwrap();
    let range = range.unwrap();
    range.min.is_none_or(|min| min <= world) && range.max.is_none_or(|max| world <= max)
}

#[test]
fn a_label_never_names_a_version_the_range_rejects() {
    let packs = scan_listing(&[
        (
            "a_legacy_94.zip",
            r#"{"pack":{"description":"d","pack_format":94}}"#,
            "data",
        ),
        (
            "b_pinned_94_0.zip",
            r#"{"pack":{"description":"d","min_format":[94,0],"max_format":[94,0]}}"#,
            "data",
        ),
        (
            "c_whole_94.zip",
            r#"{"pack":{"description":"d","min_format":94,"max_format":94}}"#,
            "data",
        ),
        (
            "d_legacy_101.zip",
            r#"{"pack":{"description":"d","pack_format":101}}"#,
            "data",
        ),
    ]);
    let seen: Vec<_> = packs
        .iter()
        .zip(["1.21.11", "1.21.11", "1.21.11", "26.1"])
        .map(|(pack, world)| {
            (
                pack.name.as_str(),
                pack.game_label.as_deref(),
                world,
                frontend_admits(pack.format_range, world),
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            ("a_legacy_94", None, "1.21.11", false),
            ("b_pinned_94_0", None, "1.21.11", false),
            ("c_whole_94", Some("1.21.11"), "1.21.11", true),
            ("d_legacy_101", None, "26.1", false),
        ]
    );
}

#[test]
fn pack_format_never_carries_the_any_minor_marker() {
    let packs = scan_listing(&[
        (
            "a_max_only.zip",
            r#"{"pack":{"description":"d","max_format":88}}"#,
            "data",
        ),
        (
            "b_max_minor.zip",
            r#"{"pack":{"description":"d","max_format":[88,1]}}"#,
            "data",
        ),
        (
            "c_legacy.zip",
            r#"{"pack":{"description":"d","pack_format":94}}"#,
            "data",
        ),
    ]);
    let formats: Vec<_> = packs
        .iter()
        .map(|pack| {
            (
                pack.name.as_str(),
                serde_json::to_value(pack.format).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        formats,
        vec![
            ("a_max_only", serde_json::json!({ "major": 88, "minor": 0 })),
            (
                "b_max_minor",
                serde_json::json!({ "major": 88, "minor": 1 })
            ),
            ("c_legacy", serde_json::json!({ "major": 94, "minor": 0 })),
        ]
    );
}
