# Mine Utils

Mine Utils is a desktop app for Minecraft resource packs, worlds, and shared
content. It runs as a Tauri 2 app with a React frontend and a Rust core crate.
PR-01 ships the app shell, game dir discovery, and settings. Pack and world
operations land in later PRs.

## Requirements

- Node 22 and pnpm 10
- Rust stable
- On Linux, the Tauri system dependencies. See the `tauri` job in
  `.github/workflows/ci.yml` for the package list.

## Develop

Run the app.

```sh
pnpm install
env -u APPIMAGE pnpm tauri dev
```

`env -u APPIMAGE` matters on this setup. The T3 AppImage exports `APPIMAGE`, and
rustup then mistakes it for a proxy name.

Run the browser shell without Tauri. It renders the same UI against three mock
game dirs in `src/lib/ipc.mock.ts`.

```sh
pnpm dev
```

## Test

```sh
pnpm lint
pnpm typecheck
pnpm test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mine-utils-core
```

The probe binary prints the discovered game dirs as JSON. It calls the same
core functions as the app.

```sh
cargo run -p mine-utils-core --bin mine-utils-probe list-game-dirs
```

## Layout

- `crates/mine-utils-core` owns `GameDir`, `DiscoverySource`, `Config`, and
  `AppError`, plus discovery, config persistence, and the probe binary.
- `src-tauri` owns the Tauri shell and the IPC commands.
- `src/lib/types.ts` mirrors the core shapes one to one.
- `src/lib/ipc.ts` picks the Tauri invoke transport or the localStorage mock.

## Config

The app stores `config.json` at `~/.config/mine-utils`, with data in
`~/.local/share/mine-utils` and cache in `~/.cache/mine-utils`. The config file
carries `schema`, `game_dirs`, `active_game_dir`, `last_world`, `theme`, and
`backups`. Writes go through a temp file, fsync, and rename.

Game dir discovery scans `~/.minecraft`, the Flatpak data dir, and Prism
Launcher and MultiMC instance dirs. A dir counts when it holds `versions`,
`saves`, or `resourcepacks`.

## Fixtures

Test fixtures are synthetic. Read `tests/fixtures/README.md` for the policy.
Never commit real worlds or packs.
