use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Where a game dir came from. The frontend mirrors these names exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    Std,
    Flatpak,
    PrismLauncher,
    MultiMc,
    Manual,
}

/// Identity of a game dir. `path` is canonical so symlinks resolve to one entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GameDir {
    pub path: PathBuf,
    pub source: DiscoverySource,
}

/// A game dir plus its on-disk state at the time of the listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameDirEntry {
    pub path: PathBuf,
    pub source: DiscoverySource,
    pub present: bool,
}

const MARKERS: [&str; 3] = ["versions", "saves", "resourcepacks"];

/// A game dir is a directory with at least one of the marker subdirectories.
pub fn is_game_dir(path: &Path) -> bool {
    path.is_dir() && MARKERS.iter().any(|marker| path.join(marker).is_dir())
}

/// Accept a user-supplied path. The result carries the canonical path so two
/// spellings of one dir never create two entries.
pub fn validate_game_dir(path: &Path) -> Result<GameDir, AppError> {
    let canonical = std::fs::canonicalize(path).map_err(|err| AppError::InvalidGameDir {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    if !is_game_dir(&canonical) {
        return Err(AppError::InvalidGameDir {
            path: path.to_path_buf(),
            reason: "expected one of versions, saves, or resourcepacks".to_string(),
        });
    }
    Ok(GameDir {
        path: canonical,
        source: DiscoverySource::Manual,
    })
}

/// Scan the known launcher layouts under `home`. Dirs are checked in source
/// order and deduped by canonical path, so a symlinked instance counts once.
pub fn discover_game_dirs(home: &Path) -> Vec<GameDir> {
    let mut found: Vec<GameDir> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    add_candidate(
        &mut found,
        &mut seen,
        home.join(".minecraft"),
        DiscoverySource::Std,
    );
    add_candidate(
        &mut found,
        &mut seen,
        home.join(".var/app/com.mojang.Minecraft/data/minecraft"),
        DiscoverySource::Flatpak,
    );
    let launchers = [
        (
            home.join(".local/share/PrismLauncher/instances"),
            DiscoverySource::PrismLauncher,
        ),
        (
            home.join(".local/share/multimc/instances"),
            DiscoverySource::MultiMc,
        ),
    ];
    for (instances, source) in launchers {
        for instance in instance_dirs(&instances) {
            add_candidate(&mut found, &mut seen, instance.join(".minecraft"), source);
        }
    }
    found
}

/// Merge discovered dirs with the dirs stored in config. The first spelling of
/// a canonical path wins, so discovery keeps its richer source label.
pub fn merge_game_dirs(discovered: Vec<GameDir>, stored: &[GameDir]) -> Vec<GameDir> {
    let mut merged = discovered;
    let mut seen: std::collections::HashSet<PathBuf> =
        merged.iter().map(|dir| dir.path.clone()).collect();
    for dir in stored {
        let key = std::fs::canonicalize(&dir.path).unwrap_or_else(|_| dir.path.clone());
        if seen.insert(key) {
            merged.push(dir.clone());
        }
    }
    merged
}

pub fn entry_for(dir: &GameDir) -> GameDirEntry {
    GameDirEntry {
        path: dir.path.clone(),
        source: dir.source,
        present: is_game_dir(&dir.path),
    }
}

fn add_candidate(
    found: &mut Vec<GameDir>,
    seen: &mut std::collections::HashSet<PathBuf>,
    path: PathBuf,
    source: DiscoverySource,
) {
    if !is_game_dir(&path) {
        return;
    }
    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if seen.insert(key.clone()) {
        found.push(GameDir { path: key, source });
    }
}

fn instance_dirs(instances: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(instances) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game_dir(root: &Path, name: &str, marker: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.join(marker)).unwrap();
        path
    }

    #[test]
    fn accepts_each_marker() {
        let root = tempfile::tempdir().unwrap();
        for marker in MARKERS {
            let path = make_game_dir(root.path(), marker, marker);
            assert!(is_game_dir(&path), "{marker} marker should make a game dir");
        }
    }

    #[test]
    fn rejects_plain_and_missing_dirs() {
        let root = tempfile::tempdir().unwrap();
        let plain = root.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert!(!is_game_dir(&plain));
        assert!(!is_game_dir(&root.path().join("missing")));

        let err = validate_game_dir(&plain).unwrap_err();
        assert_eq!(err.code(), "invalid_game_dir");
        assert!(
            err.to_string().contains(&plain.display().to_string()),
            "error names the path: {err}"
        );
        let err = validate_game_dir(&root.path().join("missing")).unwrap_err();
        assert!(
            err.to_string().contains("missing"),
            "error names the path: {err}"
        );
    }

    #[test]
    fn validate_canonicalizes_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let real = make_game_dir(root.path(), "real", "saves");
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let dir = validate_game_dir(&link).unwrap();
        assert_eq!(dir.path, std::fs::canonicalize(&real).unwrap());
        assert_eq!(dir.source, DiscoverySource::Manual);
    }

    #[test]
    fn discovery_dedupes_symlinked_dirs() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        make_game_dir(&home, ".minecraft", "versions");
        let instances = home.join(".local/share/PrismLauncher/instances/one");
        std::fs::create_dir_all(&instances).unwrap();
        std::os::unix::fs::symlink(home.join(".minecraft"), instances.join(".minecraft")).unwrap();

        let found = discover_game_dirs(&home);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, DiscoverySource::Std);
    }

    #[test]
    fn discovery_finds_each_layout_once() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        make_game_dir(&home, ".minecraft", "saves");
        make_game_dir(
            &home,
            ".var/app/com.mojang.Minecraft/data/minecraft",
            "resourcepacks",
        );
        for name in ["alpha", "beta"] {
            let instance = home.join(".local/share/multimc/instances").join(name);
            std::fs::create_dir_all(&instance).unwrap();
            std::fs::create_dir_all(instance.join(".minecraft/versions")).unwrap();
        }

        let found = discover_game_dirs(&home);
        assert_eq!(found.len(), 4);
        assert_eq!(found[0].source, DiscoverySource::Std);
        assert_eq!(found[1].source, DiscoverySource::Flatpak);
        assert_eq!(found[2].source, DiscoverySource::MultiMc);
        assert_eq!(found[3].source, DiscoverySource::MultiMc);
    }

    #[test]
    fn merge_keeps_one_entry_per_path() {
        let root = tempfile::tempdir().unwrap();
        let path = make_game_dir(root.path(), "game", "versions");
        let canonical = std::fs::canonicalize(&path).unwrap();
        let discovered = vec![GameDir {
            path: canonical.clone(),
            source: DiscoverySource::Std,
        }];
        let stored = vec![
            GameDir {
                path: path.clone(),
                source: DiscoverySource::Manual,
            },
            GameDir {
                path: canonical.clone(),
                source: DiscoverySource::Manual,
            },
        ];

        let merged = merge_game_dirs(discovered, &stored);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, DiscoverySource::Std);
    }

    #[test]
    fn entry_reports_presence() {
        let root = tempfile::tempdir().unwrap();
        let path = make_game_dir(root.path(), "game", "saves");
        let dir = validate_game_dir(&path).unwrap();
        assert!(entry_for(&dir).present);

        std::fs::remove_dir_all(&path).unwrap();
        assert!(!entry_for(&dir).present);
    }
}
