use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use mine_utils_core::resource_stack::WriteGuard;
use mine_utils_core::{
    data_stack, level_dat, resource_stack, AppError, DataPaths, DatapackImportOutcome, GameRunning,
    GameRunningKind, HashCache, IconCache, ImportChoice, ImportOutcome, PackError, StackPaths,
    TrashService,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

struct Trash(PathBuf);

impl TrashService for Trash {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
        let name = path.file_name().unwrap().to_string_lossy();
        let name = if name.starts_with(".mine-utils-replace.") {
            "held-original"
        } else {
            &name
        };
        std::fs::rename(path, self.0.join(name)).map_err(|err| PackError::Trash {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })
    }
}

struct Guard {
    calls: Cell<usize>,
    fail_at: usize,
}

impl WriteGuard for Guard {
    fn ensure_writable(&self, _: &Path) -> Result<(), AppError> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == self.fail_at {
            return Err(AppError::GameRunning {
                running: GameRunning {
                    kind: GameRunningKind::Process,
                    detail: "fixture guard refusal".into(),
                    lock_path: None,
                    pid: None,
                    command: None,
                },
            });
        }
        Ok(())
    }
}

fn pack(path: &Path, content: &[u8], zipped: bool) {
    let meta = br#"{"pack":{"pack_format":48,"description":"fixture"}}"#;
    if zipped {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in [("pack.mcmeta", meta.as_slice()), ("data/test.txt", content)] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    } else {
        std::fs::create_dir_all(path.join("data")).unwrap();
        std::fs::write(path.join("pack.mcmeta"), meta).unwrap();
        std::fs::write(path.join("data/test.txt"), content).unwrap();
    }
}

