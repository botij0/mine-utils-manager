use std::io::Write;
use std::path::Path;
use std::process::Command;

use mine_utils_core::{pack_import, HashCache, IconCache, ImportChoice, ImportOutcome};

fn zip_pack(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("pack.mcmeta", options).unwrap();
    zip.write_all(br#"{"pack":{"pack_format":34,"description":"Faithful"}}"#)
        .unwrap();
    zip.start_file("assets/test.txt", options).unwrap();
    zip.write_all(b"original assets").unwrap();
    zip.finish().unwrap();
}

#[test]
fn system_trash_replace_preserves_recoverable_names() {
    for name in ["Faithful-1.21.zip", "Faithful.zip"] {
        let root = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "system_trash_fixture", "--nocapture"])
            .env("MINE_UTILS_TRASH_FIXTURE", root.path())
            .env("MINE_UTILS_TRASH_FILENAME", name)
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .output()
            .unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn system_trash_fixture() {
    let Some(root) = std::env::var_os("MINE_UTILS_TRASH_FIXTURE") else {
        return;
    };
    let root = Path::new(&root);
    let filename = std::env::var("MINE_UTILS_TRASH_FILENAME").unwrap();
    let packs = root.join("game/resourcepacks");
    let old = packs.join("Faithful.zip");
    zip_pack(&old);
    let before = std::fs::read(&old).unwrap();
    let source = root.join("incoming").join(&filename);
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::copy(&old, &source).unwrap();
    let data_home = root.join("xdg-data");
    std::fs::create_dir_all(data_home.join("Trash/files")).unwrap();
    let result = pack_import::import_pack(
        &source,
        &packs,
        &IconCache::new(&root.join("cache")),
        ImportChoice::Replace,
        &HashCache::new(),
    )
    .unwrap();
    assert!(matches!(result, ImportOutcome::Imported { .. }));
    let entries: Vec<_> = std::fs::read_dir(data_home.join("Trash/files"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1);
    let trashed = &entries[0];
    let name = trashed.file_name().unwrap().to_str().unwrap();
    let original_path = if filename == "Faithful.zip" {
        assert!(name.starts_with(".mine-utils-replace."), "{name}");
        assert!(name.ends_with(".tmp"), "{name}");
        packs.join(name)
    } else {
        assert_eq!(name, "Faithful.zip");
        old
    };
    let trashed_pack = if filename == "Faithful.zip" {
        trashed.join("Faithful.zip")
    } else {
        trashed.to_owned()
    };
    assert_eq!(std::fs::read(trashed_pack).unwrap(), before);
    assert_eq!(std::fs::read(packs.join(filename)).unwrap(), before);
    let info = std::fs::read_to_string(
        data_home
            .join("Trash/info")
            .join(format!("{name}.trashinfo")),
    )
    .unwrap();
    assert!(
        info.lines()
            .any(|line| line == format!("Path={}", original_path.display())),
        "{info}"
    );
    println!("trash entry {name}; {info}");
}
