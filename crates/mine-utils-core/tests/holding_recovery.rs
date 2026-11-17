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
