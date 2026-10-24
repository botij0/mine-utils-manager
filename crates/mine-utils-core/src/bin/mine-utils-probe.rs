use std::path::{Path, PathBuf};

use mine_utils_core::{
    discover_game_dirs, entry_for, home_dir, import_pack, scan_packs, AppPaths, GameDirEntry,
    IconCache, ImportChoice, ImportOutcome, PackListing,
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
        Some(other) => Err(format!("unknown subcommand {other}\n{USAGE}")),
        None => Err(USAGE.to_string()),
    }
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
game-dir defaults to $MINE_UTILS_GAME_DIR or ~/.minecraft";
