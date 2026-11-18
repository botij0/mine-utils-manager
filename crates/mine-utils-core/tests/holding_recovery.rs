use mine_utils_core::{data_stack, scan_packs, IconCache};

#[test]
fn leftover_replace_entries_are_listed_with_original_names() {
    let root = tempfile::tempdir().unwrap();
    let icons = IconCache::new(root.path());
    for folder in ["resourcepacks", "saves/World/datapacks"] {
        let packs = root.path().join(folder);
        std::fs::create_dir_all(&packs).unwrap();
        std::fs::write(
            packs.join(".mine-utils-replace-My.pack.zip.4294967295-123-0.tmp"),
            b"original",
        )
        .unwrap();
        let listing = if folder == "resourcepacks" {
            scan_packs(&packs, &icons).unwrap()
        } else {
            data_stack::scan_datapacks(&packs, &icons).unwrap()
        };
        assert_eq!(
            serde_json::to_value(listing).unwrap()["holding_entries"],
            serde_json::json!([
                {"holdingName": ".mine-utils-replace-My.pack.zip.4294967295-123-0.tmp", "originalName": "My.pack.zip"}
            ])
        );
    }
}

const HOLDING: &str = ".mine-utils-replace-My.pack.zip.4294967295-123-0.tmp";

fn paths(root: &std::path::Path) -> mine_utils_core::StackPaths {
    mine_utils_core::StackPaths::for_game_dir(root, &root.join("app-data"), Default::default())
}

#[test]
fn restore_moves_original_bytes_and_preserves_references() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join(HOLDING), b"old pack").unwrap();
    std::fs::write(
        root.path().join("options.txt"),
        b"resourcePacks:[\"file/My.pack.zip\"]\n",
    )
    .unwrap();
    mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), HOLDING)
        .unwrap();
    assert_eq!(
        std::fs::read(folder.join("My.pack.zip")).unwrap(),
        b"old pack"
    );
    assert!(!folder.join(HOLDING).exists());
    assert_eq!(
        std::fs::read(root.path().join("options.txt")).unwrap(),
        b"resourcePacks:[\"file/My.pack.zip\"]\n"
    );
}

#[test]
fn restore_refuses_existing_files_and_directories_without_replacement() {
    for directory in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("resourcepacks");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join(HOLDING), b"old pack").unwrap();
        if directory {
            std::fs::create_dir(folder.join("My.pack.zip")).unwrap();
        } else {
            std::fs::write(folder.join("My.pack.zip"), b"new pack").unwrap();
        }
        let err = mine_utils_core::holding_recovery::restore_resource_holding(
            &paths(root.path()),
            HOLDING,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("original name must be free"),
            "{err}"
        );
        assert_eq!(std::fs::read(folder.join(HOLDING)).unwrap(), b"old pack");
        if directory {
            assert!(folder.join("My.pack.zip").is_dir());
        } else {
            assert_eq!(
                std::fs::read(folder.join("My.pack.zip")).unwrap(),
                b"new pack"
            );
        }
    }
}

#[test]
fn restore_datapack_folder_preserves_level_dat() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("saves/World/datapacks");
    std::fs::create_dir_all(folder.join(HOLDING)).unwrap();
    std::fs::write(folder.join(HOLDING).join("pack.mcmeta"), b"original folder").unwrap();
    std::fs::write(
        root.path().join("saves/World/level.dat"),
        b"unchanged level",
    )
    .unwrap();
    let world = mine_utils_core::WorldPaths::for_world(root.path(), "World").unwrap();
    let paths = mine_utils_core::DataPaths::for_world(&world, root.path(), Default::default());
    mine_utils_core::holding_recovery::restore_datapack_holding(&paths, HOLDING).unwrap();
    assert_eq!(
        std::fs::read(folder.join("My.pack.zip/pack.mcmeta")).unwrap(),
        b"original folder"
    );
    assert_eq!(
        std::fs::read(world.level_dat_path()).unwrap(),
        b"unchanged level"
    );
}

