//! Modrinth install-path tests: the shared prepare/finish routine on a temp
//! game dir with a mock transport. Nothing here touches the network or the
//! real game dir.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mine_utils_core::{
    finish_modrinth_install, level_dat, prepare_modrinth_install, provenance_lookup,
    resource_stack, AppError, BackupsConfig, CoreInstallOutcome, DataPaths, HashCache, IconCache,
    ModrinthClient, ModrinthError, RawResponse, StackPaths, Transport,
};
use sha2::{Digest, Sha256, Sha512};

const PROJECT: &str = "testproj1";
const VERSION: &str = "version01";
const RESOURCE_FILE: &str = "fresh-1.9.zip";
const DATAPACK_FILE: &str = "cool-datapack-2.0.zip";
const RESOURCE_URL: &str = "https://cdn.modrinth.com/data/abc/fresh-1.9.zip";
const DATAPACK_URL: &str = "https://cdn.modrinth.com/data/def/cool-datapack-2.0.zip";
const WORLD_VERSION: &str = "1.21.10";

fn versions_url() -> String {
    format!("https://api.modrinth.com/v2/project/{PROJECT}/version")
}

struct MockApi {
    calls: Mutex<Vec<String>>,
    bodies: BTreeMap<String, Vec<u8>>,
}

impl MockApi {
    fn new() -> Self {
        MockApi {
            calls: Mutex::new(Vec::new()),
            bodies: BTreeMap::new(),
        }
    }

    fn with(mut self, url: &str, body: Vec<u8>) -> Self {
        self.bodies.insert(url.to_string(), body);
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }

    fn cdn_calls(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|url| url.contains("cdn.modrinth.com"))
            .collect()
    }
}

impl Transport for MockApi {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
        self.calls.lock().expect("calls").push(url.to_string());
        match self.bodies.get(url) {
            Some(body) => Ok(RawResponse {
                status: 200,
                headers: vec![],
                body: Box::new(std::io::Cursor::new(body.clone())),
            }),
            None => Ok(RawResponse {
                status: 404,
                headers: vec![],
                body: Box::new(std::io::Cursor::new(Vec::new())),
            }),
        }
    }
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    for (name, data) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(data).expect("write");
    }
    zip.finish().expect("finish");
    out.into_inner()
}

fn pack_mcmeta() -> &'static [u8] {
    br#"{"pack": {"pack_format": 48, "description": "temp"}}"#
}

fn resource_zip() -> Vec<u8> {
    zip_bytes(&[
        ("assets/minecraft/textures/a.png", b"asset"),
        ("pack.mcmeta", pack_mcmeta()),
    ])
}

fn datapack_zip() -> Vec<u8> {
    zip_bytes(&[
        ("data/example/loot_tables/x.json", b"{}"),
        ("pack.mcmeta", pack_mcmeta()),
    ])
}

