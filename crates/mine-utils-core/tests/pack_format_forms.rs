use std::io::Write as _;
use std::path::Path;

use mine_utils_core::{scan_packs, IconCache};

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

fn scan(packs: &[(&str, &str, &str)]) -> Vec<(String, serde_json::Value, Option<String>)> {
    let dir = tempfile::tempdir().unwrap();
    let resourcepacks = dir.path().join("resourcepacks");
    std::fs::create_dir_all(&resourcepacks).unwrap();
    for (name, mcmeta, root) in packs {
        write_zip(&resourcepacks.join(name), mcmeta, root);
    }
    let icons = IconCache::new(&dir.path().join("cache"));
    let listing = scan_packs(&resourcepacks, &icons).unwrap();
    listing
        .packs
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
