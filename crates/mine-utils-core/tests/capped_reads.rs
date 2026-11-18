use std::path::Path;

use mine_utils_core::{
    data_stack, modrinth, options_txt, provenance, BackupsConfig, DataPaths, WorldPaths,
};

fn oversized(path: &Path, bytes: u64) {
    std::fs::File::create(path).unwrap().set_len(bytes).unwrap();
}

#[test]
fn remaining_readers_refuse_oversized_files() {
    let root = tempfile::tempdir().unwrap();
    let options = root.path().join("options.txt");
    std::fs::write(&options, b"resourcePacks:[]\n").unwrap();
    assert_eq!(
        options_txt::read_options(&options)
            .unwrap()
            .resource_packs(),
        Vec::<String>::new()
    );
    oversized(&options, 1024 * 1024 + 1);
    assert_eq!(
        options_txt::read_options(&options).unwrap_err().to_string(),
        "io error: file is too large"
    );

    let record = provenance::provenance_file(root.path());
    std::fs::write(&record, b"{}").unwrap();
    assert_eq!(provenance::lookup(root.path(), "missing").unwrap(), None);
    oversized(&record, 16 * 1024 * 1024 + 1);
    assert_eq!(
        provenance::lookup(root.path(), "missing")
            .unwrap_err()
            .code(),
        "provenance_io"
    );

    let profiles = root.path().join("launcher_profiles.json");
    let selected = br#"{"profiles":{"test":{"lastVersionId":"modded","inheritsFrom":"1.21.4"}}}"#;
    std::fs::write(&profiles, selected).unwrap();
    assert_eq!(
        modrinth::preferred_launcher_version(root.path()),
        Some("1.21.4".into())
    );
    let mut padded = selected.to_vec();
    padded.resize(4 * 1024 * 1024 + 1, b' ');
    std::fs::write(&profiles, padded).unwrap();
    assert_eq!(modrinth::preferred_launcher_version(root.path()), None);

    let world = root.path().join("saves/world");
    std::fs::create_dir_all(world.join("datapacks")).unwrap();
    let paths = DataPaths::for_world(
        &WorldPaths::for_world(root.path(), "world").unwrap(),
        &root.path().join("app"),
        BackupsConfig::default(),
    );
    oversized(paths.level_dat_path(), 64 * 1024 * 1024 + 1);
    assert_eq!(
        data_stack::load(&paths).unwrap_err().to_string(),
        "io error: file is too large"
    );
}

#[cfg(unix)]
#[test]
fn shared_readers_follow_regular_files_and_keep_their_caps() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("shared");
    symlink(&target, root.path().join("options.txt")).unwrap();
    symlink(&target, root.path().join("launcher_profiles.json")).unwrap();
    symlink(&target, provenance::provenance_file(root.path())).unwrap();

    std::fs::write(&target, b"resourcePacks:[\"vanilla\"]\n").unwrap();
    assert_eq!(
        options_txt::read_options(&root.path().join("options.txt"))
            .unwrap()
            .resource_packs(),
        ["vanilla"]
    );
    oversized(&target, 1024 * 1024 + 1);
    assert_eq!(
        options_txt::read_options(&root.path().join("options.txt"))
            .unwrap_err()
            .to_string(),
        "io error: file is too large"
    );
    let selected = br#"{"profiles":{"test":{"lastVersionId":"modded","inheritsFrom":"1.21.4"}}}"#;
    std::fs::write(&target, selected).unwrap();
    assert_eq!(
        modrinth::preferred_launcher_version(root.path()),
        Some("1.21.4".into())
    );
    let mut padded = selected.to_vec();
    padded.resize(4 * 1024 * 1024 + 1, b' ');
    std::fs::write(&target, padded).unwrap();
    assert_eq!(modrinth::preferred_launcher_version(root.path()), None);

    std::fs::write(
        &target,
        br#"{"hash":{"project_id":"project","version_id":"version","file_name":"pack.zip"}}"#,
    )
    .unwrap();
    assert_eq!(
        provenance::lookup(root.path(), "hash").unwrap(),
        Some(provenance::ProvenanceEntry {
            project_id: "project".into(),
            version_id: "version".into(),
            file_name: "pack.zip".into(),
        })
    );
    oversized(&target, 16 * 1024 * 1024 + 1);
    assert_eq!(
        provenance::lookup(root.path(), "hash").unwrap_err().code(),
        "provenance_io"
    );
}

