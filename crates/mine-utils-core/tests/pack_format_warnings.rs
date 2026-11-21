use std::io::Write as _;
use std::path::Path;

use mine_utils_core::{scan_packs, IconCache, Pack, PackWarningCode};

const DATA_LOOSE: &str = r#"{"pack":{"description":"d","pack_format":82}}"#;
const RESOURCE_LOOSE: &str = r#"{"pack":{"description":"d","pack_format":65}}"#;

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

/// Goes through the public `scan_packs` entry that the desktop app calls.
fn scan_one(mcmeta: &str, root: &str, folder: bool) -> Pack {
    let dir = tempfile::tempdir().unwrap();
    let resourcepacks = dir.path().join("resourcepacks");
    std::fs::create_dir_all(&resourcepacks).unwrap();
    if folder {
        let pack = resourcepacks.join("Pack");
        std::fs::create_dir_all(pack.join(root)).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), mcmeta).unwrap();
        std::fs::write(pack.join(root).join("x.json"), b"{}").unwrap();
    } else {
        write_zip(&resourcepacks.join("Pack.zip"), mcmeta, root);
    }
    let icons = IconCache::new(&dir.path().join("cache"));
    let mut packs = scan_packs(&resourcepacks, &icons).unwrap().packs;
    assert_eq!(packs.len(), 1);
    packs.remove(0)
}

fn warnings(pack: &Pack) -> Vec<(PackWarningCode, &str)> {
    pack.warnings
        .iter()
        .map(|warning| (warning.code, warning.message.as_str()))
        .collect()
}

#[test]
fn a_data_pack_with_only_pack_format_82_warns_about_both_fields() {
    let pack = scan_one(DATA_LOOSE, "data", false);
    assert_eq!(pack.name, "Pack");
    assert_eq!(
        warnings(&pack),
        vec![(
            PackWarningCode::MissingMinMaxFormat,
            "pack_format 82 needs min_format and max_format, but pack.mcmeta is missing min_format and max_format.",
        )]
    );
}

#[test]
fn a_resource_pack_with_only_pack_format_65_warns_about_both_fields() {
    let pack = scan_one(RESOURCE_LOOSE, "assets", false);
    assert_eq!(
        warnings(&pack),
        vec![(
            PackWarningCode::MissingMinMaxFormat,
            "pack_format 65 needs min_format and max_format, but pack.mcmeta is missing min_format and max_format.",
        )]
    );
}

#[test]
fn a_folder_pack_with_only_pack_format_82_warns_too() {
    let pack = scan_one(DATA_LOOSE, "data", true);
    assert_eq!(
        warnings(&pack),
        vec![(
            PackWarningCode::MissingMinMaxFormat,
            "pack_format 82 needs min_format and max_format, but pack.mcmeta is missing min_format and max_format.",
        )]
    );
}

#[test]
fn a_partial_declaration_names_only_the_missing_field() {
    let pack = scan_one(
        r#"{"pack":{"description":"d","pack_format":82,"min_format":82}}"#,
        "data",
        false,
    );
    assert_eq!(
        warnings(&pack),
        vec![(
            PackWarningCode::MissingMinMaxFormat,
            "pack_format 82 needs min_format and max_format, but pack.mcmeta is missing max_format.",
        )]
    );
}

#[test]
fn formats_just_below_the_threshold_stay_warning_free() {
    let data_81 = scan_one(
        r#"{"pack":{"description":"d","pack_format":81}}"#,
        "data",
        false,
    );
    let resource_64 = scan_one(
        r#"{"pack":{"description":"d","pack_format":64}}"#,
        "assets",
        false,
    );
    assert!(data_81.warnings.is_empty());
    assert!(resource_64.warnings.is_empty());
}

#[test]
fn declaring_min_and_max_formats_stays_warning_free() {
    let pack = scan_one(
        r#"{"pack":{"description":"d","pack_format":82,"min_format":82,"max_format":88}}"#,
        "data",
        false,
    );
    assert!(pack.warnings.is_empty());
    assert!(pack.error.is_none());
}
