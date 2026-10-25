use std::path::Path;

use mine_utils_core::level_dat;

const REAL: &str = "/tmp/opencode/pr-04-owner/fixtures/level-real.dat";

#[test]
fn real_level_dat_round_trip_preserves_unknown_tags() {
    let bytes = std::fs::read(Path::new(REAL))
        .expect("real level.dat copy must exist at /tmp/opencode/pr-04-owner/fixtures/level-real.dat");
    let view = level_dat::parse(&bytes).expect("real level.dat must parse");
    assert_eq!(view.version_name, "1.21.10");
    assert_eq!(
        view.enabled,
        vec![
            "vanilla",
            "file/armored elytra v1.0.4 (MC 1.17.x).zip",
            "file/double shulker shells v1.3.1 (MC 1.17.x).zip",
            "file/dragon drops v1.3.1 (MC 1.17.x).zip",
            "file/fast leaf decay v2.0.4 (MC 1.17.x).zip",
            "file/graves v2.8.5 (MC 1.17.x).zip",
            "fabric",
            "fabric-convention-tags-v2",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
    assert_eq!(
        view.disabled,
        vec![
            "minecart_improvements",
            "redstone_experiments",
            "trade_rebalance",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );

    let toggled = level_dat::set_enabled(&bytes, "graves v2.8.5 (MC 1.17.x).zip", false)
        .expect("toggle off must succeed");
    let after = level_dat::parse(&toggled).expect("toggled bytes must parse");
    assert!(!after.enabled.iter().any(|e| e.contains("graves")));
    assert!(after
        .disabled
        .contains(&"file/graves v2.8.5 (MC 1.17.x).zip".to_string()));

    level_dat::assert_outside_datapacks_unchanged(&bytes, &toggled)
        .expect("every tag outside Data.DataPacks must be unchanged");
}
