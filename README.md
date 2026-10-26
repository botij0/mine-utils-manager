# Mine Utils

Mine Utils is a desktop app for Minecraft resource packs, worlds, and shared
content. It runs as a Tauri 2 app with a React frontend and a Rust core crate.
It lists and imports resource packs, toggles them through `options.txt`,
manages world datapacks through `level.dat`, and discovers new packs on
Modrinth.

## Install

Download the release for your system from the GitHub releases page. Each
release ships an AppImage, an RPM, and a `SHA256SUMS.txt`. Check the download
before running it:

```sh
sha256sum -c SHA256SUMS.txt
```

Run the AppImage directly (stable release name, version from the tag):

```sh
chmod +x mine-utils_*_amd64.AppImage
./mine-utils_*_amd64.AppImage
```

Or install the RPM on Fedora (and other rpm systems):

```sh
sudo dnf install ./mine-utils-*-1.x86_64.rpm
```

There is no auto-update and the artifacts are not signed: install a new
release to upgrade, and verify the checksum above.

## Requirements

- Node 22 and pnpm 10
- Rust stable
- On Linux, the Tauri system dependencies. See the `tauri` job in
  `.github/workflows/ci.yml` for the package list. For the full release
  artifacts (AppImage + RPM, `.github/workflows/release.yml`) you also need
  `rpm`/`rpmbuild` plus the webkit/gtk dev packages in `release.yml`.

## Develop

Run the app.

```sh
pnpm install
env -u APPIMAGE pnpm tauri dev
```

`env -u APPIMAGE` matters on this setup. The T3 AppImage exports `APPIMAGE`, and
rustup then mistakes it for a proxy name.

Run the browser shell without Tauri. It renders the same UI against three mock
game dirs in `src/lib/ipc.mock.ts`. The Discover mock simulates Modrinth
locally and never reaches the network.

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

Modrinth search and install go through the same core client. `install-modrinth`
needs an explicit game dir (positional or `$MINE_UTILS_GAME_DIR`); it never
falls through to `~/.minecraft`:

```sh
cargo run -p mine-utils-core --bin mine-utils-probe search-modrinth fresh resourcepack
cargo run -p mine-utils-core --bin mine-utils-probe install-modrinth <project> <version> resourcepack /tmp/demo-game
```

## Build a release

Tagged `v*` pushes build the AppImage and the RPM on Ubuntu 24.04 (see
`.github/workflows/release.yml`) and attach them plus `SHA256SUMS.txt` to the
GitHub release. To build the same artifacts locally:

```sh
env -u APPIMAGE pnpm tauri build
```

The bundles land under `target/release/bundle/` (the workspace target dir,
not `src-tauri/target/`). The RPM needs `webkit2gtk4.1` and `gtk3` at install
time; the AppImage bundles its own WebKit, which is why it is large
(~112 MB, ~320 MB extracted). No signing, no auto-update.

## Layout

- `crates/mine-utils-core` owns `GameDir`, `DiscoverySource`, `Config`, and
  `AppError`, plus discovery, config persistence, pack/world scanning, the
  Modrinth client, verified downloads, provenance, and the probe binary.
- `src-tauri` owns the Tauri shell and the IPC commands.
- `src/lib/types.ts` mirrors the core shapes one to one.
- `src/lib/ipc.ts` picks the Tauri invoke transport or the localStorage mock.

## Data locations

- Config: `~/.config/mine-utils/config.json` (`schema`, `game_dirs`,
  `active_game_dir`, `last_world`, `theme`, `backups`). Writes go through a
  temp file, fsync, and rename.
- Data: `~/.local/share/mine-utils` (backups and `provenance.json`, which maps
  an installed file's SHA-256 back to its Modrinth project and version).
- Cache: `~/.cache/mine-utils` (pack icons, keyed by SHA-256).

Game dir discovery scans `~/.minecraft`, the Flatpak data dir, and Prism
Launcher and MultiMC instance dirs. A dir counts when it holds `versions`,
`saves`, or `resourcepacks`.

## Safety rules

- Writes are blocked while Minecraft runs (session lock or process scan), and
  every write keeps a backup under the data dir.
- Modrinth search fires only from the Discover tab; the local tabs never
  touch the network.
- Downloads come only from `https://cdn.modrinth.com` on the default port.
  Redirects elsewhere are refused, files are capped at 500 MB, the SHA-512
  from the version metadata must match, and the zip must contain `assets/`
  (resource packs) or `data/` (datapacks). A mismatch deletes the temp file
  and leaves nothing behind. The verified file is committed under the API
  filename without replacing an existing file, then routed through the same
  import/enable path as a manual install; a running game refuses before any
  download starts, except a locked datapack world, which still copies the new
  datapack and leaves it disabled (`Locked`) until the lock releases. A fresh
  game dir with no `options.txt` is refused before any download: launch
  Minecraft once so it writes the file, then install.
- A hash mismatch, a non-zip, or a wrong archive root is never retried.
  A 429 or 5xx is retried up to three times, honoring `Retry-After`.

## Known limits

- Pack icons in the local grid use the `asset://` protocol (see issue #8);
  Modrinth result icons are plain remote HTTPS and are unaffected.
- A locked world copies a new datapack but leaves it disabled until the lock
  releases.
- The installed `.desktop` entry lists `Categories=Utility;Game;` for the RPM
  (via `src-tauri/linux/mine-utils.desktop.hbs`). The AppImage entry remains
  `Utility;` because the AppImage config has no template; that category is the
  operator's call at packaging time.
- No auto-update: watch the releases page for new versions.