#[test]
fn listing_and_restore_reject_malformed_names_and_current_process() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    std::fs::create_dir(&folder).unwrap();
    let active = format!(
        ".mine-utils-replace-Active.zip.{}-123-0.tmp",
        std::process::id()
    );
    let invalid = [
        "plain.zip",
        ".mine-utils-replace-X.0-123-0.tmp",
        ".mine-utils-replace-X.5-abc-0.tmp",
        ".mine-utils-replace-X.5-123-0-extra.tmp",
        ".mine-utils-replace-...5-123-0.tmp",
        active.as_str(),
    ];
    for name in invalid {
        std::fs::write(folder.join(name), b"keep").unwrap();
        let err =
            mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), name)
                .unwrap_err();
        assert_eq!(err.code(), "pack_broken_archive");
        assert_eq!(std::fs::read(folder.join(name)).unwrap(), b"keep");
    }
    std::fs::write(folder.join(HOLDING), b"recoverable").unwrap();
    let listing = scan_packs(&folder, &IconCache::new(root.path())).unwrap();
    assert_eq!(listing.holding_entries.len(), 1);
    assert_eq!(listing.holding_entries[0].original_name, "My.pack.zip");
    let err = mine_utils_core::holding_recovery::restore_resource_holding(
        &paths(root.path()),
        "../outside",
    )
    .unwrap_err();
    assert_eq!(err.code(), "pack_broken_archive");
}

#[cfg(unix)]
#[test]
fn symlinked_pack_folders_list_packs_but_never_offer_or_restore_holdings() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join(HOLDING), b"outside").unwrap();
    std::fs::create_dir(outside.path().join("Valid")).unwrap();
    std::fs::write(
        outside.path().join("Valid/pack.mcmeta"),
        br#"{"pack":{"pack_format":15,"description":"valid pack"}}"#,
    )
    .unwrap();
    let folder = root.path().join("resourcepacks");
    std::os::unix::fs::symlink(outside.path(), &folder).unwrap();
    let listing = scan_packs(&folder, &IconCache::new(root.path())).unwrap();
    assert_eq!(listing.count, 1);
    assert_eq!(listing.packs[0].name, "Valid");
    assert!(listing.packs[0].error.is_none());
    assert!(listing.holding_entries.is_empty());
    assert_eq!(
        mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), HOLDING)
            .unwrap_err()
            .code(),
        "pack_symlink"
    );
    std::fs::remove_file(&folder).unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::os::unix::fs::symlink(outside.path().join(HOLDING), folder.join(HOLDING)).unwrap();
    assert_eq!(
        mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), HOLDING)
            .unwrap_err()
            .code(),
        "pack_symlink"
    );
    assert!(scan_packs(&folder, &IconCache::new(root.path()))
        .unwrap()
        .holding_entries
        .is_empty());
    assert_eq!(
        std::fs::read(outside.path().join(HOLDING)).unwrap(),
        b"outside"
    );
}

#[cfg(unix)]
#[test]
fn restore_refuses_a_running_game() {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    let world = root.path().join("saves/World");
    std::fs::create_dir(&folder).unwrap();
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(folder.join(HOLDING), b"old pack").unwrap();
    let mut child = Command::new("python3").args(["-c", "import fcntl,sys; f=open(sys.argv[1], 'w'); fcntl.lockf(f, fcntl.LOCK_EX); print('locked', flush=True); sys.stdin.read()"])
        .arg(world.join("session.lock")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "locked\n");
    let err =
        mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), HOLDING)
            .unwrap_err();
    drop(child.stdin.take());
    child.wait().unwrap();
    assert_eq!(err.code(), "game_running");
    assert_eq!(std::fs::read(folder.join(HOLDING)).unwrap(), b"old pack");
}

#[cfg(unix)]
#[test]
fn restore_permission_denied_names_the_holding_without_collision_advice() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join(HOLDING), b"old pack").unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result =
        mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), HOLDING);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    let message = result.unwrap_err().to_string();
    assert!(message.contains("Permission denied"), "{message}");
    assert!(
        message.contains(&folder.join(HOLDING).display().to_string()),
        "{message}"
    );
    assert!(!message.contains("unsupported"), "{message}");
    assert!(!message.contains("must be free"), "{message}");
    assert_eq!(std::fs::read(folder.join(HOLDING)).unwrap(), b"old pack");
}

