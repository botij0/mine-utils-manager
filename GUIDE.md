# Mine Utils user guide

This guide covers the Linux desktop app for Minecraft: Java Edition. If you want to run the app for the first time, start with [README.md](README.md#quick-start).

## Contents

- [Before you start](#before-you-start)
- [Choose a game folder](#choose-a-game-folder)
- [Manage resource packs](#manage-resource-packs)
- [Manage world datapacks](#manage-world-datapacks)
- [Discover and install from Modrinth](#discover-and-install-from-modrinth)
- [Settings and backups](#settings-and-backups)
- [Safety and network behavior](#safety-and-network-behavior)
- [Troubleshooting](#troubleshooting)

## Before you start

Mine Utils edits files that Minecraft already owns. It does not replace your launcher.

- Launch Minecraft once before using a new game folder.
- Create and open a world once before installing datapacks into it.
- Close Minecraft before importing, enabling, disabling, reordering, deleting, or restoring content.
- Keep Mine Utils open while you work. It rechecks the game state every five seconds and immediately before backend writes.

> [!WARNING]
> Do not edit `options.txt` or `level.dat` in another program at the same time. Reopen the relevant Mine Utils page after an external change so it can show the latest state.

### The four sections

| Section | Use it to |
| --- | --- |
| **Packs** | Manage resource packs in the active game folder |
| **Worlds** | Manage datapacks for one selected world |
| **Discover** | Search Modrinth and install new content |
| **Settings** | Choose game folders, change the theme, and restore backups |

The **Active game dir** button in the header shows which folder every section uses. Select it to jump to Settings.

## Choose a game folder

### First run

When no active folder is configured, Mine Utils shows the welcome screen.

1. Wait for the folder scan.
2. Select a detected folder to make it active.
3. If your folder is missing, paste its full path under **Add a game folder**, choose **Add**, then select it.

You can also choose **Skip for now** and configure a folder later in **Settings**.

### Supported folder layouts

Mine Utils looks in these places:

| Source | Location scanned |
| --- | --- |
| Standard launcher | `~/.minecraft` |
| Flatpak launcher | `~/.var/app/com.mojang.Minecraft/.minecraft` (the wrapper's `data/minecraft` link points to it) |
| Prism Launcher | Instances under `~/.local/share/PrismLauncher/instances` and, for the Flatpak, `~/.var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances` |
| MultiMC | Instances under `~/.local/share/multimc/instances` |

A valid game folder contains at least one of `versions`, `saves`, or `resourcepacks`. Prism Launcher instances use their inner `minecraft` folder, or `.minecraft` when only that one exists. MultiMC instances use their inner `.minecraft` folder.

### Change or remove a folder

1. Open **Settings**.
2. Under **Game dirs**, select another folder to make it active.
3. Use **Add** to register a folder that was not detected.
4. Use the remove action beside a folder to remove it from Mine Utils.

Removing a folder from this list does **not** delete the folder or any Minecraft files. Removing the active folder clears the active selection until you choose another one.

## Manage resource packs

Resource packs live in the active folder's `resourcepacks` directory. The active stack is stored in `options.txt`.

### Import a pack

1. Open **Packs**.
2. Choose **Import pack**.
3. Choose **Browse file** for a zip or **Browse folder** for an unpacked pack. You can also paste a path.
4. Choose **Import**.

You can drop a zip or pack folder onto the page instead. Import dropped items one at a time.

A folder must contain `pack.mcmeta`. Mine Utils copies folder packs into `resourcepacks`; it does not link to the original folder. After importing, select the pack and enable it if needed.

### Handle duplicates and name collisions

Mine Utils compares content hashes as well as filenames.

- **Duplicate** means the same content is already installed. Choose **Skip** to keep the installed pack, or **Replace** to replace the matching old entry.

- **Collision** means the filename exists with different content. Choose **Cancel**, or **Replace** to install the new pack before moving the old pack to the system trash.

If both a duplicate and a separate filename collision exist, Mine Utils leaves both installed packs in place. Rename the source pack before importing it.

During Replace, Mine Utils holds the old pack under a hidden sibling name until the new copy commits. A failed commit restores the old name when it is free. If another file takes that name, the error shows the holding path where you can recover the original. If trash cleanup fails after installation, the error reports the failure and any holding path that remains.

Replacing a resource pack with a differently named duplicate keeps its position in both `options.txt` pack lists.

When the replacement has a different filename, the trash entry keeps the original filename if that name is still free. For a same-name replacement, look for `.mine-utils-replace-<original-name>.<pid>-<stamp>-<counter>.tmp` in the trash. Restore that entry outside the pack folder, rename it to the original filename, then import it. A zip needs its `.zip` extension back. If an import error says the new pack is installed, check the refreshed pack list and stack before retrying. The error also names the saved original.

### Find and inspect packs

- Use the search box to match names and descriptions.
- Use the kind and format filters to narrow the grid.
- Choose **Clear filters** if no packs match.
- Select a pack card to see its description, kind, declared formats, supported game versions, and full path.

Mine Utils displays format information. It never rewrites a pack's declared format range.

### Enable or disable a pack

1. Select a pack card.
2. Use **Enabled in the active stack** in the details panel.

Only resource packs, or packs that contain both resource and data content, belong in this stack. A pack must also be readable.

### Change load priority

The **Active stack** lists enabled entries with the highest priority first.

> **Top card loads last and wins.**

Drag a pack to a new position, or use its up and down controls. Entries such as `vanilla` that Minecraft owns are locked and cannot move. Disabling a pack also removes it from the incompatible-pack list.

### Delete a pack

1. Select the pack.
2. Choose **Delete pack**.
3. Confirm with **Move to trash**.

Mine Utils removes the pack's stack references before moving it to the system trash. It does not permanently delete the pack. Resource packs whose root is a symlink are not deleted by Mine Utils; remove those links with your file manager.

A real pack folder containing a nested symlink shows the link path in its details. Remove that link in a file manager before enabling the pack. You can still disable an enabled pack or delete it. Deleting moves the whole pack folder to the trash, including the link, without touching its target.

## Manage world datapacks

Datapacks belong to one world. Mine Utils reads them from `saves/<world>/datapacks` and stores their enabled and disabled lists in that world's `level.dat`.

### Select a world

Open **Worlds** and choose a world in the left rail. The rail shows the world's name, icon, and game version when available.

Unreadable worlds stay visible. If a world is marked read-only, repair or restore its files before changing datapacks.

### Import a datapack

1. Select the target world.
2. Choose **Import zip** or **Import folder**.
3. Pick the datapack.
4. Resolve any duplicate or collision prompt with **Replace**, **Skip**, or **Cancel**.

Imported datapacks can be enabled from their row or details panel. If the selected world is locked, Mine Utils can copy the datapack but leaves it disabled. Enable it after the lock is released.

### Enable, disable, and reorder

- Use **Enable** or **Disable** on a datapack row.
- Enabled datapacks appear first. Disabled and newly discovered datapacks follow.
- Use **↑** and **↓** to change priority.
- The top enabled datapack loads last and has the highest priority.
- Entries locked by Minecraft cannot move or toggle.

Pack files copied directly into `datapacks` still appear even when `level.dat` does not mention them. Use **Enable** to add them to the stack.

### Read compatibility warnings

Mine Utils compares the world's data format with each pack's declared format range. A warning helps you spot a possible mismatch before launching the world. Select a datapack to see its format and compatibility details.

A warning is different from a broken-pack error. A broken pack cannot be inspected normally, but an enabled broken datapack can still be disabled so you can recover the world.

### Remove a missing reference

If `level.dat` names a datapack that no longer exists, the row says **File missing from datapacks**.

1. Select that row.
2. Choose **Remove reference**.

This removes the dead entry from `level.dat`. It does not try to enable a missing file.

### Delete a datapack

Select a datapack and choose **Delete**. Mine Utils disables the pack, moves it to the system trash, then removes its reference from the world. If the pack cannot be moved to the trash, it stays installed but disabled.

When the datapack root is a symlink, Mine Utils does not delete it. Remove the link from `datapacks` in a file manager; the folder it points to is not touched.

For a real datapack folder containing a nested symlink, details show the link path. Remove that link in a file manager before enabling the pack. You can still disable an enabled pack. Deleting moves the whole pack folder to the trash, including the link, without touching its target.

## Discover and install from Modrinth

The **Discover** section is the only part of the app that contacts Modrinth.

### Search

1. Choose **Resource packs** or **Datapacks**.
2. Type a search term. Search starts shortly after you stop typing.
3. Optionally choose a game version and a sort order: **Relevance**, **Downloads**, **Follows**, **Newest**, or **Updated**.
4. Select a result to open its details.

Mine Utils loads results 20 at a time. Select **Load more** below the results to fetch and append the next page. Changing the search term, project type, game version, or sort starts again from the first page. An empty search does not send a request, opening Discover alone does not prefetch content, and there is no infinite scroll or background loading.

### Choose a version

The version picker lists the available releases for the selected project.

- Mine Utils preselects a compatible version when it can determine the target game or world version.
- You can still choose another listed version.
- Datapack installs show only versions whose primary download is a `.zip` file.
- The backend checks datapack compatibility with the target world before installing.

### Install

1. Choose **Install…** in the project details.
2. Review the version and filename.
3. For a datapack, select the **Target world**.
4. Choose **Install**.

During a download, choose **Cancel** to stop it. The sheet shows **Install cancelled.** after the backend confirms cancellation and removes the temporary file. Cancellation is available until the commit phase begins. After that point, the install completes and the sheet shows its outcome.

Switching tabs keeps the install running. Return to **Discover** to see the result. Closing the app view or changing game folders requests cancellation before the commit phase. Installation preparation has a 15-minute total deadline, including a slow trickle of bytes.

A successful install copies the pack and enables it. Return to **Packs** or **Worlds** to adjust it.

Special outcomes:

- Identical content already at the target filename is reused without another download.
- A duplicate is reported without replacing the installed content.
- A filename collision is reported before downloading, and the existing file is left untouched. Remove or rename that file, then install again.
- A locked datapack world receives the file, but the datapack stays disabled until the lock is released.
- Other running-game states block the install before downloading.

## Settings and backups

### Theme

Open **Settings** and choose **System**, **Light**, or **Dark**. System follows your desktop preference and updates when that preference changes.

### What gets backed up

With the default configuration, Mine Utils creates a timestamped backup before changing:

- The active game folder's `options.txt`
- A world's `level.dat`

Backups do not contain copies of every resource pack or datapack. Deleted packs are recoverable through the system trash instead.

### Restore `options.txt`

1. Close Minecraft.
2. Open **Settings** and find **Backups**.
3. Review the date, filename, and size.
4. Choose **Restore** beside the desired backup.

Restoring replaces the current `options.txt`. Mine Utils first saves the current file as a new backup. It can also recreate a missing `options.txt` through this recovery path.

### Restore a world's `level.dat`

1. Close Minecraft.
2. In **Settings**, scroll to **World level.dat**.
3. Find the world group and choose **Restore** beside a backup.

If the backup and current world have different readable seeds, Mine Utils refuses the restore and changes nothing. A missing or corrupt current file, or an unreadable seed, still permits restore. Mine Utils backs up the current `level.dat` before a restore and can recreate a missing file. Return to **Worlds** afterward to review the restored datapack stack.

## Safety and network behavior

### While Minecraft runs

Reads remain available, but writes pause. Mine Utils detects a running game through world `session.lock` files and a process scan.

- Close Minecraft and choose **Recheck** to resume.
- A stale `session.lock` that no process holds is only a notice. It does not block writes.
- If the game-state check fails, Mine Utils treats the state as unknown and keeps writes paused.
- Automatic lock detection is supported on the targeted Unix and Linux platforms. On unsupported platforms, the app warns that writes cannot be blocked automatically.

### Network and downloads

- Local **Packs**, **Worlds**, and **Settings** pages do not use the network.
- Modrinth requests happen only from **Discover** actions.
- Downloads must come from `https://cdn.modrinth.com` on the default HTTPS port.
- Files are capped at 500 MB and checked against Modrinth's SHA-512 metadata.
- Resource pack archives must contain `assets/`; datapack archives must contain `data/`.
- Invalid files are removed instead of being left in your pack folder.
- Temporary download files are committed without replacing an existing file.

## Troubleshooting

| Problem | What to do |
| --- | --- |
| **No game folders found** | Add the full path manually. Check that it contains `versions`, `saves`, or `resourcepacks`. For Prism Launcher, use the instance's inner `minecraft` folder (`.minecraft` on older instances). For MultiMC, use the inner `.minecraft` folder. |
| **`options.txt` is missing** | Launch Minecraft once with that game folder, close it, then retry. To recover an older file, use an existing backup in Settings. |
| **Writes are paused** | Close every Minecraft process using the active folder and choose **Recheck**. If the guard is unavailable, wait for a successful check rather than editing files concurrently. |
| **A datapack installs as Locked** | The file was copied safely but left disabled. Close the world or game, recheck, then enable the datapack. |
| **A world has no datapack controls** | The world is read-only because its metadata or required files cannot be read. Restore `level.dat` from Settings if a backup exists. |
| **Discover is offline or fails** | Check your connection and use **Retry**. Temporary Modrinth 429 and 5xx responses are retried automatically; invalid hashes and invalid archives are not. |
| **No compatible version is preselected** | Check the version list and target game or world version. For datapacks, choose a release that explicitly supports the world. |
| **A pack archive reads as broken** | Mine Utils supports zip archives using Stored or Deflate compression. Recompress archives that use bzip2, zstd, or lzma. |
| **A local pack icon does not appear** | This is a known limitation of the local `asset://` icon path. The pack can still be managed, and Modrinth result icons are unaffected. |
| **The app reports an unreadable config** | Mine Utils preserves a copy of the bad config when possible, reports its path in a startup warning, and starts with defaults. Select your game folder again after reviewing the warning. |

For file locations and implementation details, see [ARCHITECTURE.md](ARCHITECTURE.md). For demo and development commands, see [DEVELOPMENT.md](DEVELOPMENT.md).

## Recover an interrupted Replace

If Mine Utils stops during Replace, the original pack can remain under a hidden
`.mine-utils-replace-<original>.<pid>-<stamp>-<counter>.tmp` name.
Packs and the selected world's datapack panel show a recovery notice with the
original filename. Select **Restore** to move the held file or folder back.

The original name must be free. If another pack already uses it, recovery refuses
without replacing either entry. Move that pack out of the folder before retrying.
Close Minecraft before restoring. Recovery refuses symlinked folders and entries.
Entries owned by the current app process stay hidden because Replace may still
be using them. Restart Mine Utils to recover an entry left by that process.

Restore changes only the filename. It leaves `options.txt` and `level.dat`
unchanged because their references may already point at the original name.
Recovery refuses filesystems without atomic no-replace rename support. On Windows, Restore always shows a message asking you to rename the holding entry by hand.
