use std::path::{Path, PathBuf};
use std::time::Instant;

use mine_utils_core::{
    backup, discover_game_dirs, entry_for, home_dir, import_pack, resource_stack, scan_packs,
    AppPaths, GameDirEntry, HashCache, IconCache, ImportChoice, ImportOutcome, PackListing,
    ResourceStack, StackPaths,
};

fn main() {
    match run() {
        Ok(json) => println!("{json}"),
        Err(err) => {
            eprintln!("mine-utils-probe: {err}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<String, String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("list-game-dirs") => {
            let home = home_dir().map_err(|err| err.to_string())?;
            let dirs = discover_game_dirs(&home);
            let entries: Vec<GameDirEntry> = dirs.iter().map(entry_for).collect();
            serde_json::to_string_pretty(&entries).map_err(|err| err.to_string())
        }
        Some("scan-packs") => {
            let game_dir = game_dir(args.next())?;
            let icons = icon_cache()?;
            let listing = scan_packs(&game_dir.join("resourcepacks"), &icons)
                .map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&listing).map_err(|err| err.to_string())
        }
        Some("import-pack") => {
            let source = args
                .next()
                .ok_or_else(|| format!("import-pack needs a source path\n{USAGE}"))?;
            let game_dir = game_dir(args.next())?;
            let choice = match args.next().as_deref() {
                None | Some("default") => ImportChoice::Default,
                Some("replace") => ImportChoice::Replace,
                Some("skip") => ImportChoice::Skip,
                Some(other) => return Err(format!("unknown import choice {other}\n{USAGE}")),
            };
            let icons = icon_cache()?;
            let outcome = import_pack(
                Path::new(&source),
                &game_dir.join("resourcepacks"),
                &icons,
                choice,
                &HashCache::new(),
            )
            .map_err(|err| err.to_string())?;
            match &outcome {
                ImportOutcome::Imported { pack } => {
                    let listing = PackListing {
                        packs: vec![(**pack).clone()],
                        count: 1,
                        bytes_read: pack.bytes_read,
                        elapsed_ms: 0.0,
                    };
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "imported",
                        "listing": listing,
                    }))
                    .map_err(|err| err.to_string())
                }
                other => serde_json::to_string_pretty(other).map_err(|err| err.to_string()),
            }
        }
        Some("toggle-resource") => {
            let filename = args
                .next()
                .ok_or_else(|| format!("toggle-resource needs a filename\n{USAGE}"))?;
            let enabled = match args.next().as_deref() {
                Some("enable") | Some("on") => true,
                Some("disable") | Some("off") => false,
                other => {
                    return Err(format!(
                        "toggle-resource needs enable or disable, got {other:?}\n{USAGE}"
                    ))
                }
            };
            let paths = stack_paths(game_dir(args.next())?)?;
            let started = Instant::now();
            let stack = resource_stack::set_enabled(&paths, &filename, enabled)
                .map_err(|err| err.to_string())?;
            timed_stack(started.elapsed(), stack)
        }
        Some("reorder-resource") => {
            let order_raw = args
                .next()
                .ok_or_else(|| format!("reorder-resource needs the full order as JSON\n{USAGE}"))?;
            let order: Vec<String> = serde_json::from_str(&order_raw)
                .map_err(|err| format!("order is not JSON: {err}"))?;
            let paths = stack_paths(game_dir(args.next())?)?;
            let expected = resource_stack::load(&paths)
                .map_err(|err| err.to_string())?
                .snapshot();
            let started = Instant::now();
            let stack = resource_stack::reorder_stack(&paths, &order, &expected)
                .map_err(|err| err.to_string())?;
            timed_stack(started.elapsed(), stack)
        }
        Some("list-backups") => {
            let paths = stack_paths(game_dir(args.next())?)?;
            let backups = backup::list(&paths.backups_root).map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({ "backups": backups }))
                .map_err(|err| err.to_string())
        }
        Some("restore-backup") => {
            let name = args
                .next()
                .ok_or_else(|| format!("restore-backup needs a backup file name\n{USAGE}"))?;
            let paths = stack_paths(game_dir(args.next())?)?;
            let started = Instant::now();
            let stack =
                resource_stack::restore_backup(&paths, &name).map_err(|err| err.to_string())?;
            timed_stack(started.elapsed(), stack)
        }
        Some(other) => Err(format!("unknown subcommand {other}\n{USAGE}")),
        None => Err(USAGE.to_string()),
    }
}

fn timed_stack(elapsed: std::time::Duration, stack: ResourceStack) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
        "stack": stack,
    }))
    .map_err(|err| err.to_string())
}

/// The same write context the Tauri commands build: the game dir, the backups
/// folder, and the backup policy from the app config.
fn stack_paths(game_dir: PathBuf) -> Result<StackPaths, String> {
    let paths = AppPaths::resolve().map_err(|err| err.to_string())?;
    let config = mine_utils_core::load_config(&paths.config_file()).unwrap_or_default();
    Ok(StackPaths::for_game_dir(
        &game_dir,
        &paths.data_dir,
        config.backups,
    ))
}

fn game_dir(argument: Option<String>) -> Result<PathBuf, String> {
    let path = argument
        .or_else(|| std::env::var("MINE_UTILS_GAME_DIR").ok())
        .map(PathBuf::from)
        .or_else(|| home_dir().ok().map(|home| home.join(".minecraft")))
        .ok_or_else(|| "cannot resolve the home directory".to_string())?;
    if !path.is_dir() {
        return Err(format!("game dir {} does not exist", path.display()));
    }
    Ok(path)
}

fn icon_cache() -> Result<IconCache, String> {
    let paths = AppPaths::resolve().map_err(|err| err.to_string())?;
    Ok(IconCache::new(&paths.cache_dir))
}

const USAGE: &str = "usage: mine-utils-probe <subcommand>\n\
  list-game-dirs\n\
  scan-packs [game-dir]\n\
  import-pack <source> [game-dir] [default|replace|skip]\n\
  toggle-resource <filename> <enable|disable> [game-dir]\n\
  reorder-resource <order-json> [game-dir]\n\
  list-backups [game-dir]\n\
  restore-backup <backup-file-name> [game-dir]\n\
game-dir defaults to $MINE_UTILS_GAME_DIR or ~/.minecraft";