#[test]
fn vanished_holdings_and_pack_folders_report_the_named_entry_is_gone() {
    for datapack in [false, true] {
        for remove_folder in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join(if datapack {
                "saves/World/datapacks"
            } else {
                "resourcepacks"
            });
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join(HOLDING), b"old pack").unwrap();
            let listing = if datapack {
                data_stack::scan_datapacks(&folder, &IconCache::new(root.path())).unwrap()
            } else {
                scan_packs(&folder, &IconCache::new(root.path())).unwrap()
            };
            assert_eq!(listing.holding_entries[0].original_name, "My.pack.zip");
            std::fs::remove_file(folder.join(HOLDING)).unwrap();
            if remove_folder {
                std::fs::remove_dir(&folder).unwrap();
            }
            let result = if datapack {
                let world = mine_utils_core::WorldPaths::for_world(root.path(), "World").unwrap();
                let paths =
                    mine_utils_core::DataPaths::for_world(&world, root.path(), Default::default());
                mine_utils_core::holding_recovery::restore_datapack_holding(&paths, HOLDING)
            } else {
                mine_utils_core::holding_recovery::restore_resource_holding(
                    &paths(root.path()),
                    HOLDING,
                )
            };
            let err = result.unwrap_err();
            assert_eq!(err.code(), "pack_not_found");
            assert!(err.to_string().contains("entry is gone"), "{err}");
            assert!(err.to_string().contains(HOLDING), "{err}");
        }
    }
}

const DIRECTORY_HOLDING: &str = ".mine-utils-replace.4294967295-123-0.tmp";

#[test]
fn directory_holdings_restore_full_names_for_resources_and_datapacks() {
    for datapack in [false, true] {
        for pack_directory in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join(if datapack {
                "saves/World/datapacks"
            } else {
                "resourcepacks"
            });
            let holding = folder.join(DIRECTORY_HOLDING);
            std::fs::create_dir_all(&holding).unwrap();
            let name = format!("{}.zip", "a".repeat(216));
            let child = holding.join(&name);
            if pack_directory {
                std::fs::create_dir(&child).unwrap();
                std::fs::write(child.join("pack.mcmeta"), b"original").unwrap();
            } else {
                std::fs::write(&child, b"original").unwrap();
            }
            let references = root.path().join(if datapack {
                "saves/World/level.dat"
            } else {
                "options.txt"
            });
            std::fs::write(&references, b"unchanged references").unwrap();
            let listing = if datapack {
                data_stack::scan_datapacks(&folder, &IconCache::new(root.path())).unwrap()
            } else {
                scan_packs(&folder, &IconCache::new(root.path())).unwrap()
            };
            assert_eq!(
                serde_json::to_value(listing).unwrap()["holding_entries"],
                serde_json::json!([
                    {"holdingName": DIRECTORY_HOLDING, "originalName": name}
                ])
            );
            let restore = || {
                if datapack {
                    let world =
                        mine_utils_core::WorldPaths::for_world(root.path(), "World").unwrap();
                    let paths = mine_utils_core::DataPaths::for_world(
                        &world,
                        root.path(),
                        Default::default(),
                    );
                    mine_utils_core::holding_recovery::restore_datapack_holding(
                        &paths,
                        DIRECTORY_HOLDING,
                    )
                } else {
                    mine_utils_core::holding_recovery::restore_resource_holding(
                        &paths(root.path()),
                        DIRECTORY_HOLDING,
                    )
                }
            };
            let destination = folder.join(&name);
            std::fs::write(&destination, b"rival").unwrap();
            assert!(restore()
                .unwrap_err()
                .to_string()
                .contains("original name must be free"));
            assert_eq!(std::fs::read(&destination).unwrap(), b"rival");
            std::fs::remove_file(&destination).unwrap();
            restore().unwrap();
            let restored = if pack_directory {
                destination.join("pack.mcmeta")
            } else {
                destination
            };
            assert_eq!(std::fs::read(restored).unwrap(), b"original");
            assert!(!holding.exists());
            assert_eq!(std::fs::read(references).unwrap(), b"unchanged references");
        }
    }
}