fn sha512_hex(bytes: &[u8]) -> String {
    hex::encode(Sha512::digest(bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn versions_json(url: &str, filename: &str, sha512: &str, games: &[&str]) -> Vec<u8> {
    let games: Vec<String> = games.iter().map(|g| g.to_string()).collect();
    serde_json::json!([{
        "id": VERSION,
        "project_id": PROJECT,
        "name": "1.0",
        "version_number": "1.0",
        "game_versions": games,
        "loaders": ["minecraft"],
        "files": [{
            "url": url,
            "filename": filename,
            "size": 100,
            "primary": true,
            "hashes": { "sha512": sha512, "sha1": "" },
        }],
    }])
    .to_string()
    .into_bytes()
}

fn level_bytes(version: &str, enabled: &[&str], disabled: &[&str]) -> Vec<u8> {
    use fastnbt::Value;
    let strings = |entries: &[&str]| {
        Value::List(
            entries
                .iter()
                .map(|e| Value::String(e.to_string()))
                .collect(),
        )
    };
    let data = Value::Compound(
        [
            (
                "Version".to_string(),
                Value::Compound(
                    [("Name".to_string(), Value::String(version.to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ),
            ("LevelName".to_string(), Value::String("Temp".to_string())),
            (
                "DataPacks".to_string(),
                Value::Compound(
                    [
                        ("Enabled".to_string(), strings(enabled)),
                        ("Disabled".to_string(), strings(disabled)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let root = Value::Compound([("Data".to_string(), data)].into_iter().collect());
    level_dat::gzip(&fastnbt::to_bytes(&root).unwrap())
}

struct Fixture {
    _dir: tempfile::TempDir,
    game_dir: PathBuf,
    data_dir: PathBuf,
    stack_paths: StackPaths,
    icons: IconCache,
    hashes: HashCache,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(game_dir.join("resourcepacks")).unwrap();
    std::fs::write(
        game_dir.join("options.txt"),
        "version:4707\nresourcePacks:[]\n",
    )
    .unwrap();
    let world = game_dir.join("saves").join("alpha");
    std::fs::create_dir_all(world.join("datapacks")).unwrap();
    std::fs::write(
        world.join("level.dat"),
        level_bytes(WORLD_VERSION, &[], &[]),
    )
    .unwrap();
    let data_dir = dir.path().join("data");
    let icons_dir = dir.path().join("icons");
    let stack_paths = StackPaths::for_game_dir(
        &game_dir,
        &data_dir,
        BackupsConfig {
            enabled: true,
            keep: 5,
        },
    );
    Fixture {
        _dir: dir,
        game_dir,
        data_dir,
        stack_paths,
        icons: IconCache::new(&icons_dir),
        hashes: HashCache::new(),
    }
}

fn data_paths(fx: &Fixture) -> DataPaths {
    DataPaths::for_world(
        &mine_utils_core::WorldPaths::for_world(&fx.game_dir, "alpha").unwrap(),
        &fx.data_dir,
        BackupsConfig {
            enabled: true,
            keep: 5,
        },
    )
}

fn client(api: MockApi) -> (ModrinthClient<Arc<MockApi>>, Arc<MockApi>) {
    let api = Arc::new(api);
    (ModrinthClient::without_sleep(Arc::clone(&api)), api)
}

fn leftovers(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.contains("mine-utils-download") {
                    out.push(path.display().to_string());
                }
            }
        }
    }
    out
}

#[cfg(unix)]
#[test]
fn installs_ignore_siblings_with_nested_symlinks() {
    use std::os::unix::fs::symlink;

    for datapack in [false, true] {
        let fx = fixture();
        let (kind, url, filename, bytes, root) = if datapack {
            (
                "datapack",
                DATAPACK_URL,
                DATAPACK_FILE,
                datapack_zip(),
                fx.game_dir.join("saves/alpha/datapacks"),
            )
        } else {
            (
                "resourcepack",
                RESOURCE_URL,
                RESOURCE_FILE,
                resource_zip(),
                fx.game_dir.join("resourcepacks"),
            )
        };
        let sibling = root.join("sibling");
        std::fs::create_dir(&sibling).unwrap();
        symlink(fx.game_dir.join("options.txt"), sibling.join("link")).unwrap();
        let (client, _) = client(
            MockApi::new()
                .with(
                    &versions_url(),
                    versions_json(url, filename, &sha512_hex(&bytes), &[WORLD_VERSION]),
                )
                .with(url, bytes.clone()),
        );
        let prepared = prepare_modrinth_install(
            &client,
            &fx.game_dir,
            datapack.then_some("alpha"),
            PROJECT,
            VERSION,
            filename,
            kind,
        )
        .unwrap();
        let paths = data_paths(&fx);
        let outcome = finish_modrinth_install(
            prepared,
            &fx.stack_paths,
            datapack.then_some(&paths),
            &fx.data_dir,
            &fx.icons,
            &fx.hashes,
        )
        .unwrap();
        assert!(
            matches!(
                outcome,
                CoreInstallOutcome::ResourcePack {
                    outcome: mine_utils_core::ImportOutcome::Imported { .. },
                    ..
                } | CoreInstallOutcome::Datapack {
                    outcome: mine_utils_core::DatapackImportOutcome::Imported { .. },
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(std::fs::read(root.join(filename)).unwrap(), bytes);
    }
}

#[test]
fn resource_pack_installs_enables_and_records_provenance() {
    let fx = fixture();
    let bytes = resource_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&bytes),
                    &["1.21.10", "1.21.9"],
                ),
            )
            .with(RESOURCE_URL, bytes.clone()),
    );

    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("finish");

    let CoreInstallOutcome::ResourcePack {
        outcome,
        provenance_warning,
    } = outcome
    else {
        panic!("wrong outcome kind");
    };
    assert!(provenance_warning.is_none(), "{provenance_warning:?}");
    assert!(
        matches!(outcome, mine_utils_core::ImportOutcome::Imported { .. }),
        "{outcome:?}"
    );

    let dest = fx.game_dir.join("resourcepacks").join(RESOURCE_FILE);
    assert_eq!(std::fs::read(&dest).unwrap(), bytes, "API filename lands");
    let options = std::fs::read_to_string(fx.game_dir.join("options.txt")).unwrap();
    assert!(
        options.contains(&format!("file/{RESOURCE_FILE}")),
        "{options}"
    );
    let stack = resource_stack::load(&fx.stack_paths).unwrap();
    assert!(
        stack
            .snapshot()
            .resource
            .iter()
            .any(|e| e == &format!("file/{RESOURCE_FILE}")),
        "enabled in the stack"
    );

    let entry = provenance_lookup(&fx.data_dir, &sha256_hex(&bytes))
        .expect("lookup")
        .expect("provenance recorded");
    assert_eq!(entry.project_id, PROJECT);
    assert_eq!(entry.version_id, VERSION);
    assert_eq!(entry.file_name, RESOURCE_FILE);
    assert!(leftovers(&fx.game_dir).is_empty(), "no download temps left");
    assert_eq!(api.cdn_calls().len(), 1, "one download");
}

#[test]
fn datapack_installs_into_the_selected_world_and_enables() {
    let fx = fixture();
    let bytes = datapack_zip();
    let (client, _) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    DATAPACK_URL,
                    DATAPACK_FILE,
                    &sha512_hex(&bytes),
                    &[WORLD_VERSION],
                ),
            )
            .with(DATAPACK_URL, bytes.clone()),
    );
    let paths = data_paths(&fx);

    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("alpha"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect("prepare");
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        Some(&paths),
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("finish");

    let CoreInstallOutcome::Datapack {
        outcome,
        provenance_warning,
    } = outcome
    else {
        panic!("wrong outcome kind");
    };
    assert!(provenance_warning.is_none(), "{provenance_warning:?}");
    assert!(
        matches!(
            outcome,
            mine_utils_core::DatapackImportOutcome::Imported { .. }
        ),
        "{outcome:?}"
    );

    let dest = fx
        .game_dir
        .join("saves")
        .join("alpha")
        .join("datapacks")
        .join(DATAPACK_FILE);
    assert_eq!(std::fs::read(&dest).unwrap(), bytes);
    let parsed = level_dat::parse(&std::fs::read(paths.level_dat_path()).unwrap()).unwrap();
    assert!(
        parsed
            .enabled
            .iter()
            .any(|e| e == &format!("file/{DATAPACK_FILE}")),
        "enabled in level.dat: {:?}",
        parsed.enabled
    );
    let entry = provenance_lookup(&fx.data_dir, &sha256_hex(&bytes))
        .expect("lookup")
        .expect("provenance recorded");
    assert_eq!(entry.project_id, PROJECT);
    assert!(leftovers(&fx.game_dir).is_empty(), "no download temps left");
}

#[test]
fn identical_existing_file_reuses_without_download() {
    let fx = fixture();
    let bytes = resource_zip();
    std::fs::write(
        fx.game_dir.join("resourcepacks").join(RESOURCE_FILE),
        &bytes,
    )
    .unwrap();
    let (client, api) = client(MockApi::new().with(
        &versions_url(),
        versions_json(
            RESOURCE_URL,
            RESOURCE_FILE,
            &sha512_hex(&bytes),
            &["1.21.10"],
        ),
    ));

    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("finish");

    assert!(
        matches!(
            outcome,
            CoreInstallOutcome::ResourcePack {
                outcome: mine_utils_core::ImportOutcome::Imported { .. },
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(
        api.cdn_calls().is_empty(),
        "identical bytes are reused, not downloaded"
    );
    assert!(provenance_lookup(&fx.data_dir, &sha256_hex(&bytes))
        .expect("lookup")
        .is_some());
}

#[test]
fn colliding_existing_file_reports_collision_and_keeps_dest() {
    let fx = fixture();
    let installed = resource_zip();
    let other = datapack_zip();
    assert_ne!(installed, other);
    std::fs::write(
        fx.game_dir.join("resourcepacks").join(RESOURCE_FILE),
        &other,
    )
    .unwrap();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&installed),
                    &["1.21.10"],
                ),
            )
            .with(RESOURCE_URL, installed.clone()),
    );

    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("collision is an outcome, not an error");

    assert!(
        matches!(
            outcome,
            CoreInstallOutcome::ResourcePack {
                outcome: mine_utils_core::ImportOutcome::Collision { .. },
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read(fx.game_dir.join("resourcepacks").join(RESOURCE_FILE)).unwrap(),
        other,
        "the existing file is untouched"
    );
    assert!(provenance_lookup(&fx.data_dir, &sha256_hex(&installed))
        .expect("lookup")
        .is_none());
    assert!(
        api.cdn_calls().is_empty(),
        "a collision does not download the file"
    );
    assert!(leftovers(&fx.game_dir).is_empty(), "no download temps left");
}

#[cfg(unix)]
#[test]
fn ambiguous_lossy_name_is_refused_before_download() {
    use std::os::unix::ffi::OsStrExt;

    let fx = fixture();
    let packs = fx.game_dir.join("resourcepacks");
    let raw = packs.join(std::ffi::OsStr::from_bytes(b"Caf\xe9.zip"));
    std::fs::write(&raw, b"raw").unwrap();
    let lossy = "Caf\u{FFFD}.zip";
    let bytes = resource_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(RESOURCE_URL, lossy, &sha512_hex(&bytes), &["1.21.10"]),
            )
            .with(RESOURCE_URL, bytes),
    );

    let error = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        lossy,
        "resourcepack",
    )
    .expect_err("an ambiguous name is refused");

    assert_eq!(error.code(), "pack_non_utf8_name");
    assert!(api.cdn_calls().is_empty(), "nothing was downloaded");
    let mut names: Vec<_> = std::fs::read_dir(&packs)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, vec![raw.file_name().unwrap().to_owned()]);
}

#[test]
fn datapack_target_version_mismatch_is_refused_before_download() {
    let fx = fixture();
    let bytes = datapack_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    DATAPACK_URL,
                    DATAPACK_FILE,
                    &sha512_hex(&bytes),
                    &["1.20.1"],
                ),
            )
            .with(DATAPACK_URL, bytes.clone()),
    );

    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("alpha"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect_err("world is 1.21.10, version only lists 1.20.1");
    let message = err.to_string();
    assert!(message.contains("1.21.10"), "{message}");
    assert!(api.cdn_calls().is_empty(), "refused before any download");
    assert!(!fx
        .game_dir
        .join("saves")
        .join("alpha")
        .join("datapacks")
        .join(DATAPACK_FILE)
        .exists());
}

#[test]
fn locked_world_copies_but_leaves_the_datapack_disabled() {
    if !mine_utils_core::game_lock::lock_detection_supported() {
        eprintln!("skipping: POSIX lock probing is unsupported here");
        return;
    }
    let fx = fixture();
    let world_dir = fx.game_dir.join("saves").join("alpha");
    let lock_path = world_dir.join("session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("MINE_UTILS_LOCK_HOLDER", &lock_path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn the lock holder");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if mine_utils_core::game_lock::world_locked(&world_dir).is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the held lock is detected"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let bytes = datapack_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    DATAPACK_URL,
                    DATAPACK_FILE,
                    &sha512_hex(&bytes),
                    &[WORLD_VERSION],
                ),
            )
            .with(DATAPACK_URL, bytes.clone()),
    );
    let paths = data_paths(&fx);
    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("alpha"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect("a locked world still downloads");
    assert!(
        !api.cdn_calls().is_empty(),
        "the file is fetched, not refused"
    );
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        Some(&paths),
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("locked is an outcome, not an error");
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        matches!(
            outcome,
            CoreInstallOutcome::Datapack {
                outcome: mine_utils_core::DatapackImportOutcome::Locked { .. },
                ..
            }
        ),
        "{outcome:?}"
    );
    let dest = world_dir.join("datapacks").join(DATAPACK_FILE);
    assert_eq!(std::fs::read(&dest).unwrap(), bytes, "the file is copied");
    let parsed = level_dat::parse(&std::fs::read(paths.level_dat_path()).unwrap()).unwrap();
    assert!(
        !parsed.enabled.iter().any(|e| e.contains(DATAPACK_FILE)),
        "left disabled"
    );
    assert!(provenance_lookup(&fx.data_dir, &sha256_hex(&bytes))
        .expect("lookup")
        .is_some());
    assert!(leftovers(&fx.game_dir).is_empty());
}

