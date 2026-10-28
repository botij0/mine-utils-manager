use std::path::{Path, PathBuf};
use std::time::Instant;

use mine_utils_core::{
    backup, data_stack, discover_game_dirs, entry_for, finish_modrinth_install, home_dir,
    import_pack, level_backup, prepare_modrinth_install, primary_file, resource_stack, scan_packs,
    world, AppPaths, DataPaths, GameDirEntry, HashCache, IconCache, ImportChoice, ImportOutcome,
    ModrinthClient, PackListing, ReqwestTransport, ResourceStack, SearchKind, SearchQuery,
    SearchSort, StackPaths, WorldPaths, ZipRoot,
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
        Some("scan-worlds") => {
            let game_dir = game_dir(args.next())?;
            let icons = icon_cache()?;
            let started = Instant::now();
            let worlds = world::scan_worlds(&game_dir, &icons);
            serde_json::to_string_pretty(&serde_json::json!({
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
                "count": worlds.len(),
                "worlds": worlds,
            }))
            .map_err(|err| err.to_string())
        }
        Some("toggle-datapack") => {
            let world_name = args
                .next()
                .ok_or_else(|| format!("toggle-datapack needs a world\n{USAGE}"))?;
            let filename = args
                .next()
                .ok_or_else(|| format!("toggle-datapack needs a filename\n{USAGE}"))?;
            let enabled = match args.next().as_deref() {
                Some("enable") | Some("on") => true,
                Some("disable") | Some("off") => false,
                other => {
                    return Err(format!(
                        "toggle-datapack needs enable or disable, got {other:?}\n{USAGE}"
                    ))
                }
            };
            let paths = data_paths(game_dir(args.next())?, &world_name)?;
            let started = Instant::now();
            let stack = data_stack::set_enabled(&paths, &filename, enabled)
                .map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
                "stack": stack,
            }))
            .map_err(|err| err.to_string())
        }
        Some("list-level-backups") => {
            let rest: Vec<String> = args.collect();
            let (world_name, game) = match rest.as_slice() {
                [world] => (world.clone(), None),
                [world, game] => (world.clone(), Some(game.clone())),
                _ => (String::new(), None),
            };
            if world_name.is_empty() {
                return Err(format!("list-level-backups needs a world\n{USAGE}"));
            }
            let paths = data_paths(game_dir(game)?, &world_name)?;
            let backups = level_backup::list(&paths.backups_root, &paths.world_name())
                .map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({ "backups": backups }))
                .map_err(|err| err.to_string())
        }
        Some("restore-level-backup") => {
            let world_name = args
                .next()
                .ok_or_else(|| format!("restore-level-backup needs a world\n{USAGE}"))?;
            let name = args
                .next()
                .ok_or_else(|| format!("restore-level-backup needs a backup file name\n{USAGE}"))?;
            let paths = data_paths(game_dir(args.next())?, &world_name)?;
            let started = Instant::now();
            let stack = data_stack::restore_backup(&paths, &name).map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
                "world_format": world_format_name(&paths),
                "stack": stack,
            }))
            .map_err(|err| err.to_string())
        }
        Some("search-modrinth") => {
            let query = args
                .next()
                .ok_or_else(|| format!("search-modrinth needs a query\n{USAGE}"))?;
            let kind = match args.next().as_deref() {
                None | Some("resourcepack") => SearchKind::ResourcePack,
                Some("datapack") => SearchKind::Datapack,
                Some(other) => return Err(format!("unknown kind {other}\n{USAGE}")),
            };
            let game_version = args.next();
            let client =
                ModrinthClient::new(ReqwestTransport::new().map_err(|err| err.to_string())?);
            let started = Instant::now();
            let hits = client
                .search(&SearchQuery {
                    text: query,
                    kind,
                    game_version,
                    sort: SearchSort::Relevance,
                    limit: 20,
                    offset: 0,
                })
                .map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
                "count": hits.len(),
                "hits": hits,
            }))
            .map_err(|err| err.to_string())
        }
        Some("install-modrinth") => {
            let project = args
                .next()
                .ok_or_else(|| format!("install-modrinth needs a project id\n{USAGE}"))?;
            let version = args
                .next()
                .ok_or_else(|| format!("install-modrinth needs a version id\n{USAGE}"))?;
            let kind = match args.next().as_deref() {
                Some("resourcepack") => (SearchKind::ResourcePack, ZipRoot::Assets),
                Some("datapack") => (SearchKind::Datapack, ZipRoot::Data),
                _ => {
                    return Err(format!(
                        "install-modrinth needs resourcepack|datapack\n{USAGE}"
                    ))
                }
            };
            let world_arg = args.next();
            let (world_name, game_dir) = match kind.0 {
                SearchKind::ResourcePack => {
                    // No world for resource packs: the next positional is the
                    // game dir, and it is required — never ~/.minecraft.
                    (None, require_game_dir(world_arg)?)
                }
                SearchKind::Datapack => {
                    let name = world_arg
                        .ok_or_else(|| format!("datapack install needs a world\n{USAGE}"))?;
                    let game = require_game_dir(args.next())?;
                    // Resolve the world eagerly so a typo fails before any
                    // download starts.
                    WorldPaths::for_world(&game, &name).map_err(|err| err.to_string())?;
                    (Some(name), game)
                }
            };
            let paths = AppPaths::resolve().map_err(|err| err.to_string())?;
            let client =
                ModrinthClient::new(ReqwestTransport::new().map_err(|err| err.to_string())?);
            let started = Instant::now();
            // The same two-phase routine the Tauri command runs: metadata and
            // download first, then commit/import/enable/provenance.
            let prepared = prepare_modrinth_install(
                &client,
                &game_dir,
                world_name.as_deref(),
                &project,
                &version,
                // The filename comes from the version metadata, not the CLI:
                // resolve it the same way the app does.
                &version_filename(&client, &project, &version)?,
                match kind.0 {
                    SearchKind::ResourcePack => "resourcepack",
                    SearchKind::Datapack => "datapack",
                },
            )
            .map_err(|err| err.to_string())?;
            let config = mine_utils_core::load_config(&paths.config_file()).unwrap_or_default();
            let stack_paths =
                StackPaths::for_game_dir(&game_dir, &paths.data_dir, config.backups.clone());
            let data_paths = match &world_name {
                Some(name) => {
                    let resolved =
                        WorldPaths::for_world(&game_dir, name).map_err(|err| err.to_string())?;
                    Some(DataPaths::for_world(
                        &game_dir,
                        &resolved.world_dir,
                        &paths.data_dir,
                        config.backups,
                    ))
                }
                None => None,
            };
            let icons = icon_cache()?;
            let outcome = finish_modrinth_install(
                prepared,
                &stack_paths,
                data_paths.as_ref(),
                &paths.data_dir,
                &icons,
                &HashCache::new(),
            )
            .map_err(|err| err.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
                "outcome": outcome_summary(&outcome),
            }))
            .map_err(|err| err.to_string())
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

