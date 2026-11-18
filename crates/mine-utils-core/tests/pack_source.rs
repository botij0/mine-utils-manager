use std::io::Write;
use std::path::Path;

use mine_utils_core::data_stack::{self, DataPaths};
use mine_utils_core::pack::{PackError, PackLocation};
use mine_utils_core::pack_import::TrashService;
use mine_utils_core::resource_stack::WriteGuard;
use mine_utils_core::{scan_packs, AppError, BackupsConfig, HashCache, IconCache, ImportChoice};

const META: &[u8] = br#"{"pack":{"pack_format":61,"description":"source"}}"#;

fn zip(path: &Path, prefix: &str, meta: &[u8]) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in [
        ("pack.mcmeta", meta),
        ("data/test/a.json", b"{}".as_slice()),
    ] {
        writer
            .start_file(
                format!("{prefix}{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn folder(path: &Path, meta: &[u8]) {
    std::fs::create_dir_all(path.join("data/test")).unwrap();
    std::fs::write(path.join("pack.mcmeta"), meta).unwrap();
    std::fs::write(path.join("data/test/a.json"), b"{}").unwrap();
}

struct Services;
impl WriteGuard for Services {
    fn ensure_writable(&self, _: &Path) -> Result<(), AppError> {
        Ok(())
    }
}
impl TrashService for Services {
    fn move_to_trash(&self, _: &Path) -> Result<(), PackError> {
        panic!("a fresh import must not trash anything")
    }
}

fn import_result(
    source: &Path,
    root: &Path,
) -> Result<data_stack::DatapackImportOutcome, AppError> {
    let world = root.join("game/saves/test");
    std::fs::create_dir_all(world.join("datapacks")).unwrap();
    let data = serde_json::json!({"Data":{"Version":{"Name":"1.21.4"},"DataPacks":{"Enabled":["vanilla"],"Disabled":[]}}});
    let nbt = fastnbt::to_bytes(&data).unwrap();
    std::fs::write(
        world.join("level.dat"),
        mine_utils_core::level_dat::gzip(&nbt),
    )
    .unwrap();
    let paths = DataPaths::for_world(
        &mine_utils_core::WorldPaths::for_world(&root.join("game"), "test").unwrap(),
        &root.join("app-data"),
        BackupsConfig {
            enabled: false,
            keep: 10,
        },
    );
    data_stack::import_datapack_with(
        &paths,
        source,
        ImportChoice::Default,
        &IconCache::new(&root.join("icons")),
        &HashCache::new(),
        &Services,
        &Services,
    )
}

fn import(source: &Path, root: &Path) -> String {
    match import_result(source, root) {
        Ok(data_stack::DatapackImportOutcome::Imported { .. }) => "imported".into(),
        Ok(other) => format!("{other:?}"),
        Err(err) => err.code().into(),
    }
}

#[test]
fn zip_and_folder_metadata_share_the_cap() {
    let root = tempfile::tempdir().unwrap();
    let icons = IconCache::new(&root.path().join("icons"));
    let packs = root.path().join("packs");
    std::fs::create_dir(&packs).unwrap();
    for size in [1024 * 1024, 1024 * 1024 + 1] {
        let mut meta = META.to_vec();
        meta.resize(size, b' ');
        zip(&packs.join("Zip.zip"), "", &meta);
        folder(&packs.join("Dir"), &meta);
        let listing = scan_packs(&packs, &icons).unwrap();
        let codes: Vec<_> = listing
            .packs
            .iter()
            .map(|p| p.error.as_ref().map(|e| e.code.as_str()))
            .collect();
        if size == 1024 * 1024 {
            assert_eq!(codes, vec![None, None]);
        } else {
            assert_eq!(
                codes,
                vec![Some("pack_broken_archive"), Some("pack_broken_archive")]
            );
        }
    }
}

#[test]
fn dotted_zip_metadata_imports_like_plain_metadata() {
    let root = tempfile::tempdir().unwrap();
    for prefix in ["", "./"] {
        let source = root.path().join(if prefix.is_empty() {
            "plain.zip"
        } else {
            "dotted.zip"
        });
        zip(&source, prefix, META);
        assert_eq!(import(&source, root.path()), "imported");
    }
}

#[cfg(unix)]
#[test]
fn scan_and_hash_reject_nested_folder_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("packs");
    let source = packs.join("Linked");
    folder(&source, META);
    std::os::unix::fs::symlink(root.path().join("outside"), source.join("data/link")).unwrap();
    let card = scan_packs(&packs, &IconCache::new(&root.path().join("icons")))
        .unwrap()
        .packs
        .remove(0);
    assert_eq!(card.error.map(|e| e.code), Some("pack_symlink".into()));
    assert_eq!(
        HashCache::new()
            .hash(&source, PackLocation::Directory)
            .unwrap_err()
            .code(),
        "pack_symlink"
    );
    assert_eq!(import(&source, root.path()), "pack_symlink");
}

#[cfg(unix)]
#[test]
fn source_entrypoint_receipt() {
    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("packs");
    std::fs::create_dir(&packs).unwrap();
    zip(&packs.join("Zip.zip"), "", META);
    zip(&packs.join("Dotted.zip"), "./", META);
    folder(&packs.join("Dir"), META);
    folder(&packs.join("Linked"), META);
    std::fs::write(root.path().join("outside"), b"outside").unwrap();
    std::os::unix::fs::symlink(root.path().join("outside"), packs.join("Linked/data/link"))
        .unwrap();
    let listing = scan_packs(&packs, &IconCache::new(&root.path().join("icons"))).unwrap();
    assert_eq!(listing.count, 4);
    println!("name | scan | sha256 | datapack import");
    let mut rows = Vec::new();
    for card in listing.packs {
        let path = &card.path;
        let hash = HashCache::new()
            .hash(path, card.location)
            .unwrap_or_else(|e| e.code().into());
        let row = format!(
            "{} | {} | {} | {}",
            card.name,
            card.error.as_ref().map_or("ok", |e| &e.code),
            hash,
            import(path, root.path())
        );
        println!("{row}");
        rows.push(row);
    }
    assert_eq!(rows, [
        "Dir | ok | 57116f1f0a3f9324a03be6b2d9fd5a9fbdcd73b43351ae64e7faae5a24907839 | imported",
        "Dotted | ok | 42b4746227141f4b166279ffb302b788c3947866c0388955def98ef6adb86fd4 | imported",
        "Linked | pack_symlink | pack_symlink | pack_symlink",
        "Zip | ok | 0419bcc7bec8a1f701452f9441872481272bacdb3b92d0fa19cb5b762563ae8e | imported",
    ]);
}

#[test]
fn a_file_named_data_does_not_make_a_datapack() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("fake");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("pack.mcmeta"), META).unwrap();
    std::fs::write(source.join("data"), b"not a directory").unwrap();
    assert_eq!(import(&source, root.path()), "level_dat");
    assert!(matches!(
        import_result(&source, root.path()),
        Err(AppError::DataStack(mine_utils_core::data_stack::DataStackError::ResourceOnly { name })) if name == "fake"
    ));
}