#[cfg(unix)]
#[test]
fn remaining_readers_refuse_non_regular_files_without_blocking() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    for linked in [false, true] {
        for fifo in [false, true] {
            let root = tempfile::tempdir().unwrap();
            for name in ["options.txt", "provenance.json", "launcher_profiles.json"] {
                let path = root.path().join(name);
                let target = if linked {
                    root.path().join(format!("{name}.target"))
                } else {
                    path.clone()
                };
                if fifo {
                    let name = std::ffi::CString::new(target.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                } else {
                    std::fs::create_dir(&target).unwrap();
                }
                if linked {
                    symlink(&target, &path).unwrap();
                }
            }
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let paths = mine_utils_core::StackPaths::for_game_dir(
                    root.path(),
                    &root.path().join("app"),
                    BackupsConfig::default(),
                );
                let _ = tx.send((
                    mine_utils_core::resource_stack::load(&paths)
                        .unwrap_err()
                        .to_string(),
                    provenance::lookup(root.path(), "missing")
                        .unwrap_err()
                        .code()
                        .to_string(),
                    modrinth::preferred_launcher_version(root.path()),
                ));
            });
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
                (
                    "io error: not a regular file".into(),
                    "provenance_io".into(),
                    None
                )
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn data_stack_keeps_refusing_leaf_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let world = root.path().join("saves/world");
    std::fs::create_dir_all(&world).unwrap();
    let target = root.path().join("shared-level.dat");
    std::fs::write(&target, b"unused").unwrap();
    std::os::unix::fs::symlink(target, world.join("level.dat")).unwrap();
    let paths = DataPaths::for_world(
        &WorldPaths::for_world(root.path(), "world").unwrap(),
        &root.path().join("app"),
        BackupsConfig::default(),
    );
    assert_eq!(data_stack::load(&paths).unwrap_err().code(), "world");
}

#[test]
fn backup_reads_cap_both_restore_sources_and_current_files() {
    use mine_utils_core::{backup, level_backup, level_dat};
    let root = tempfile::tempdir().unwrap();
    let backups = root.path().join("backups");
    let config = BackupsConfig::default();
    let options_bytes = b"resourcePacks:[]\n";
    let options = root.path().join("options.txt");
    let saved = backup::create(&backups, options_bytes, &config)
        .unwrap()
        .created
        .unwrap();
    assert_eq!(
        backup::read(&backups, &saved.file_name).unwrap(),
        options_bytes
    );
    oversized(&options, 1024 * 1024 + 1);
    assert_eq!(
        backup::restore(&backups, &options, &saved.file_name, &config)
            .unwrap_err()
            .to_string(),
        "io error: file is too large"
    );
    std::fs::write(&options, options_bytes).unwrap();
    oversized(&backups.join(&saved.file_name), 1024 * 1024 + 1);
    assert_eq!(
        backup::read(&backups, &saved.file_name).unwrap_err().code(),
        "backup"
    );
    assert_eq!(
        backup::restore(&backups, &options, &saved.file_name, &config)
            .unwrap_err()
            .code(),
        "backup"
    );
    assert_eq!(std::fs::read(&options).unwrap(), options_bytes);

    let level_bytes = level_dat::gzip(&fastnbt::to_bytes(&serde_json::json!({"Data": {"Version": {"Name": "1.21.4"}, "DataPacks": {"Enabled": ["vanilla"], "Disabled": []}}})).unwrap());
    let saved = level_backup::create(&backups, "world", &level_bytes, &config)
        .unwrap()
        .created
        .unwrap();
    let level = root.path().join("level.dat");
    std::fs::write(&level, &level_bytes).unwrap();
    level_backup::restore_guarded(&backups, "world", &level, &saved.file_name, &config, || {
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(&level).unwrap(), level_bytes);
    oversized(&level, 64 * 1024 * 1024 + 1);
    assert_eq!(
        level_backup::restore_guarded(&backups, "world", &level, &saved.file_name, &config, || Ok(
            ()
        ))
        .unwrap_err()
        .to_string(),
        "io error: file is too large"
    );
    std::fs::write(&level, &level_bytes).unwrap();
    oversized(&backups.join(&saved.file_name), 64 * 1024 * 1024 + 1);
    assert_eq!(
        level_backup::restore_guarded(&backups, "world", &level, &saved.file_name, &config, || Ok(
            ()
        ))
        .unwrap_err()
        .code(),
        "level_backup"
    );
    assert_eq!(std::fs::read(&level).unwrap(), level_bytes);
}

#[test]
fn provenance_write_limit_preserves_the_readable_store() {
    let root = tempfile::tempdir().unwrap();
    let entry = provenance::ProvenanceEntry {
        project_id: "project".into(),
        version_id: "version".into(),
        file_name: "pack.zip".into(),
    };
    provenance::record(root.path(), "existing", entry.clone()).unwrap();
    let before = std::fs::read(provenance::provenance_file(root.path())).unwrap();
    let too_large = provenance::ProvenanceEntry {
        file_name: "x".repeat(16 * 1024 * 1024),
        ..entry.clone()
    };
    assert_eq!(
        provenance::record(root.path(), "new", too_large)
            .unwrap_err()
            .to_string(),
        "provenance record unavailable: provenance record is too large"
    );
    assert_eq!(
        std::fs::read(provenance::provenance_file(root.path())).unwrap(),
        before
    );
    assert_eq!(
        provenance::lookup(root.path(), "existing").unwrap(),
        Some(entry)
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