#[test]
fn bad_identifiers_are_refused_without_network() {
    let fx = fixture();
    let (client, api) = client(MockApi::new());
    for bad in ["../x", "a/b", "", "v 1"] {
        let err = prepare_modrinth_install(
            &client,
            &fx.game_dir,
            None,
            bad,
            VERSION,
            RESOURCE_FILE,
            "resourcepack",
        )
        .expect_err("bad project id must fail");
        assert!(matches!(err, AppError::Modrinth(_)), "{bad}: {err}");
    }
    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "mod",
    )
    .expect_err("bad kind must fail");
    assert!(matches!(err, AppError::Modrinth(_)), "{err}");
    assert!(api.calls().is_empty(), "no request for bad identifiers");
}

/// Spawned helper: takes a POSIX write lock on `$MINE_UTILS_LOCK_HOLDER` and
/// holds it until killed. Same pattern as the game-lock tests: POSIX locks
/// are per process, so the holder must be a second process.
#[test]
fn lock_holder_helper() {
    let Ok(path) = std::env::var("MINE_UTILS_LOCK_HOLDER") else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open the lock file");
    let mut lock = libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &mut lock) };
    assert_eq!(result, 0, "the helper takes the write lock");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[test]
fn running_game_refuses_before_any_download() {
    if !mine_utils_core::game_lock::lock_detection_supported() {
        eprintln!("skipping: POSIX lock probing is unsupported here");
        return;
    }
    let fx = fixture();
    let lock_path = fx.game_dir.join("saves").join("alpha").join("session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("MINE_UTILS_LOCK_HOLDER", &lock_path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn the lock holder");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if mine_utils_core::game_lock::check(&fx.game_dir)
            .running
            .is_some()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the held lock is detected"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let bytes = resource_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&bytes),
                    &["1.21.10"],
                ),
            )
            .with(RESOURCE_URL, bytes.clone()),
    );
    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect_err("a running game refuses the install");
    let _ = child.kill();
    let _ = child.wait();

    assert!(matches!(err, AppError::GameRunning { .. }), "{err}");
    assert!(api.calls().is_empty(), "refused before any request");
    assert!(!fx
        .game_dir
        .join("resourcepacks")
        .join(RESOURCE_FILE)
        .exists());
    assert!(leftovers(&fx.game_dir).is_empty());
}

