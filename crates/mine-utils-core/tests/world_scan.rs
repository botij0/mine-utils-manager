use std::collections::HashMap;
use std::path::Path;

use fastnbt::Value;
use mine_utils_core::{level_dat, IconCache};

fn compound(pairs: Vec<(&str, Value)>) -> Value {
    Value::Compound(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn level_bytes(version: Option<&str>, with_packs: bool, level_name: Option<&str>) -> Vec<u8> {
    let mut data: HashMap<String, Value> = HashMap::new();
    if let Some(version) = version {
        data.insert(
            "Version".to_string(),
            compound(vec![("Name", Value::String(version.to_string()))]),
        );
    }
    if with_packs {
        data.insert(
            "DataPacks".to_string(),
            compound(vec![
                (
                    "Enabled",
                    Value::List(vec![Value::String("vanilla".to_string())]),
                ),
                ("Disabled", Value::List(Vec::new())),
            ]),
        );
    }
    if let Some(name) = level_name {
        data.insert("LevelName".to_string(), Value::String(name.to_string()));
    }
    let root = Value::Compound(
        [("Data".to_string(), Value::Compound(data))]
            .into_iter()
            .collect(),
    );
    level_dat::gzip(&fastnbt::to_bytes(&root).unwrap())
}

fn game_dir() -> (tempfile::TempDir, std::path::PathBuf, IconCache) {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    std::fs::create_dir_all(game.join("saves")).unwrap();
    let icons = IconCache::new(&dir.path().join("cache"));
    (dir, game, icons)
}

fn add_world(game: &Path, name: &str, level: Option<Vec<u8>>, icon: bool) {
    let world = game.join("saves").join(name);
    std::fs::create_dir_all(&world).unwrap();
    if let Some(bytes) = level {
        std::fs::write(world.join("level.dat"), bytes).unwrap();
    }
    if icon {
        let png = tiny_png();
        std::fs::write(world.join("icon.png"), png).unwrap();
    }
}

fn tiny_png() -> Vec<u8> {
    // A minimal 1x1 PNG. The cache stores bytes, it does not decode pixels.
    vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8,
        0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0xcc, 0x59, 0xe7, 0x00, 0x00, 0x00,
        0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ]
}

#[test]
fn missing_level_dat_is_skipped() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "real",
        Some(level_bytes(Some("1.21.10"), true, Some("Real"))),
        false,
    );
    add_world(&game, "empty", None, false);
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    assert_eq!(worlds.len(), 1);
    assert_eq!(worlds[0].dir_name, "real");
}

#[test]
fn corrupt_gzip_is_an_error_card() {
    let (_dir, game, icons) = game_dir();
    add_world(&game, "bad", Some(b"not gzip".to_vec()), false);
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    assert_eq!(worlds.len(), 1);
    let issue = worlds[0]
        .issue
        .as_ref()
        .expect("corrupt world has an issue");
    assert_eq!(issue.code, "level_dat");
    assert!(worlds[0].version.is_none());
}

#[test]
fn missing_version_is_an_error_card() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "noversion",
        Some(level_bytes(None, true, Some("NoVer"))),
        false,
    );
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    assert!(worlds[0].issue.is_some());
}

#[test]
fn missing_icon_is_none_not_an_error() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "noicon",
        Some(level_bytes(Some("1.21.10"), true, Some("NoIcon"))),
        false,
    );
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    assert!(worlds[0].icon.is_none());
    assert!(worlds[0].issue.is_none());
}

#[test]
fn icon_is_cached_through_the_shared_cache() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "iconed",
        Some(level_bytes(Some("1.21.10"), true, Some("Iconed"))),
        true,
    );
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    let icon = worlds[0].icon.as_ref().expect("icon is cached");
    assert!(icon.path.starts_with(icons.dir()));
    assert!(icon.path.is_file());
}

#[test]
fn seventeen_worlds_scan_with_error_isolation() {
    let (_dir, game, icons) = game_dir();
    for index in 0..17 {
        let name = format!("world-{index:02}");
        let level = if index == 7 {
            Some(b"corrupt".to_vec())
        } else {
            Some(level_bytes(
                Some("1.21.10"),
                true,
                Some(&format!("World {index}")),
            ))
        };
        add_world(&game, &name, level, index % 3 == 0);
    }
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    assert_eq!(worlds.len(), 17);
    let errors: Vec<_> = worlds.iter().filter(|w| w.issue.is_some()).collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].dir_name, "world-07");
    let good = worlds.iter().filter(|w| w.issue.is_none()).count();
    assert_eq!(good, 16);
}

#[test]
fn version_resolves_to_the_data_format() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "w",
        Some(level_bytes(Some("1.21.10"), true, Some("W"))),
        false,
    );
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    let version = worlds[0].version.as_ref().unwrap();
    assert_eq!(version.name, "1.21.10");
    assert_eq!(
        version.data_format,
        Some(mine_utils_core::pack::FormatVersion::from_major(88))
    );
}

#[test]
fn a_year_based_world_version_resolves_to_its_data_format() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "w",
        Some(level_bytes(Some("26.1.2"), true, Some("W"))),
        false,
    );
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    let version = serde_json::to_value(worlds[0].version.as_ref().unwrap()).unwrap();
    assert_eq!(
        version,
        serde_json::json!({
            "name": "26.1.2",
            "data_format": { "major": 101, "minor": 1 },
            "newest_known_version": null,
        })
    );
}

#[test]
fn a_world_newer_than_the_table_names_the_newest_known_version() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "w",
        Some(level_bytes(Some("26.9"), true, Some("W"))),
        false,
    );
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    let version = serde_json::to_value(worlds[0].version.as_ref().unwrap()).unwrap();
    assert_eq!(
        version,
        serde_json::json!({
            "name": "26.9",
            "data_format": null,
            "newest_known_version": "26.3",
        })
    );
}

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

fn spawn_lock_holder(path: &Path) -> std::process::Child {
    use std::process::{Command, Stdio};
    Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("MINE_UTILS_LOCK_HOLDER", path)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn the lock holder")
}

#[test]
fn locked_world_reports_its_lock() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "locked",
        Some(level_bytes(Some("1.21.10"), true, Some("Locked"))),
        false,
    );
    let lock_path = game.join("saves").join("locked").join("session.lock");
    std::fs::write(&lock_path, b"lock").unwrap();
    // POSIX locks are per process, so a helper process holds the lock while
    // the test probes it.
    let mut held = spawn_lock_holder(&lock_path);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let world = loop {
        let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
        let world = worlds.into_iter().find(|w| w.dir_name == "locked").unwrap();
        if world.world_locked.is_some() || std::time::Instant::now() > deadline {
            break world;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let _ = held.kill();
    let _ = held.wait();
    assert!(world.world_locked.is_some(), "the held lock is reported");
    assert!(!world.stale_lock);
}

#[test]
fn stale_lock_is_not_locked() {
    let (_dir, game, icons) = game_dir();
    add_world(
        &game,
        "stale",
        Some(level_bytes(Some("1.21.10"), true, Some("Stale"))),
        false,
    );
    std::fs::write(
        game.join("saves").join("stale").join("session.lock"),
        b"lock",
    )
    .unwrap();
    let worlds = mine_utils_core::world::scan_worlds(&game, &icons);
    let world = worlds.iter().find(|w| w.dir_name == "stale").unwrap();
    assert!(world.world_locked.is_none(), "an unheld lock is stale");
    assert!(world.stale_lock);
}