#[cfg(unix)]
#[test]
fn zip_and_folder_root_symlinks_are_never_read() {
    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("packs");
    std::fs::create_dir(&packs).unwrap();
    let dir = root.path().join("RealDir");
    let archive = root.path().join("RealZip.zip");
    folder(&dir, META);
    zip(&archive, "", META);
    for (name, target, location) in [
        ("Dir", dir, PackLocation::Directory),
        ("Zip.zip", archive, PackLocation::Zip),
    ] {
        let source = packs.join(name);
        std::os::unix::fs::symlink(target, &source).unwrap();
        assert_eq!(
            HashCache::new().hash(&source, location).unwrap_err().code(),
            "pack_symlink"
        );
        assert_eq!(import(&source, root.path()), "pack_symlink");
    }
    let codes: Vec<_> = scan_packs(&packs, &IconCache::new(&root.path().join("icons")))
        .unwrap()
        .packs
        .into_iter()
        .map(|card| card.error.unwrap().code)
        .collect();
    assert_eq!(codes, vec!["pack_symlink", "pack_symlink"]);
}

#[test]
fn zip_metadata_cap_uses_actual_bytes_when_declared_size_is_false() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Forged.zip");
    let mut meta = META.to_vec();
    meta.resize(1024 * 1024 + 1, b' ');
    zip(&source, "", &meta);
    let mut bytes = std::fs::read(&source).unwrap();
    let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
    bytes[22..26].copy_from_slice(&1u32.to_le_bytes());
    std::fs::write(&source, bytes).unwrap();
    let packs = scan_packs(root.path(), &IconCache::new(&root.path().join("icons")))
        .unwrap()
        .packs;
    assert_eq!(packs[0].error.as_ref().unwrap().code, "pack_broken_archive");
    assert!(packs[0]
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("larger than"));
    assert_eq!(import(&source, root.path()), "pack_broken_archive");
}

#[cfg(unix)]
#[test]
fn symlink_cards_serialize_the_offending_path() {
    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("packs");
    folder(&packs.join("Nested"), META);
    let nested = packs.join("Nested/data/link");
    let linked = packs.join("Root");
    std::os::unix::fs::symlink("missing", &nested).unwrap();
    std::os::unix::fs::symlink(packs.join("Nested"), &linked).unwrap();
    let cards = scan_packs(&packs, &IconCache::new(&root.path().join("icons"))).unwrap();
    let wire = serde_json::to_value(cards).unwrap();
    assert_eq!(
        wire["packs"][0]["error"]["symlink_path"],
        nested.to_str().unwrap()
    );
    assert_eq!(
        wire["packs"][1]["error"]["symlink_path"],
        linked.to_str().unwrap()
    );
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
}

#[cfg(unix)]
#[test]
fn folder_metadata_and_icon_that_are_not_regular_files_are_skipped_without_blocking() {
    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("packs");
    let fifo_meta = packs.join("FifoMeta");
    folder(&fifo_meta, META);
    std::fs::remove_file(fifo_meta.join("pack.mcmeta")).unwrap();
    mkfifo(&fifo_meta.join("pack.mcmeta"));
    let dir_meta = packs.join("DirMeta");
    folder(&dir_meta, META);
    std::fs::remove_file(dir_meta.join("pack.mcmeta")).unwrap();
    std::fs::create_dir(dir_meta.join("pack.mcmeta")).unwrap();
    let fifo_icon = packs.join("FifoIcon");
    folder(&fifo_icon, META);
    mkfifo(&fifo_icon.join("pack.png"));
    let (tx, rx) = std::sync::mpsc::channel();
    let icons = root.path().join("icons");
    std::thread::spawn(move || {
        let cards = scan_packs(&packs, &IconCache::new(&icons)).unwrap().packs;
        let _ = tx.send(
            cards
                .into_iter()
                .map(|card| {
                    (
                        card.name,
                        card.error.map(|error| error.code),
                        card.icon.is_some(),
                    )
                })
                .collect::<Vec<_>>(),
        );
    });
    let rows = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("scan must not block on a FIFO");
    assert_eq!(
        rows,
        vec![
            (
                "DirMeta".to_string(),
                Some("pack_missing_meta".to_string()),
                false
            ),
            ("FifoIcon".to_string(), None, false),
            (
                "FifoMeta".to_string(),
                Some("pack_missing_meta".to_string()),
                false
            ),
        ]
    );
}