fn disk(root: &Path) -> BTreeMap<String, Value> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Value>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            if path.is_dir() {
                files.insert(format!("{relative}/"), json!("directory"));
                visit(root, &path, files);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                let mut entry = json!({"sha256":hex::encode(Sha256::digest(&bytes))});
                if path.file_name().unwrap() == "level.dat" {
                    let mut raw = Vec::new();
                    std::io::Read::read_to_end(
                        &mut flate2::read::GzDecoder::new(bytes.as_slice()),
                        &mut raw,
                    )
                    .unwrap();
                    let nbt: fastnbt::Value = fastnbt::from_bytes(&raw).unwrap();
                    entry["nbt"] = serde_json::to_value(nbt).unwrap();
                }
                files.insert(relative, entry);
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

struct Lock(Child);
impl Drop for Lock {
    fn drop(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}

fn lock_world(world: &Path) -> Lock {
    let path = world.join("session.lock");
    std::fs::write(&path, b"lock").unwrap();
    let child = Command::new("python3")
        .args(["-c", "import fcntl,sys; f=open(sys.argv[1],'r+'); fcntl.lockf(f,fcntl.LOCK_EX); print('locked',flush=True); sys.stdin.read()"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lock = Lock(child);
    use std::io::BufRead;
    let mut ready = String::new();
    std::io::BufReader::new(lock.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "locked\n");
    assert!(mine_utils_core::game_lock::world_locked(world).is_some());
    lock
}

#[test]
fn import_entries_preserve_outcomes_and_disk_state() {
    let cases = [
        ("new", ImportChoice::Default, "imported"),
        ("duplicate", ImportChoice::Default, "duplicate"),
        ("skip", ImportChoice::Skip, "skipped"),
        ("replace_duplicate", ImportChoice::Replace, "imported"),
        ("replace_same_duplicate", ImportChoice::Replace, "imported"),
        ("replace_destination", ImportChoice::Replace, "imported"),
        ("collision", ImportChoice::Default, "collision"),
        ("skip_collision", ImportChoice::Skip, "collision"),
        ("duplicate_collision", ImportChoice::Replace, "collision"),
        ("guard", ImportChoice::Replace, "game_running"),
        ("guard_recheck", ImportChoice::Replace, "game_running"),
        ("locked", ImportChoice::Default, "locked"),
        ("locked_replace", ImportChoice::Replace, "level_dat"),
    ];
    for data in [false, true] {
        for zipped in [false, true] {
            for (case, choice, expected) in cases {
                let root = tempfile::tempdir().unwrap();
                let game = root.path().join("game");
                let world = game.join("saves/alpha");
                let packs = if data {
                    world.join("datapacks")
                } else {
                    game.join("resourcepacks")
                };
                std::fs::create_dir_all(&packs).unwrap();
                std::fs::create_dir_all(&world).unwrap();
                let name = if zipped { "New.zip" } else { "New" };
                let old = if zipped { "Old.zip" } else { "Old" };
                let source = root.path().join("incoming").join(name);
                pack(&source, b"incoming", zipped);
                if matches!(
                    case,
                    "duplicate"
                        | "skip"
                        | "replace_duplicate"
                        | "duplicate_collision"
                        | "guard"
                        | "guard_recheck"
                ) {
                    pack(&packs.join(old), b"incoming", zipped);
                }
                if case == "replace_same_duplicate" {
                    pack(&packs.join(name), b"incoming", zipped);
                }
                if matches!(
                    case,
                    "replace_destination" | "collision" | "skip_collision" | "duplicate_collision"
                ) {
                    pack(&packs.join(name), b"different", zipped);
                }
                std::fs::write(game.join("options.txt"), format!("music:0.4\nresourcePacks:[\"vanilla\",\"file/{old}\"]\nincompatibleResourcePacks:[\"file/{old}\"]\n")).unwrap();
                #[derive(serde::Serialize)]
                #[serde(rename_all = "PascalCase")]
                struct Lists {
                    enabled: Vec<String>,
                    disabled: Vec<String>,
                }
                #[derive(serde::Serialize)]
                #[serde(rename_all = "PascalCase")]
                struct Data {
                    level_name: String,
                    version: std::collections::BTreeMap<String, String>,
                    data_packs: Lists,
                }
                let nbt = BTreeMap::from([(
                    "Data",
                    Data {
                        level_name: "fixture".into(),
                        version: BTreeMap::from([("Name".into(), "1.21.10".into())]),
                        data_packs: Lists {
                            enabled: vec!["vanilla".into()],
                            disabled: vec![],
                        },
                    },
                )]);
                std::fs::write(
                    world.join("level.dat"),
                    level_dat::gzip(&fastnbt::to_bytes(&nbt).unwrap()),
                )
                .unwrap();
                let trash_dir = root.path().join("trash");
                std::fs::create_dir(&trash_dir).unwrap();
                let trash = Trash(trash_dir.clone());
                let icons = IconCache::new(&root.path().join("cache"));
                let hashes = HashCache::new();
                let guard = Guard {
                    calls: Cell::new(0),
                    fail_at: match case {
                        "guard" => 1,
                        "guard_recheck" => 2,
                        _ => usize::MAX,
                    },
                };
                let _lock = case.starts_with("locked").then(|| lock_world(&world));
                let before = disk(&game);
                let result = if data {
                    let paths = DataPaths::for_world(
                        &mine_utils_core::WorldPaths::for_world(&game, "alpha").unwrap(),
                        &root.path().join("backups"),
                        mine_utils_core::BackupsConfig {
                            enabled: false,
                            keep: 10,
                        },
                    );
                    data_stack::import_datapack_with(&paths, &source, choice, &icons, &hashes, &trash, &guard).map(|outcome| match outcome {
                        DatapackImportOutcome::Imported { filename } => json!({"outcome":"imported", "filename":filename}),
                        DatapackImportOutcome::Locked { filename, .. } => json!({"outcome":"locked", "filename":filename}),
                        DatapackImportOutcome::Duplicate { name, sha256, existing } => json!({"outcome":"duplicate", "name":name,"sha256":sha256,"existing":Path::new(&existing).strip_prefix(&game).unwrap()}),
                        DatapackImportOutcome::Collision { name } => json!({"outcome":"collision", "name":name}),
                        DatapackImportOutcome::Skipped => json!({"outcome":"skipped"}),
                    })
                } else {
                    let paths = StackPaths::for_game_dir(
                        &game,
                        &root.path().join("backups"),
                        mine_utils_core::BackupsConfig {
                            enabled: false,
                            keep: 10,
                        },
                    );
                    if case.starts_with("locked") {
                        resource_stack::import_pack_safely(&paths, &source, choice, &icons, &hashes)
                    } else {
                        resource_stack::import_pack_safely_with(&paths, &source, choice, &icons, &hashes, &trash, &guard)
                    }.map(|outcome| match outcome {
                        ImportOutcome::Imported { pack } => json!({"outcome":"imported", "name":pack.name,"kind":pack.kind,"description":pack.description,"path":pack.path.strip_prefix(&game).unwrap()}),
                        ImportOutcome::Duplicate { name, sha256, existing } => json!({"outcome":"duplicate", "name":name,"sha256":sha256,"existing":Path::new(&existing).strip_prefix(&game).unwrap()}),
                        ImportOutcome::Collision { name } => json!({"outcome":"collision", "name":name}),
                        ImportOutcome::Skipped => json!({"outcome":"skipped"}),
                    })
                };
                let outcome = result.unwrap_or_else(|err| json!({"outcome":err.code()}));
                let expected = if !data && case.starts_with("locked") {
                    "game_running"
                } else {
                    expected
                };
                assert_eq!(
                    outcome["outcome"], expected,
                    "data={data} zip={zipped} case={case}: {outcome}"
                );
                if matches!(
                    expected,
                    "collision" | "duplicate" | "skipped" | "game_running" | "level_dat"
                ) {
                    assert_eq!(
                        disk(&game),
                        before,
                        "refused or kept imports preserve every game file"
                    );
                    assert_eq!(disk(&trash_dir), BTreeMap::new());
                }
            }
        }
    }
}