#[test]
fn invalid_directory_holdings_are_skipped_and_never_touched() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    std::fs::create_dir(&folder).unwrap();
    for (index, children) in [0, 2, 1].into_iter().enumerate() {
        let name = format!(".mine-utils-replace.4294967295-123-{index}.tmp");
        let holding = folder.join(&name);
        std::fs::create_dir(&holding).unwrap();
        for child in 0..children {
            std::fs::write(holding.join(format!("child{child}")), b"keep").unwrap();
        }
        if children == 1 {
            std::fs::rename(
                holding.join("child0"),
                holding.join(".mine-utils-copy.5-123-0.tmp"),
            )
            .unwrap();
        }
        assert_eq!(
            mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), &name)
                .unwrap_err()
                .code(),
            "pack_broken_archive"
        );
        assert_eq!(std::fs::read_dir(&holding).unwrap().count(), children);
    }
    std::fs::write(folder.join(HOLDING), b"legacy").unwrap();
    let listing = scan_packs(&folder, &IconCache::new(root.path())).unwrap();
    assert_eq!(listing.holding_entries.len(), 1);
    assert_eq!(listing.holding_entries[0].holding_name, HOLDING);
}

#[cfg(unix)]
#[test]
fn directory_holdings_reject_symlink_children_and_hide_current_process() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    let holding = folder.join(DIRECTORY_HOLDING);
    std::fs::create_dir_all(&holding).unwrap();
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"outside").unwrap();
    std::os::unix::fs::symlink(&outside, holding.join("Pack.zip")).unwrap();
    assert_eq!(
        mine_utils_core::holding_recovery::restore_resource_holding(
            &paths(root.path()),
            DIRECTORY_HOLDING
        )
        .unwrap_err()
        .code(),
        "pack_symlink"
    );
    let active = format!(".mine-utils-replace.{}-123-0.tmp", std::process::id());
    std::fs::create_dir(folder.join(&active)).unwrap();
    std::fs::write(folder.join(&active).join("Pack.zip"), b"active").unwrap();
    assert_eq!(
        mine_utils_core::holding_recovery::restore_resource_holding(&paths(root.path()), &active)
            .unwrap_err()
            .code(),
        "pack_broken_archive"
    );
    std::fs::write(folder.join(HOLDING), b"legacy").unwrap();
    let listing = scan_packs(&folder, &IconCache::new(root.path())).unwrap();
    assert_eq!(listing.holding_entries.len(), 1);
    assert_eq!(listing.holding_entries[0].holding_name, HOLDING);
    assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    assert_eq!(
        std::fs::read(folder.join(active).join("Pack.zip")).unwrap(),
        b"active"
    );
}

#[test]
fn replace_prefixed_pack_and_copy_temp_are_not_holdings() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("resourcepacks");
    std::fs::create_dir(&folder).unwrap();
    let mut zip = zip::ZipWriter::new(
        std::fs::File::create(folder.join("mine-utils-replace-X.zip")).unwrap(),
    );
    zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"pack":{"pack_format":34,"description":"valid"}}"#)
        .unwrap();
    zip.start_file("assets/test.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"pack").unwrap();
    zip.finish().unwrap();
    std::fs::write(
        folder.join(".mine-utils-copy.4294967295-123-0.tmp"),
        b"partial copy",
    )
    .unwrap();
    let listing = scan_packs(&folder, &IconCache::new(root.path())).unwrap();
    assert_eq!(listing.count, 1);
    assert_eq!(listing.packs[0].name, "mine-utils-replace-X");
    assert!(listing.packs[0].error.is_none());
    assert!(listing.holding_entries.is_empty());
}
