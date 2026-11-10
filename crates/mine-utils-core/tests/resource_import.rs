use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use mine_utils_core::resource_stack::{self, WriteGuard};
use mine_utils_core::{
    AppError, GameRunning, GameRunningKind, HashCache, IconCache, ImportChoice, ImportOutcome,
    PackError, StackPaths, TrashService,
};

#[derive(Default)]
struct Trash(RefCell<Vec<PathBuf>>);

impl TrashService for Trash {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
        self.0.borrow_mut().push(path.to_path_buf());
        std::fs::remove_dir_all(path).unwrap();
        Ok(())
    }
}

const BEFORE: &str = "music:0.4\nresourcePacks:[\"vanilla\",\"file/Faithful\",\"fabric\"]\nincompatibleResourcePacks:[\"fabric\",\"file/Faithful\"]\n";
const AFTER: &str = "music:0.4\nresourcePacks:[\"vanilla\",\"file/Faithful-1.21\",\"fabric\"]\nincompatibleResourcePacks:[\"fabric\",\"file/Faithful-1.21\"]\n";

struct Fixture {
    _root: tempfile::TempDir,
    paths: StackPaths,
    source: PathBuf,
    icons: IconCache,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let old = game.join("resourcepacks/Faithful");
        let source = root.path().join("incoming/Faithful-1.21");
        for pack in [&old, &source] {
            std::fs::create_dir_all(pack.join("assets")).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"pack_format":34,"description":"Faithful"}}"#,
            )
            .unwrap();
        }
        std::fs::write(game.join("options.txt"), BEFORE).unwrap();
        let paths = StackPaths::for_game_dir(&game, &root.path().join("data"), Default::default());
        let icons = IconCache::new(&root.path().join("cache"));
        Self {
            _root: root,
            paths,
            source,
            icons,
        }
    }

    fn listing(&self) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(self.paths.game_dir.join("resourcepacks"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }
}

#[test]
fn replace_preserves_reference_indices_in_both_options_lists() {
    let fixture = Fixture::new();
    let outcome = resource_stack::import_pack_safely(
        &fixture.paths,
        &fixture.source,
        ImportChoice::Replace,
        &fixture.icons,
        &HashCache::new(),
    )
    .unwrap();
    assert!(matches!(outcome, ImportOutcome::Imported { .. }));
    let after = std::fs::read_to_string(&fixture.paths.options_path).unwrap();
    assert_eq!(after, AFTER);
    assert_eq!(fixture.listing(), ["Faithful-1.21"]);
    let backups = mine_utils_core::backup::list(&fixture.paths.backups_root).unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read_to_string(fixture.paths.backups_root.join(&backups[0].file_name)).unwrap(),
        BEFORE
    );
    println!(
        "options.txt before:\n{BEFORE}options.txt after:\n{after}folder listing: {:?}",
        fixture.listing()
    );
}

struct FailingGuard {
    calls: Cell<usize>,
    fail_at: usize,
}

impl WriteGuard for FailingGuard {
    fn ensure_writable(&self, _: &Path) -> Result<(), AppError> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == self.fail_at {
            Err(AppError::GameRunning {
                running: GameRunning {
                    kind: GameRunningKind::Process,
                    detail: "injected running game".to_string(),
                    lock_path: None,
                    pid: Some(42),
                    command: Some("java".to_string()),
                },
            })
        } else {
            Ok(())
        }
    }
}

#[test]
fn each_guard_refusal_keeps_the_original_and_never_trashes() {
    for fail_at in 1..=6 {
        let fixture = Fixture::new();
        let trash = Trash::default();
        let guard = FailingGuard {
            calls: Cell::new(0),
            fail_at,
        };
        let err = resource_stack::import_pack_safely_with(
            &fixture.paths,
            &fixture.source,
            ImportChoice::Replace,
            &fixture.icons,
            &HashCache::new(),
            &trash,
            &guard,
        )
        .unwrap_err();
        assert_eq!(err.code(), "game_running", "guard check {fail_at}");
        assert_eq!(
            std::fs::read_to_string(
                fixture
                    .paths
                    .game_dir
                    .join("resourcepacks/Faithful/pack.mcmeta")
            )
            .unwrap(),
            r#"{"pack":{"pack_format":34,"description":"Faithful"}}"#
        );
        assert_eq!(*trash.0.borrow(), Vec::<PathBuf>::new());
        if fail_at <= 5 {
            assert_eq!(
                std::fs::read_to_string(&fixture.paths.options_path).unwrap(),
                BEFORE
            );
        }
        if fail_at <= 2 {
            assert_eq!(fixture.listing(), ["Faithful"]);
        }
    }
}

#[cfg(unix)]
#[test]
fn resource_import_lock_holder() {
    use std::os::fd::AsRawFd;
    let Ok(path) = std::env::var("MINE_UTILS_IMPORT_LOCK_HOLDER") else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut lock = libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    // SAFETY: the descriptor and flock pointer remain valid throughout fcntl.
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &mut lock) },
        0
    );
    println!("IMPORT_LOCK_HELD");
    let mut release = String::new();
    std::io::stdin().read_line(&mut release).unwrap();
}