#[test]
fn game_started_after_prepare_refuses_resource_commit() {
    if !mine_utils_core::game_lock::lock_detection_supported() {
        return;
    }
    let fx = fixture();
    let bytes = resource_zip();
    let (client, _) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&bytes),
                    &[WORLD_VERSION],
                ),
            )
            .with(RESOURCE_URL, bytes.clone()),
    );
    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .unwrap();
    let lock_path = fx.game_dir.join("saves/alpha/session.lock");
    std::fs::write(&lock_path, b"\0").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("MINE_UTILS_LOCK_HOLDER", &lock_path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while mine_utils_core::game_lock::check(&fx.game_dir)
        .running
        .is_none()
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let err = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .unwrap_err();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(matches!(err, AppError::GameRunning { .. }), "{err}");
    assert!(!fx
        .game_dir
        .join("resourcepacks")
        .join(RESOURCE_FILE)
        .exists());
    assert!(leftovers(&fx.game_dir).is_empty());
    assert!(provenance_lookup(&fx.data_dir, &sha256_hex(&bytes))
        .unwrap()
        .is_none());
}

#[test]
fn datapack_typo_world_fails_before_any_request() {
    let fx = fixture();
    let (client, api) = client(MockApi::new());
    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("beta"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect_err("a typo world must fail");
    assert!(matches!(err, AppError::World { .. }), "{err}");
    assert!(api.calls().is_empty(), "no metadata or CDN request");
    assert!(!fx
        .game_dir
        .join("saves")
        .join("beta")
        .join("datapacks")
        .join(DATAPACK_FILE)
        .exists());
}

#[test]
fn datapack_world_without_level_dat_fails_before_any_request() {
    let fx = fixture();
    std::fs::create_dir_all(fx.game_dir.join("saves").join("gamma").join("datapacks")).unwrap();
    let (client, api) = client(MockApi::new());
    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("gamma"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect_err("a world without level.dat must fail");
    assert!(err.to_string().contains("no readable level.dat"), "{err}");
    assert!(api.calls().is_empty(), "no metadata or CDN request");
}

#[test]
fn datapack_corrupt_level_dat_fails_before_any_request() {
    let fx = fixture();
    std::fs::write(
        fx.game_dir.join("saves").join("alpha").join("level.dat"),
        b"garbage-not-gzip",
    )
    .unwrap();
    let (client, api) = client(MockApi::new());
    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("alpha"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect_err("a corrupt level.dat must fail");
    assert!(err.to_string().contains("no readable level.dat"), "{err}");
    assert!(api.calls().is_empty(), "no metadata or CDN request");
    assert!(!fx
        .game_dir
        .join("saves")
        .join("alpha")
        .join("datapacks")
        .join(DATAPACK_FILE)
        .exists());
}

#[cfg(unix)]
#[test]
fn symlinked_datapacks_refuses_before_any_request() {
    let fx = fixture();
    let world = fx.game_dir.join("saves").join("alpha");
    let link_target = fx._dir.path().join("outside-packs");
    std::fs::create_dir_all(&link_target).unwrap();
    std::fs::remove_dir_all(world.join("datapacks")).unwrap();
    std::os::unix::fs::symlink(&link_target, world.join("datapacks")).unwrap();

    let bytes = datapack_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    DATAPACK_URL,
                    DATAPACK_FILE,
                    &sha512_hex(&bytes),
                    &[WORLD_VERSION],
                ),
            )
            .with(DATAPACK_URL, bytes),
    );
    let result = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("alpha"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    );
    let requests = api.calls();
    let err = result.expect_err(&format!(
        "a symlinked datapacks folder must refuse; requests made: {requests:?}"
    ));
    assert!(matches!(err, AppError::World { .. }), "{err}");
    assert!(
        err.to_string()
            .contains("datapacks is a symlink and is never followed"),
        "{err}"
    );
    assert!(requests.is_empty(), "no API or CDN request: {requests:?}");
    assert_eq!(
        std::fs::read_dir(&link_target).unwrap().count(),
        0,
        "the link target stays empty"
    );
}

