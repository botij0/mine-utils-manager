# Development guide

Use this guide to build, test, and release Mine Utils. For the module map and design decisions, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Requirements

- Node.js 22
- pnpm 10. The repository pins `pnpm@10.30.0` in `package.json`.
- Rust stable with `rustfmt` and `clippy`
- Linux system dependencies for Tauri 2

On Ubuntu 24.04, install the development dependencies used by CI:

```sh
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev \
  libssl-dev \
  patchelf
```

Install `rpm` as well when building release bundles:

```sh
sudo apt-get install -y rpm
```

For other distributions, install the equivalent WebKitGTK 4.1, GTK, app indicator, SVG, X11 automation, SSL, and patchelf development packages.

## Install and run

Install JavaScript dependencies from the lockfile:

```sh
pnpm install --frozen-lockfile
```

Run the full desktop app:

```sh
env -u APPIMAGE pnpm tauri dev
```

`env -u APPIMAGE` avoids a rustup proxy-name problem when the terminal inherits `APPIMAGE` from another app, such as an AppImage-hosted development environment. It is safe to use in a normal terminal too.

Tauri starts Vite at `http://localhost:5173`, then opens the native window. Changes to the frontend reload through Vite. Rust changes require the normal Tauri rebuild path.

## Browser-only UI demo

Run Vite without Tauri:

```sh
pnpm dev
```

Open `http://localhost:5173`. The same React UI uses `mockClient` from `src/lib/ipc.mock.ts` because no Tauri runtime is present.

The demo:

- Provides three mock game folders and synthetic packs, worlds, stacks, and backups.
- Persists mock configuration in localStorage.
- Simulates Modrinth locally without network requests.
- Never reads or writes real Minecraft files.

The mock does not enforce every backend behavior. In particular, it ignores the stale-stack snapshot on resource reorders. Use the real Tauri backend and isolated test game folders to verify filesystem safety.

## Everyday checks

Run the frontend checks:

```sh
pnpm lint
pnpm typecheck
pnpm test
```

Run the Rust and desktop checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Build the frontend or the desktop executable without bundles:

```sh
pnpm build
pnpm tauri build --no-bundle
```

The CI workflow runs frontend, core Rust, and Tauri jobs on Ubuntu 24.04. Its Tauri job also builds the desktop app without bundles.

## Generate IPC wire types

Rust owns the serialized DTOs and `ErrorCode` list. After changing a wire type
or error code, regenerate the committed TypeScript file:

```sh
pnpm wire:generate
pnpm typecheck
```

Check freshness without rewriting the file:

```sh
pnpm wire:check
env -u APPIMAGE cargo run --locked -p mine-utils-core --features wire-types --bin generate-wire-types -- --check
```

CI runs clippy with `--features wire-types` and the Cargo freshness check. It
fails if `src/lib/generated/wire.ts` differs from Rust. Generation uses optional `ts-rs` derives in the core crate and needs no
Tauri runtime or GTK libraries. JSON integer fields remain TypeScript numbers.

`src/lib/types.ts` re-exports the generated types and the derived client types
in `clientTypes.ts`. The client converts `PackIcon.path` to `PackIcon.url`.
Command names and arguments in `ipc.ts` remain handwritten.

Run the destructive acceptance probes on a clean working tree:

```sh
pnpm wire:test
```

The probes temporarily rename `game_running`, `no_active_game_dir`,
`download_offline`, `stale_stack`, and `world_mismatch`, regenerate, and require
`pnpm typecheck` to reject their unchanged consumers. `download_offline` tests a
comparison against a code the mock never constructs. The probes also corrupt
the generated file and require the freshness check to fail. A `finally` block restores both files.

## Core probe CLI

`mine-utils-probe` exposes selected core operations as JSON. It uses the same discovery, pack, world, and Modrinth code as the desktop app.

List automatically discovered game folders:

```sh
cargo run -p mine-utils-core --bin mine-utils-probe list-game-dirs
```

Search Modrinth:

```sh
cargo run -p mine-utils-core --bin mine-utils-probe \
  search-modrinth fresh resourcepack
```

Install into an explicitly chosen test game folder:

```sh
cargo run -p mine-utils-core --bin mine-utils-probe \
  install-modrinth <project> <version> resourcepack /tmp/demo-game
```

A datapack install also needs a world:

```sh
cargo run -p mine-utils-core --bin mine-utils-probe \
  install-modrinth <project> <version> datapack <world> /tmp/demo-game
```

The install command requires an explicit game directory or `$MINE_UTILS_GAME_DIR`. It never falls back to `~/.minecraft`.

> [!WARNING]
> Probe commands that import, toggle, reorder, restore, or install can modify files. Use disposable game folders and isolated XDG directories when testing. Do not point experimental commands at a real world.

## Testing against real files

Prefer temporary game directories with the minimum Minecraft structure needed by the operation under test. Keep real config, data, and cache locations out of test runs by overriding the relevant XDG environment variables.

Some tests are designed to opt into real-file round trips with `MINE_UTILS_REAL_OPTIONS`. CI leaves this variable unset. Do not set it to a file you cannot safely replace.

When testing Modrinth installs, verify both the happy path and refusal cases:

- Duplicate and collision outcomes
- Running-game and locked-world behavior
- Hash, archive-root, redirect, and size-limit failures
- No temporary files after a failed install
- Correct `options.txt` or `level.dat` backups after a write

## Build release artifacts

Build the configured AppImage and RPM bundles locally:

```sh
env -u APPIMAGE pnpm tauri build
```

Artifacts land in the workspace target directory:

```text
target/release/bundle/appimage/
target/release/bundle/rpm/
```

They do not land under `src-tauri/target/`.

The RPM declares `webkit2gtk4.1` and `gtk3` as runtime dependencies. The AppImage is larger because it carries more of its runtime. Neither artifact is signed, and the app has no auto-update mechanism.

## Publish a release

The release workflow in `.github/workflows/release.yml` runs on `v*` tags. It:

1. Runs frontend checks and all workspace Rust tests.
2. Builds the AppImage and RPM on Ubuntu 24.04.
3. Requires the tag version to match `src-tauri/tauri.conf.json`.
4. Copies the artifacts to stable release filenames.
5. Generates `SHA256SUMS.txt`.
6. Publishes the files to the GitHub release.

A manual `workflow_dispatch` performs a dry run. It uploads a workflow artifact but does not publish a GitHub release.

Before tagging, update the app version and make sure CI is green on the exact commit you intend to release.

## Packaging notes

- Bundle targets are configured in `src-tauri/tauri.conf.json`.
- The RPM uses `src-tauri/linux/mine-utils.desktop.hbs` and lists `Categories=Utility;Game;`.
- The AppImage desktop entry currently remains in the general `Utility` category.
- Production frontend sourcemaps are disabled in `vite.config.ts`, so production stack traces point to minified code.
- Local pack icons use the scoped `asset://` protocol. Their remaining display issue is tracked separately from Modrinth's remote HTTPS icons.

## Documentation map

- [README.md](README.md) stays focused on features and getting started.
- [GUIDE.md](GUIDE.md) documents user workflows and recovery.
- [ARCHITECTURE.md](ARCHITECTURE.md) documents runtime boundaries, data flow, and invariants.
- This file documents contributor commands and delivery.

Update the relevant document when changing a user workflow, safety rule, path, command, or release artifact.
