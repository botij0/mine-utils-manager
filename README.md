<div align="center">
  <img src="assets/icon.svg" alt="Mine Utils logo" width="112" height="112">

  <h1>Mine Utils</h1>

  <p><strong>A safer, simpler desktop manager for Minecraft resource packs and datapacks.</strong></p>
  <p>Organize local packs, control load order, and install verified content from Modrinth.</p>

  <p>
    <a href="https://github.com/botij0/mine-utils-manager/actions/workflows/ci.yml">
      <img alt="CI status" src="https://github.com/botij0/mine-utils-manager/actions/workflows/ci.yml/badge.svg">
    </a>
    <a href="https://github.com/botij0/mine-utils-manager/releases/latest">
      <img alt="Latest release" src="https://img.shields.io/github/v/release/botij0/mine-utils-manager">
    </a>
    <img alt="Linux" src="https://img.shields.io/badge/platform-Linux-1d4ed8">
    <img alt="Minecraft Java Edition" src="https://img.shields.io/badge/Minecraft-Java%20Edition-5b82e8">
  </p>
</div>

<p align="center">
  <a href="GUIDE.md">User guide</a> ·
  <a href="ARCHITECTURE.md">Architecture</a> ·
  <a href="DEVELOPMENT.md">Development</a>
</p>

## What you can do

| | Feature | What it gives you |
| --- | --- | --- |
| 📦 | **Resource pack manager** | Import zip files or folders, inspect metadata, search and filter, enable packs, and set their priority. |
| 🌍 | **World datapacks** | Manage each world separately, check compatibility, fix missing references, and control datapack order. |
| 🧭 | **Modrinth discovery** | Search resource packs and datapacks, choose a version, and install verified downloads. |
| 🗂️ | **Multiple game folders** | Use a standard, Flatpak, Prism Launcher, MultiMC, or manually added Minecraft folder. |
| 🛡️ | **Safer changes** | Pause writes while Minecraft runs, keep timestamped backups, and move deleted packs to the system trash. |
| 🎨 | **A comfortable desktop app** | Switch between System, Light, and Dark themes. Your local pages never use the network. |

## Quick start

### 1. Download and run

Download the **AppImage** and **`SHA256SUMS.txt`** from the [latest release](https://github.com/botij0/mine-utils-manager/releases/latest). Open a terminal in your download folder:

```sh
# Verify the files you downloaded
sha256sum --ignore-missing -c SHA256SUMS.txt

# Make the app executable and launch it
chmod +x mine-utils_*_amd64.AppImage
./mine-utils_*_amd64.AppImage
```

On Fedora or another RPM-based system, you can install the RPM instead:

```sh
sudo dnf install ./mine-utils-*-1.x86_64.rpm
```

Then open **Mine Utils** from your application menu.

### 2. Pick your Minecraft folder

1. Select a detected game folder, or paste its path and choose **Add**.
2. Close Minecraft before changing packs or restoring backups.
3. Open **Packs**, **Worlds**, or **Discover** and start organizing.

> [!NOTE]
> Launch Minecraft once before using a new game folder. This creates files such as `options.txt` that Mine Utils needs.

### Run from source

You need Node 22, pnpm 10, Rust stable, and the Linux dependencies listed in [DEVELOPMENT.md](DEVELOPMENT.md#requirements).

```sh
git clone https://github.com/botij0/mine-utils-manager.git
cd mine-utils-manager
pnpm install
env -u APPIMAGE pnpm tauri dev
```

For a quick UI preview without Rust or real game files, run `pnpm dev` and open `http://localhost:5173`. This browser demo uses mock data only.

## Learn more

| Document | Best for |
| --- | --- |
| [GUIDE.md](GUIDE.md) | Everyday tasks, backups, safety behavior, and troubleshooting |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How React, Tauri, Rust, Minecraft files, and Modrinth fit together |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Local setup, tests, probe commands, and release builds |

> [!IMPORTANT]
> Releases are not signed and there is no auto-update. Verify new downloads and install a newer release to upgrade.