/// `install-modrinth` never falls through to `~/.minecraft`: the game dir
/// must come from the positional argument or `MINE_UTILS_GAME_DIR`.
fn require_game_dir(argument: Option<String>) -> Result<PathBuf, String> {
    let path = argument
        .or_else(|| std::env::var("MINE_UTILS_GAME_DIR").ok())
        .map(PathBuf::from)
        .ok_or_else(|| format!("install-modrinth needs a game dir argument\n{USAGE}"))?;
    if !path.is_dir() {
        return Err(format!("game dir {} does not exist", path.display()));
    }
    Ok(path)
}

/// The filename the app would install: the version's primary file, else its
/// first file. Mirrors the Discover picker's selection.
fn version_filename(
    client: &ModrinthClient<ReqwestTransport>,
    project: &str,
    version: &str,
) -> Result<String, String> {
    let versions = client.versions(project).map_err(|err| err.to_string())?;
    let found = versions
        .iter()
        .find(|v| v.id == version)
        .ok_or_else(|| format!("version {version} not in project {project}"))?;
    primary_file(found)
        .or_else(|| found.files.first())
        .map(|file| file.filename.clone())
        .ok_or_else(|| format!("version {version} of {project} has no downloadable file"))
}

/// A short human summary of the shared install outcome for probe output.
fn outcome_summary(outcome: &mine_utils_core::CoreInstallOutcome) -> serde_json::Value {
    use mine_utils_core::CoreInstallOutcome as O;
    match outcome {
        O::ResourcePack {
            outcome,
            provenance_warning,
        } => serde_json::json!({
            "kind": "resourcepack",
            "outcome": outcome,
            "provenance_warning": provenance_warning,
        }),
        O::Datapack {
            outcome,
            provenance_warning,
        } => serde_json::json!({
            "kind": "datapack",
            "outcome": outcome,
            "provenance_warning": provenance_warning,
        }),
    }
}

fn icon_cache() -> Result<IconCache, String> {
    let paths = AppPaths::resolve().map_err(|err| err.to_string())?;
    Ok(IconCache::new(&paths.cache_dir))
}

/// The write context one world datapack command needs: the game dir, the
/// world dir, the backups folder, and the backup policy from the app config.
fn data_paths(game_dir: PathBuf, world_name: &str) -> Result<DataPaths, String> {
    if !game_dir.is_dir() {
        return Err(format!("game dir {} does not exist", game_dir.display()));
    }
    let resolved = WorldPaths::for_world(&game_dir, world_name).map_err(|err| err.to_string())?;
    let paths = AppPaths::resolve().map_err(|err| err.to_string())?;
    let config = mine_utils_core::load_config(&paths.config_file()).unwrap_or_default();
    Ok(DataPaths::for_world(
        &game_dir,
        &resolved.world_dir,
        &paths.data_dir,
        config.backups,
    ))
}

fn world_format_name(paths: &DataPaths) -> Option<String> {
    let bytes = std::fs::read(&paths.level_dat_path).ok()?;
    let view = mine_utils_core::level_dat::parse(&bytes).ok()?;
    mine_utils_core::data_format_for_version(&view.version_name).map(|format| format.label())
}

const USAGE: &str = "usage: mine-utils-probe <subcommand>\n\
  list-game-dirs\n\
  scan-packs [game-dir]\n\
  import-pack <source> [game-dir] [default|replace|skip]\n\
  toggle-resource <filename> <enable|disable> [game-dir]\n\
  reorder-resource <order-json> [game-dir]\n\
  list-backups [game-dir]\n\
  restore-backup <backup-file-name> [game-dir]\n\
  scan-worlds [game-dir]\n\
  toggle-datapack <world> <filename> <enable|disable> [game-dir]\n\
  list-level-backups <world> [game-dir]\n\
  restore-level-backup <world> <backup-file-name> [game-dir]\n\
  search-modrinth <query> [resourcepack|datapack] [game-version]\n\
  install-modrinth <project> <version> <resourcepack|datapack> [world] <game-dir>\n\
game-dir defaults to $MINE_UTILS_GAME_DIR or ~/.minecraft
install-modrinth needs an explicit game dir (positional or $MINE_UTILS_GAME_DIR)";
