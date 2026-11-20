//! The `mine-utils-probe` binary run against temp game dirs, the way an
//! operator runs it. Its JSON output is what these tests read.

use std::path::PathBuf;
use std::process::Command;

use fastnbt::Value;
use mine_utils_core::config::BackupsConfig;
use mine_utils_core::{level_backup, level_dat};
use serde_json::json;

fn compound(pairs: Vec<(&str, Value)>) -> Value {
    Value::Compound(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn level_bytes(version: &str) -> Vec<u8> {
    let data = compound(vec![
        (
            "Version",
            compound(vec![("Name", Value::String(version.to_string()))]),
        ),
        ("LevelName", Value::String("World".to_string())),
        (
            "DataPacks",
            compound(vec![
                (
                    "Enabled",
                    Value::List(vec![Value::String("vanilla".to_string())]),
                ),
                ("Disabled", Value::List(Vec::new())),
            ]),
        ),
    ]);
    let root = compound(vec![("Data", data)]);
    level_dat::gzip(&fastnbt::to_bytes(&root).unwrap())
}

/// A temp home for the probe's XDG dirs and a game dir with an empty `saves`.
struct Install {
    _dir: tempfile::TempDir,
    home: PathBuf,
    game: PathBuf,
}

fn install() -> Install {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let game = dir.path().join("game");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(game.join("saves")).unwrap();
    Install {
        _dir: dir,
        home,
        game,
    }
}

impl Install {
    fn write_world(&self, world: &str, level: &[u8]) {
        let dir = self.game.join("saves").join(world);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("level.dat"), level).unwrap();
    }

    /// Writes a backup of `level` for `world` through the library call the app
    /// uses, so the probe finds it in the backups folder it resolves.
    fn backup(&self, world: &str, level: &[u8]) -> String {
        // The app's data dir is `$XDG_DATA_HOME/mine-utils` on Linux.
        let data_dir = self.home.join("data").join("mine-utils");
        let root = level_backup::root_for(&data_dir, &self.game);
        let outcome =
            level_backup::create(&root, world, level, &BackupsConfig::default()).unwrap();
        outcome
            .created
            .expect("backups are on by default")
            .file_name
    }

    fn probe(&self, args: &[&str]) -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_mine-utils-probe"))
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_CACHE_HOME", self.home.join("cache"))
            .output()
            .expect("run mine-utils-probe");
        assert!(
            output.status.success(),
            "probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("probe prints JSON")
    }

    fn scan_worlds(&self) -> serde_json::Value {
        let game = self.game.to_str().unwrap();
        self.probe(&["scan-worlds", game])
    }

    fn restore_level_backup(&self, world: &str, backup: &str) -> serde_json::Value {
        let game = self.game.to_str().unwrap();
        self.probe(&["restore-level-backup", world, backup, game])
    }
}

#[test]
fn scan_worlds_names_the_newest_known_version_for_a_world_newer_than_the_table() {
    let install = install();
    install.write_world("future", &level_bytes("26.9"));
    let out = install.scan_worlds();
    assert_eq!(
        out["worlds"][0]["version"],
        json!({ "name": "26.9", "data_format": null, "newest_known_version": "26.3" })
    );
}

#[test]
fn restore_level_backup_names_the_newest_known_version_for_a_world_newer_than_the_table() {
    let install = install();
    install.write_world("known", &level_bytes("1.21.10"));
    let backup = install.backup("known", &level_bytes("26.9"));
    let out = install.restore_level_backup("known", &backup);
    assert_eq!(out["world_format"], json!(null));
    assert_eq!(out["newest_known_version"], json!("26.3"));
}

#[test]
fn restore_level_backup_reports_a_null_newest_known_version_for_a_covered_world() {
    let install = install();
    install.write_world("known", &level_bytes("26.9"));
    let backup = install.backup("known", &level_bytes("1.21.10"));
    let out = install.restore_level_backup("known", &backup);
    assert_eq!(out["world_format"], json!("88"));
    assert_eq!(out.get("newest_known_version"), Some(&json!(null)));
}
