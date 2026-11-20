use mine_utils_core::pack::{PackIssue, PackIssueCode};
use mine_utils_core::world::{WorldIssue, WorldIssueCode};
use mine_utils_core::{parse_install_kind, SearchKind, SearchSort};

#[test]
fn issue_codes_keep_the_existing_json_bytes() {
    let pack_codes = [
        PackIssueCode::PackNonUtf8Name,
        PackIssueCode::PackUnknownKind,
        PackIssueCode::PackBrokenArchive,
        PackIssueCode::PackMissingMeta,
        PackIssueCode::PackSymlink,
        PackIssueCode::PackDuplicate,
        PackIssueCode::PackNameCollision,
        PackIssueCode::PackNotFound,
        PackIssueCode::PackOutsideRoot,
        PackIssueCode::PackTrash,
    ];
    assert_eq!(
        serde_json::to_vec(&pack_codes).unwrap(),
        br#"["pack_non_utf8_name","pack_unknown_kind","pack_broken_archive","pack_missing_meta","pack_symlink","pack_duplicate","pack_name_collision","pack_not_found","pack_outside_root","pack_trash"]"#
    );
    assert_eq!(
        serde_json::to_vec(&[WorldIssueCode::LevelDat, WorldIssueCode::StaleStack]).unwrap(),
        br#"["level_dat","stale_stack"]"#
    );

    let pack_issue = br#"{"code":"pack_symlink","message":"linked.zip is a symlink","symlink_path":"linked.zip"}"#;
    let pack: PackIssue = serde_json::from_slice(pack_issue).unwrap();
    assert_eq!(serde_json::to_vec(&pack).unwrap(), pack_issue);

    let world_issue = br#"{"code":"level_dat","message":"could not read level.dat"}"#;
    let world: WorldIssue = serde_json::from_slice(world_issue).unwrap();
    assert_eq!(serde_json::to_vec(&world).unwrap(), world_issue);
}

#[test]
fn modrinth_kind_and_sort_keep_the_existing_json_bytes() {
    let kinds = [SearchKind::ResourcePack, SearchKind::Datapack];
    let sorts = [
        SearchSort::Relevance,
        SearchSort::Downloads,
        SearchSort::Follows,
        SearchSort::Newest,
        SearchSort::Updated,
    ];
    assert_eq!(
        serde_json::to_vec(&kinds).unwrap(),
        br#"["resourcepack","datapack"]"#
    );
    assert_eq!(
        serde_json::to_vec(&sorts).unwrap(),
        br#"["relevance","downloads","follows","newest","updated"]"#
    );
    assert_eq!(parse_install_kind("resourcepack").unwrap(), kinds[0]);
    assert_eq!(parse_install_kind("datapack").unwrap(), kinds[1]);
    assert_eq!(
        parse_install_kind("resource_pack").unwrap_err().to_string(),
        "Modrinth response could not be read: unknown project type \"resource_pack\"; want resourcepack or datapack"
    );
}
