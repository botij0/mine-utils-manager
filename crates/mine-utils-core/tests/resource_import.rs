use std::cell::RefCell;
use std::path::{Path, PathBuf};

use mine_utils_core::{HashCache, IconCache, ImportChoice, ImportOutcome, PackError, TrashService};

#[derive(Default)]
struct Trash(RefCell<Vec<PathBuf>>);

impl TrashService for Trash {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
        self.0.borrow_mut().push(path.to_path_buf());
        std::fs::remove_dir_all(path).unwrap();
        Ok(())
    }
}

#[test]
fn replace_preserves_reference_indices_in_both_options_lists() {
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
    let before = "music:0.4\nresourcePacks:[\"vanilla\",\"file/Faithful\",\"fabric\"]\nincompatibleResourcePacks:[\"fabric\",\"file/Faithful\"]\n";
    std::fs::write(game.join("options.txt"), before).unwrap();
    let trash = Trash::default();
    let outcome = mine_utils_core::pack_scan::import_pack_with(
        &source,
        &game.join("resourcepacks"),
        &IconCache::new(&root.path().join("cache")),
        ImportChoice::Replace,
        &HashCache::new(),
        &trash,
    )
    .unwrap();
    assert!(matches!(outcome, ImportOutcome::Imported { .. }));
    assert_eq!(std::fs::read_to_string(game.join("options.txt")).unwrap(),
        "music:0.4\nresourcePacks:[\"vanilla\",\"file/Faithful-1.21\",\"fabric\"]\nincompatibleResourcePacks:[\"fabric\",\"file/Faithful-1.21\"]\n");
}