#[test]
fn fresh_game_dir_without_options_txt_refuses_before_download() {
    let fx = fixture();
    std::fs::remove_file(fx.game_dir.join("options.txt")).unwrap();
    let bytes = resource_zip();
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&bytes),
                    &["1.21.10"],
                ),
            )
            .with(RESOURCE_URL, bytes),
    );
    let err = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect_err("a fresh game dir must fail before download");
    assert!(err.to_string().contains("options.txt"), "{err}");
    assert!(api.calls().is_empty(), "no metadata or CDN request");
    assert!(!fx
        .game_dir
        .join("resourcepacks")
        .join(RESOURCE_FILE)
        .exists());
    assert!(leftovers(&fx.game_dir).is_empty(), "no download temps");
}

#[test]
fn existing_dest_swapped_between_prepare_and_finish_is_refused() {
    let fx = fixture();
    let bytes = resource_zip();
    let dest = fx.game_dir.join("resourcepacks").join(RESOURCE_FILE);
    std::fs::write(&dest, &bytes).unwrap();
    let original_sha256 = sha256_hex(&bytes);
    let (client, api) = client(MockApi::new().with(
        &versions_url(),
        versions_json(
            RESOURCE_URL,
            RESOURCE_FILE,
            &sha512_hex(&bytes),
            &["1.21.10"],
        ),
    ));

    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    // Swap the file between prepare and finish.
    std::fs::write(&dest, datapack_zip()).unwrap();

    let err = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect_err("a swapped dest must not be reused");
    assert!(err.to_string().contains("SHA-512 mismatch"), "{err}");
    assert!(api.cdn_calls().is_empty(), "no download ran");
    assert!(
        provenance_lookup(&fx.data_dir, &original_sha256)
            .expect("lookup")
            .is_none(),
        "no provenance for the wrong bytes"
    );
}