#[cfg(unix)]
#[test]
fn held_game_lock_refuses_replace_without_changing_the_folder() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let fixture = Fixture::new();
    let world = fixture.paths.game_dir.join("saves/world");
    std::fs::create_dir_all(&world).unwrap();
    let lock = world.join("session.lock");
    std::fs::write(&lock, [0]).unwrap();
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resource_import_lock_holder", "--nocapture"])
        .env("MINE_UTILS_IMPORT_LOCK_HOLDER", lock)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(holder.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "lock holder exited"
        );
        if line.contains("IMPORT_LOCK_HELD") {
            break;
        }
        line.clear();
    }
    let before = fixture.listing();
    let result = resource_stack::import_pack_safely(
        &fixture.paths,
        &fixture.source,
        ImportChoice::Replace,
        &fixture.icons,
        &HashCache::new(),
    );
    holder.kill().unwrap();
    holder.wait().unwrap();
    let err = result.unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert_eq!(before, ["Faithful"]);
    assert_eq!(fixture.listing(), ["Faithful"]);
    assert_eq!(
        std::fs::read_to_string(&fixture.paths.options_path).unwrap(),
        BEFORE
    );
    println!(
        "held lock: {}; before {:?}; after {:?}",
        err.code(),
        before,
        fixture.listing()
    );
}

#[test]
fn post_commit_guard_error_reports_the_installed_and_restored_packs() {
    let fixture = Fixture::new();
    let trash = Trash::default();
    let err = resource_stack::import_pack_safely_with(
        &fixture.paths,
        &fixture.source,
        ImportChoice::Replace,
        &fixture.icons,
        &HashCache::new(),
        &trash,
        &FailingGuard {
            calls: Cell::new(0),
            fail_at: 6,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "game_running");
    let wire = serde_json::to_value(&err).unwrap();
    assert_eq!(wire["code"], "game_running");
    assert_eq!(wire["message"], err.to_string());
    assert_eq!(wire.as_object().unwrap().len(), 2);
    assert_eq!(fixture.listing(), ["Faithful", "Faithful-1.21"]);
    assert_eq!(
        std::fs::read_to_string(&fixture.paths.options_path).unwrap(),
        AFTER
    );
    let message = err.to_string();
    assert!(
        message.contains(&format!(
            "New pack installed at {}",
            fixture
                .paths
                .game_dir
                .join("resourcepacks/Faithful-1.21")
                .display()
        )),
        "{message}"
    );
    assert!(
        message.contains(&format!(
            "Original pack retained at {}",
            fixture
                .paths
                .game_dir
                .join("resourcepacks/Faithful")
                .display()
        )),
        "{message}"
    );
    println!(
        "{message}; listing {:?}; options.txt {AFTER}",
        fixture.listing()
    );
}

#[test]
fn different_name_replace_trashes_the_original_name() {
    let fixture = Fixture::new();
    let trash = Trash::default();
    let outcome = resource_stack::import_pack_safely_with(
        &fixture.paths,
        &fixture.source,
        ImportChoice::Replace,
        &fixture.icons,
        &HashCache::new(),
        &trash,
        &FailingGuard {
            calls: Cell::new(0),
            fail_at: usize::MAX,
        },
    )
    .unwrap();
    assert!(matches!(outcome, ImportOutcome::Imported { .. }));
    assert_eq!(
        *trash.0.borrow(),
        [fixture.paths.game_dir.join("resourcepacks/Faithful")]
    );
    assert_eq!(fixture.listing(), ["Faithful-1.21"]);
}

#[test]
fn hidden_import_sources_are_refused_inside_and_outside_the_pack_folder() {
    for inside in [true, false] {
        for name in [
            ".mine-utils-replace-Faithful.1-2-3.tmp",
            ".Faithful.1-2-3.tmp",
        ] {
            let fixture = Fixture::new();
            let root = if inside {
                fixture.paths.game_dir.join("resourcepacks")
            } else {
                fixture._root.path().join("incoming")
            };
            let source = root.join(name);
            std::fs::rename(&fixture.source, &source).unwrap();
            let trash = Trash::default();
            let err = resource_stack::import_pack_safely_with(
                &fixture.paths,
                &source,
                ImportChoice::Default,
                &fixture.icons,
                &HashCache::new(),
                &trash,
                &FailingGuard {
                    calls: Cell::new(0),
                    fail_at: usize::MAX,
                },
            )
            .unwrap_err();
            assert_eq!(err.code(), "pack_broken_archive");
            assert!(
                err.to_string().contains("reserved for temporary imports"),
                "{err}"
            );
            assert!(source.join("pack.mcmeta").is_file());
            println!("refused {}: {err}", source.display());
        }
    }
}

#[test]
fn same_name_post_commit_guard_error_names_the_holding_path() {
    let mut fixture = Fixture::new();
    let source = fixture.source.with_file_name("Faithful");
    std::fs::rename(&fixture.source, &source).unwrap();
    fixture.source = source;
    let err = resource_stack::import_pack_safely_with(
        &fixture.paths,
        &fixture.source,
        ImportChoice::Replace,
        &fixture.icons,
        &HashCache::new(),
        &Trash::default(),
        &FailingGuard {
            calls: Cell::new(0),
            fail_at: 4,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "game_running");
    let holding = std::fs::read_dir(fixture.paths.game_dir.join("resourcepacks"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".mine-utils-replace-")
        })
        .unwrap();
    assert_eq!(
        std::fs::read(holding.join("pack.mcmeta")).unwrap(),
        br#"{"pack":{"pack_format":34,"description":"Faithful"}}"#
    );
    let message = err.to_string();
    assert!(
        message.contains(&format!("Original pack retained at {}", holding.display())),
        "{message}"
    );
    assert!(
        message.contains(&format!(
            "New pack installed at {}",
            fixture
                .paths
                .game_dir
                .join("resourcepacks/Faithful")
                .display()
        )),
        "{message}"
    );
}