#[test]
fn datapack_finish_without_data_paths_errors_instead_of_panicking() {
    let fx = fixture();
    let bytes = datapack_zip();
    let (client, _) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    DATAPACK_URL,
                    DATAPACK_FILE,
                    &sha512_hex(&bytes),
                    &[WORLD_VERSION],
                ),
            )
            .with(DATAPACK_URL, bytes),
    );
    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        Some("alpha"),
        PROJECT,
        VERSION,
        DATAPACK_FILE,
        "datapack",
    )
    .expect("prepare");
    let err = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect_err("missing datapack context must error");
    assert!(matches!(err, AppError::World { .. }), "{err}");
}

#[test]
fn empty_and_malformed_sha512_are_refused_before_download() {
    for bad in ["", "zz", &"0".repeat(10), &"x".repeat(128)] {
        let fx = fixture();
        let (client, api) = client(MockApi::new().with(
            &versions_url(),
            versions_json(RESOURCE_URL, RESOURCE_FILE, bad, &["1.21.10"]),
        ));
        let err = prepare_modrinth_install(
            &client,
            &fx.game_dir,
            None,
            PROJECT,
            VERSION,
            RESOURCE_FILE,
            "resourcepack",
        )
        .expect_err("bad hash must fail");
        assert!(err.to_string().contains("SHA-512"), "{bad}: {err}");
        assert!(api.cdn_calls().is_empty(), "no download for {bad:?}");
        assert!(!fx
            .game_dir
            .join("resourcepacks")
            .join(RESOURCE_FILE)
            .exists());
    }
}

#[test]
fn non_zip_pack_files_are_refused_before_download() {
    // A datapack version whose primary file is a mod `.jar` can never
    // install as a datapack; refuse before any CDN request.
    for (kind, world, dest_file) in [
        ("datapack", Some("alpha"), DATAPACK_FILE),
        ("resourcepack", None, RESOURCE_FILE),
    ] {
        let fx = fixture();
        let jar_name = if kind == "datapack" {
            "cool-mod-2.0.jar"
        } else {
            "fresh-1.9.jar"
        };
        let jar_url = if kind == "datapack" {
            "https://cdn.modrinth.com/data/def/cool-mod-2.0.jar"
        } else {
            "https://cdn.modrinth.com/data/abc/fresh-1.9.jar"
        };
        let fake_sha = &"a".repeat(128);
        let (client, api) = client(MockApi::new().with(
            &versions_url(),
            versions_json(jar_url, jar_name, fake_sha, &[WORLD_VERSION]),
        ));
        let err = prepare_modrinth_install(
            &client,
            &fx.game_dir,
            world,
            PROJECT,
            VERSION,
            jar_name,
            kind,
        )
        .expect_err(&format!("a {kind} .jar must be refused"));
        assert!(err.to_string().contains("not a .zip"), "{err}");
        assert!(
            api.cdn_calls().is_empty(),
            "refused before any download for {kind}"
        );
        let dest = if kind == "datapack" {
            fx.game_dir
                .join("saves")
                .join("alpha")
                .join("datapacks")
                .join(jar_name)
        } else {
            fx.game_dir.join("resourcepacks").join(jar_name)
        };
        assert!(!dest.exists(), "nothing lands for {kind}");
        let _ = dest_file;
    }
}

#[test]
fn malformed_pack_commit_is_removed_on_import_failure() {
    let fx = fixture();
    // Valid zip root but invalid pack metadata: commits, then import fails.
    let bad = zip_bytes(&[
        ("assets/minecraft/textures/a.png", b"asset"),
        ("pack.mcmeta", b"this is not json"),
    ]);
    let (client, api) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(RESOURCE_URL, RESOURCE_FILE, &sha512_hex(&bad), &["1.21.10"]),
            )
            .with(RESOURCE_URL, bad),
    );
    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    let err = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect_err("import must fail");
    let _ = api;
    assert!(
        !fx.game_dir
            .join("resourcepacks")
            .join(RESOURCE_FILE)
            .exists(),
        "the fresh commit is removed, got error {err}"
    );
    assert!(leftovers(&fx.game_dir).is_empty(), "no download temps");
}

#[test]
fn provenance_write_failure_warns_but_install_succeeds() {
    let fx = fixture();
    std::fs::create_dir_all(&fx.data_dir).unwrap();
    std::fs::write(fx.data_dir.join("provenance.json"), b"corrupt{").unwrap();
    let bytes = resource_zip();
    let (client, _) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&bytes),
                    &["1.21.10"],
                ),
            )
            .with(RESOURCE_URL, bytes.clone()),
    );
    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("install succeeds despite the provenance failure");
    let CoreInstallOutcome::ResourcePack {
        outcome,
        provenance_warning,
    } = outcome
    else {
        panic!("wrong outcome kind");
    };
    assert!(
        matches!(outcome, mine_utils_core::ImportOutcome::Imported { .. }),
        "{outcome:?}"
    );
    let warning = provenance_warning.expect("a warning is reported");
    assert!(warning.contains("provenance"), "{warning}");
    assert_eq!(
        std::fs::read(fx.game_dir.join("resourcepacks").join(RESOURCE_FILE)).unwrap(),
        bytes
    );
    let options = std::fs::read_to_string(fx.game_dir.join("options.txt")).unwrap();
    assert!(
        options.contains(&format!("file/{RESOURCE_FILE}")),
        "{options}"
    );
    assert!(leftovers(&fx.game_dir).is_empty(), "no download temps");
}

#[test]
fn identical_bytes_under_a_different_filename_installs_a_second_copy() {
    // Contract: the picker installs by API filename. Identical bytes already
    // present under a different name do not block the install; the new file
    // is committed as an intentional second copy.
    let fx = fixture();
    let bytes = resource_zip();
    std::fs::write(
        fx.game_dir.join("resourcepacks").join("other-name.zip"),
        &bytes,
    )
    .unwrap();
    let (client, _) = client(
        MockApi::new()
            .with(
                &versions_url(),
                versions_json(
                    RESOURCE_URL,
                    RESOURCE_FILE,
                    &sha512_hex(&bytes),
                    &["1.21.10"],
                ),
            )
            .with(RESOURCE_URL, bytes.clone()),
    );
    let prepared = prepare_modrinth_install(
        &client,
        &fx.game_dir,
        None,
        PROJECT,
        VERSION,
        RESOURCE_FILE,
        "resourcepack",
    )
    .expect("prepare");
    let outcome = finish_modrinth_install(
        prepared,
        &fx.stack_paths,
        None,
        &fx.data_dir,
        &fx.icons,
        &fx.hashes,
    )
    .expect("finish");
    assert!(
        matches!(
            outcome,
            CoreInstallOutcome::ResourcePack {
                outcome: mine_utils_core::ImportOutcome::Imported { .. },
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read(fx.game_dir.join("resourcepacks").join(RESOURCE_FILE)).unwrap(),
        bytes,
    );
    assert!(
        fx.game_dir
            .join("resourcepacks")
            .join("other-name.zip")
            .exists(),
        "the pre-existing file is untouched"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_or_dangling_symlink_at_the_name_reports_collision() {
    type MakeRival = fn(&Path);
    let rivals: [(&str, MakeRival); 2] = [
        ("dir", |path| std::fs::create_dir(path).unwrap()),
        ("dangling symlink", |path| {
            std::os::unix::fs::symlink("missing.zip", path).unwrap()
        }),
    ];
    let seen: Vec<String> = rivals
        .into_iter()
        .map(|(label, make_rival)| {
            let fx = fixture();
            let bytes = resource_zip();
            let dest = fx.game_dir.join("resourcepacks").join(RESOURCE_FILE);
            make_rival(&dest);
            let (client, _api) = client(
                MockApi::new()
                    .with(
                        &versions_url(),
                        versions_json(
                            RESOURCE_URL,
                            RESOURCE_FILE,
                            &sha512_hex(&bytes),
                            &["1.21.10"],
                        ),
                    )
                    .with(RESOURCE_URL, bytes),
            );

            let prepared = prepare_modrinth_install(
                &client,
                &fx.game_dir,
                None,
                PROJECT,
                VERSION,
                RESOURCE_FILE,
                "resourcepack",
            )
            .expect("prepare");
            let outcome = match finish_modrinth_install(
                prepared,
                &fx.stack_paths,
                None,
                &fx.data_dir,
                &fx.icons,
                &fx.hashes,
            ) {
                Ok(CoreInstallOutcome::ResourcePack { outcome, .. }) => format!("{outcome:?}"),
                Ok(other) => format!("{other:?}"),
                Err(err) => format!("error: {err}"),
            };
            let kept = std::fs::symlink_metadata(&dest).unwrap().file_type();
            format!(
                "{label}: {outcome}; kept dir {} symlink {}; temps {}",
                kept.is_dir(),
                kept.is_symlink(),
                leftovers(&fx.game_dir).len()
            )
        })
        .collect();

    assert_eq!(
        seen,
        [
            "dir: Collision { name: \"fresh-1.9.zip\" }; kept dir true symlink false; temps 0",
            "dangling symlink: Collision { name: \"fresh-1.9.zip\" }; kept dir false symlink true; temps 0",
        ]
    );
}

mod cancellation {
    use super::*;
    use mine_utils_core::download::InstallControl;
    use mine_utils_core::install::prepare_modrinth_install_controlled;
    use std::io::Read;
    use std::time::{Duration, Instant};

    struct LocalCdn {
        metadata: Vec<u8>,
        url: String,
    }

    impl Transport for LocalCdn {
        fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
            if url == versions_url() {
                return Ok(RawResponse {
                    status: 200,
                    headers: vec![],
                    body: Box::new(std::io::Cursor::new(self.metadata.clone())),
                });
            }
            assert_eq!(url, RESOURCE_URL);
            let response = reqwest::blocking::Client::new()
                .get(&self.url)
                .send()
                .map_err(|err| ModrinthError::Offline {
                    reason: err.to_string(),
                })?;
            Ok(RawResponse {
                status: response.status().as_u16(),
                headers: vec![],
                body: Box::new(response),
            })
        }
    }

    fn local_http_replay(cancel: bool) {
        let fx = fixture();
        let bytes = resource_zip();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/pack.zip", listener.local_addr().unwrap());
        let metadata = versions_json(
            RESOURCE_URL,
            RESOURCE_FILE,
            &sha512_hex(&bytes),
            &["1.21.10"],
        );
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            for (index, chunk) in bytes.chunks(8).enumerate() {
                if stream.write_all(chunk).is_err() {
                    break;
                }
                if index == 0 {
                    started_tx.send(()).unwrap();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let control = InstallControl::with_timeout(if cancel {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(70)
        });
        let canceller = if cancel {
            let control = control.clone();
            Some(std::thread::spawn(move || {
                started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                std::thread::sleep(Duration::from_millis(20));
                assert!(control.cancel());
            }))
        } else {
            None
        };
        let client = ModrinthClient::without_sleep(LocalCdn { metadata, url });
        let started = Instant::now();
        let result = prepare_modrinth_install_controlled(
            &client,
            &fx.game_dir,
            None,
            PROJECT,
            VERSION,
            RESOURCE_FILE,
            "resourcepack",
            &control,
        );
        let elapsed = started.elapsed();
        if let Some(canceller) = canceller {
            canceller.join().unwrap();
        }
        let result = result.and_then(|prepared| {
            finish_modrinth_install(
                prepared,
                &fx.stack_paths,
                None,
                &fx.data_dir,
                &fx.icons,
                &fx.hashes,
            )
        });
        let files: Vec<_> = std::fs::read_dir(fx.game_dir.join("resourcepacks"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let options = std::fs::read_to_string(fx.game_dir.join("options.txt")).unwrap();
        println!("local HTTP cancel={cancel} result={result:?} elapsed={elapsed:?} files={files:?} options={options:?}");
        server.join().unwrap();
        assert_eq!(
            result.unwrap_err().code(),
            if cancel {
                "download_cancelled"
            } else {
                "download_timeout"
            }
        );
        assert_eq!(files, Vec::<std::ffi::OsString>::new());
        assert_eq!(options, "version:4707\nresourcePacks:[]\n");
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn local_http_cancel_mid_download() {
        local_http_replay(true);
    }

    #[test]
    fn local_http_total_deadline() {
        local_http_replay(false);
    }

    #[test]
    fn cancel_before_finish_cleans_verified_temp_without_enabling() {
        let fx = fixture();
        let bytes = resource_zip();
        let (client, _) = client(
            MockApi::new()
                .with(
                    &versions_url(),
                    versions_json(
                        RESOURCE_URL,
                        RESOURCE_FILE,
                        &sha512_hex(&bytes),
                        &["1.21.10"],
                    ),
                )
                .with(RESOURCE_URL, bytes),
        );
        let control = InstallControl::default();
        let prepared = prepare_modrinth_install_controlled(
            &client,
            &fx.game_dir,
            None,
            PROJECT,
            VERSION,
            RESOURCE_FILE,
            "resourcepack",
            &control,
        )
        .unwrap();
        assert!(control.cancel());
        let err = finish_modrinth_install(
            prepared,
            &fx.stack_paths,
            None,
            &fx.data_dir,
            &fx.icons,
            &fx.hashes,
        )
        .unwrap_err();
        assert_eq!(err.code(), "download_cancelled");
        assert_eq!(
            std::fs::read_dir(fx.game_dir.join("resourcepacks"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            std::fs::read_to_string(fx.game_dir.join("options.txt")).unwrap(),
            "version:4707\nresourcePacks:[]\n"
        );
    }

    #[test]
    fn cancel_after_commit_keeps_the_complete_enabled_install() {
        let fx = fixture();
        let bytes = resource_zip();
        let sha256 = sha256_hex(&bytes);
        let (client, _) = client(
            MockApi::new()
                .with(
                    &versions_url(),
                    versions_json(
                        RESOURCE_URL,
                        RESOURCE_FILE,
                        &sha512_hex(&bytes),
                        &["1.21.10"],
                    ),
                )
                .with(RESOURCE_URL, bytes.clone()),
        );
        let control = InstallControl::default();
        let prepared = prepare_modrinth_install_controlled(
            &client,
            &fx.game_dir,
            None,
            PROJECT,
            VERSION,
            RESOURCE_FILE,
            "resourcepack",
            &control,
        )
        .unwrap();
        finish_modrinth_install(
            prepared,
            &fx.stack_paths,
            None,
            &fx.data_dir,
            &fx.icons,
            &fx.hashes,
        )
        .unwrap();
        assert!(!control.cancel());
        assert_eq!(
            std::fs::read(fx.game_dir.join("resourcepacks").join(RESOURCE_FILE)).unwrap(),
            bytes
        );
        assert!(std::fs::read_to_string(fx.game_dir.join("options.txt"))
            .unwrap()
            .contains("file/fresh-1.9.zip"));
        assert!(provenance_lookup(&fx.data_dir, &sha256).unwrap().is_some());
        assert!(leftovers(&fx.game_dir).is_empty());
    }

    #[test]
    fn cancel_and_commit_have_exactly_one_winner() {
        for _ in 0..100 {
            let control = InstallControl::default();
            let worker = control.clone();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let other_barrier = barrier.clone();
            let canceller = std::thread::spawn(move || {
                other_barrier.wait();
                worker.cancel()
            });
            barrier.wait();
            let commit = control.begin_commit();
            let cancelled = canceller.join().unwrap();
            assert_eq!(commit.is_ok(), !cancelled);
            if cancelled {
                assert_eq!(commit.unwrap_err().code(), "download_cancelled");
            }
        }
    }
}
