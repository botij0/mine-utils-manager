import { Channel, convertFileSrc, invoke } from '@tauri-apps/api/core';
import { mockClient } from './ipc.mock';
import { IpcError } from './types';
import { ERROR_CODES } from './generated/wire';
import type * as Wire from './generated/wire';
import type {
  BackupInfo,
  Config,
  DataSnapshot,
  DataStack,
  DatapackDeleteOutcome,
  DatapackImportOutcome,
  DatapackListing,
  GameDirEntry,
  GameDirState,
  GameGuard,
  ImportChoice,
  ImportOutcome,
  LevelBackupInfo,
  ModrinthInstallArgs,
  ModrinthInstallOutcome,
  ModrinthKind,
  ModrinthProject,
  ModrinthSearchPage,
  ModrinthSort,
  ModrinthTargets,
  ModrinthVersion,
  PackIcon,
  PackListing,
  ResourceStack,
  StackSnapshot,
  Theme,
  World,
} from './types';

/** The pack surface. The page is tested against this small interface. */
export interface PackClient {
  listResourcePacks(): Promise<PackListing>;
  importResourcePack(path: string, choice?: ImportChoice): Promise<ImportOutcome>;
  restoreResourceHolding(holdingName: string): Promise<void>;
  deleteResourcePack(path: string): Promise<void>;
  getResourceStack(): Promise<ResourceStack>;
  setResourcePackEnabled(filename: string, enabled: boolean): Promise<ResourceStack>;
  reorderResourceStack(order: string[], expected: StackSnapshot): Promise<ResourceStack>;
  listOptionsBackups(): Promise<BackupInfo[]>;
  restoreOptionsBackup(backup: string): Promise<ResourceStack>;
}

/** The world and datapack surface. The page is tested against this interface. */
export interface WorldsClient {
  listWorlds(): Promise<World[]>;
  getConfig(): Promise<Config>;
  setLastWorld(path: string): Promise<void>;
  listDatapacks(world: string): Promise<DatapackListing>;
  importDatapack(world: string, path: string, choice?: ImportChoice): Promise<DatapackImportOutcome>;
  restoreDatapackHolding(world: string, holdingName: string): Promise<void>;
  deleteDatapack(world: string, path: string): Promise<DatapackDeleteOutcome>;
  removeDatapackReference(world: string, filename: string): Promise<DataStack>;
  setDatapackEnabled(world: string, filename: string, enabled: boolean): Promise<DataStack>;
  reorderDatapacks(world: string, order: string[], expected: DataSnapshot): Promise<DataStack>;
  listLevelBackups(world: string): Promise<LevelBackupInfo[]>;
  listAllLevelBackups(): Promise<LevelBackupInfo[]>;
  restoreLevelBackup(world: string, backup: string): Promise<DataStack>;
}

/** The narrow surface `BackupsPanel` needs: options backups, the game guard,
 *  and every world's `level.dat` backups in one batch call. Tests fake this
 *  instead of the whole `IpcClient`. */
export type BackupsClient = Pick<PackClient, 'listOptionsBackups' | 'restoreOptionsBackup'> &
  Pick<WorldsClient, 'listLevelBackups' | 'listAllLevelBackups' | 'restoreLevelBackup'> &
  Pick<IpcClient, 'getGameGuard'>;

/** The Modrinth discovery surface. Search fires only from the Discover page:
 *  the local tabs never trigger a network request. */
export interface DiscoverClient {
  searchModrinth(
    text: string,
    kind: ModrinthKind,
    gameVersion?: string | null,
    sort?: ModrinthSort,
    offset?: number,
  ): Promise<ModrinthSearchPage>;
  getModrinthProject(projectId: string): Promise<ModrinthProject>;
  listModrinthVersions(projectId: string): Promise<ModrinthVersion[]>;
  modrinthTargets(world?: string | null): Promise<ModrinthTargets>;
  cancelModrinthInstall(installId: string): Promise<boolean>;
  installModrinthPack(
    args: ModrinthInstallArgs,
    onProgress?: (phase: Wire.ModrinthInstallPhase) => void,
  ): Promise<ModrinthInstallOutcome>;
}

export interface IpcClient extends PackClient, WorldsClient, DiscoverClient {
  listGameDirs(): Promise<GameDirEntry[]>;
  addGameDir(path: string): Promise<GameDirState>;
  removeGameDir(path: string): Promise<GameDirState>;
  setActiveGameDir(path: string): Promise<GameDirState>;
  getConfig(): Promise<Config>;
  setTheme(theme: Theme): Promise<Config>;
  getStartupWarning(): Promise<string | null>;
  /** Only the game-running guard; it never reads `options.txt`. */
  getGameGuard(): Promise<GameGuard>;
}

export function isTauriRuntime(win: Window = window): boolean {
  return '__TAURI_INTERNALS__' in win;
}

export function toAppError(err: unknown): IpcError {
  if (err instanceof IpcError) return err;
  if (typeof err === 'object' && err !== null && 'code' in err && 'message' in err) {
    if (typeof err.code === 'string' && typeof err.message === 'string') {
      const code = ERROR_CODES.find((code) => code === err.code) ?? 'unknown';
      const installedPath =
        'installed_path' in err && typeof err.installed_path === 'string'
          ? err.installed_path
          : undefined;
      return new IpcError(code, err.message, code === 'unknown' ? err.code : undefined, installedPath);
    }
  }
  if (err instanceof Error) return new IpcError('unknown', err.message);
  return new IpcError('unknown', String(err));
}

/** Without a folder there is nothing to guard, and every write already needs
 *  one, so `no_active_game_dir` is an answer rather than a failed check. */
export async function activeGameRunning(
  client: Pick<IpcClient, 'getGameGuard'>,
): Promise<GameGuard> {
  try {
    return await client.getGameGuard();
  } catch (err) {
    if (toAppError(err).code === 'no_active_game_dir') return { running: null, stale_locks: [], detection_supported: true, process_probe_available: true };
    throw err;
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toAppError(err);
  }
}

export type WirePack = Wire.Pack;
export type WireWorld = Wire.World;
export type WireImportOutcome = Wire.ImportOutcome;

export function withIconUrl<T extends { icon: { sha256: string; path: string } | null }>(
  item: T,
): Omit<T, 'icon'> & { icon: PackIcon | null } {
  return {
    ...item,
    icon: item.icon
      ? { sha256: item.icon.sha256, url: convertFileSrc(item.icon.path) }
      : null,
  };
}

export function listingWithIconUrls(
  listing: Wire.PackListing,
): PackListing {
  return { ...listing, packs: listing.packs.map(withIconUrl) };
}

export function outcomeWithIconUrl(outcome: WireImportOutcome): ImportOutcome {
  return outcome.status === 'imported'
    ? { ...outcome, pack: withIconUrl(outcome.pack) }
    : outcome;
}

const installRegistrations = new Map<string, Promise<void>>();

const tauriClient: IpcClient = {
  listGameDirs: () => call<GameDirEntry[]>('list_game_dirs'),
  addGameDir: (path) => call<GameDirState>('add_game_dir', { path }),
  removeGameDir: (path) => call<GameDirState>('remove_game_dir', { path }),
  setActiveGameDir: (path) => call<GameDirState>('set_active_game_dir', { path }),
  setLastWorld: (path) => call<void>('set_last_world', { path }),
  getConfig: () => call<Config>('get_config'),
  setTheme: (theme) => call<Config>('set_theme', { theme }),
  getStartupWarning: () => call<string | null>('get_startup_warning'),
  getGameGuard: () => call<GameGuard>('get_game_guard'),
  async listResourcePacks() {
    const listing = await call<Wire.PackListing>(
      'list_resource_packs',
    );
    return listingWithIconUrls(listing);
  },
  async importResourcePack(path, choice) {
    const outcome = await call<WireImportOutcome>('import_resource_pack', {
      path,
      choice: choice ?? 'default',
    });
    return outcomeWithIconUrl(outcome);
  },
  restoreResourceHolding: (holdingName) => call<void>('restore_resource_holding', { holdingName }),
  deleteResourcePack: (path) => call<void>('delete_resource_pack', { path }),
  getResourceStack: () => call<ResourceStack>('get_resource_stack'),
  setResourcePackEnabled: (filename, enabled) =>
    call<ResourceStack>('set_resource_pack_enabled', { filename, enabled }),
  reorderResourceStack: (order, expected) =>
    call<ResourceStack>('reorder_resource_stack', { order, expected }),
  listOptionsBackups: () => call<BackupInfo[]>('list_options_backups'),
  restoreOptionsBackup: (backup) =>
    call<ResourceStack>('restore_options_backup', { backup }),
  async listWorlds() {
    const worlds = await call<WireWorld[]>('list_worlds');
    return worlds.map(withIconUrl);
  },
  async listDatapacks(world: string) {
    const listing = await call<
      Wire.DatapackListing
    >('list_datapacks', { world });
    return {
      ...listing,
      packs: listingWithIconUrls(listing.packs),
      stack: listing.stack,
    };
  },
  importDatapack: (world, path, choice) =>
    call<DatapackImportOutcome>('import_datapack', {
      world,
      path,
      choice: choice ?? 'default',
    }),
  restoreDatapackHolding: (world, holdingName) => call<void>('restore_datapack_holding', { world, holdingName }),
  deleteDatapack: (world, path) =>
    call<DatapackDeleteOutcome>('delete_datapack', { world, path }),
  removeDatapackReference: (world, filename) =>
    call<DataStack>('remove_datapack_reference', { world, filename }),
  setDatapackEnabled: (world, filename, enabled) =>
    call<DataStack>('set_datapack_enabled', { world, filename, enabled }),
  reorderDatapacks: (world, order, expected) =>
    call<DataStack>('reorder_datapacks', { world, order, expected }),
  listLevelBackups: (world) => call<LevelBackupInfo[]>('list_level_backups', { world }),
  listAllLevelBackups: () => call<LevelBackupInfo[]>('list_all_level_backups'),
  restoreLevelBackup: (world, backup) =>
    call<DataStack>('restore_level_backup', { world, backup }),
  searchModrinth: (text, kind, gameVersion, sort, offset) =>
    call<ModrinthSearchPage>('search_modrinth', {
      query: text,
      kind,
      gameVersion: gameVersion ?? null,
      sort: sort ?? 'relevance',
      offset: offset ?? 0,
    }),
  getModrinthProject: (projectId) =>
    call<ModrinthProject>('get_modrinth_project', { projectId }),
  listModrinthVersions: (projectId) =>
    call<ModrinthVersion[]>('list_modrinth_versions', { projectId }),
  modrinthTargets: (world) =>
    call<ModrinthTargets>('modrinth_targets', { world: world ?? null }),
  cancelModrinthInstall: async (installId) => {
    const registration = installRegistrations.get(installId);
    if (!registration) return false;
    await registration;
    return call<boolean>('cancel_modrinth_install', { installId });
  },
  installModrinthPack: async (args, onProgress) => {
    const installId = args.installId ?? crypto.randomUUID();
    const registration = call<void>('begin_modrinth_install', { installId });
    installRegistrations.set(installId, registration);
    try {
      await registration;
      const outcome = await call<Wire.ModrinthInstallOutcome>('install_modrinth_pack', {
        installId,
        projectId: args.projectId,
        versionId: args.versionId,
        filename: args.filename,
        kind: args.kind,
        world: args.world ?? null,
        progress: new Channel<Wire.ModrinthInstallPhase>(onProgress),
      });
      if (outcome.kind === 'resourcepack') {
        return {
          ...outcome,
          outcome: outcomeWithIconUrl(outcome.outcome),
        };
      }
      return outcome;
    } finally {
      installRegistrations.delete(installId);
    }
  },
};

/**
 * The one client the UI talks to. It picks the Tauri invoke transport when the
 * runtime is present and the localStorage mock otherwise.
 */
export const ipc: IpcClient = {
  listGameDirs: () => pick().listGameDirs(),
  addGameDir: (path) => pick().addGameDir(path),
  removeGameDir: (path) => pick().removeGameDir(path),
  setActiveGameDir: (path) => pick().setActiveGameDir(path),
  setLastWorld: (path) => pick().setLastWorld(path),
  getConfig: () => pick().getConfig(),
  setTheme: (theme) => pick().setTheme(theme),
  getStartupWarning: () => pick().getStartupWarning(),
  getGameGuard: () => pick().getGameGuard(),
  listResourcePacks: () => pick().listResourcePacks(),
  importResourcePack: (path, choice) => pick().importResourcePack(path, choice),
  restoreResourceHolding: (holdingName) => pick().restoreResourceHolding(holdingName),
  deleteResourcePack: (path) => pick().deleteResourcePack(path),
  getResourceStack: () => pick().getResourceStack(),
  setResourcePackEnabled: (filename, enabled) =>
    pick().setResourcePackEnabled(filename, enabled),
  reorderResourceStack: (order, expected) =>
    pick().reorderResourceStack(order, expected),
  listOptionsBackups: () => pick().listOptionsBackups(),
  restoreOptionsBackup: (backup) => pick().restoreOptionsBackup(backup),
  listWorlds: () => pick().listWorlds(),
  listDatapacks: (world) => pick().listDatapacks(world),
  importDatapack: (world, path, choice) => pick().importDatapack(world, path, choice),
  restoreDatapackHolding: (world, holdingName) => pick().restoreDatapackHolding(world, holdingName),
  deleteDatapack: (world, path) => pick().deleteDatapack(world, path),
  removeDatapackReference: (world, filename) =>
    pick().removeDatapackReference(world, filename),
  setDatapackEnabled: (world, filename, enabled) =>
    pick().setDatapackEnabled(world, filename, enabled),
  reorderDatapacks: (world, order, expected) =>
    pick().reorderDatapacks(world, order, expected),
  listLevelBackups: (world) => pick().listLevelBackups(world),
  listAllLevelBackups: () => pick().listAllLevelBackups(),
  restoreLevelBackup: (world, backup) => pick().restoreLevelBackup(world, backup),
  searchModrinth: (text, kind, gameVersion, sort, offset) =>
    pick().searchModrinth(text, kind, gameVersion, sort, offset),
  getModrinthProject: (projectId) => pick().getModrinthProject(projectId),
  listModrinthVersions: (projectId) => pick().listModrinthVersions(projectId),
  modrinthTargets: (world) => pick().modrinthTargets(world),
  cancelModrinthInstall: (installId) => pick().cancelModrinthInstall(installId),
  installModrinthPack: (args, onProgress) => pick().installModrinthPack(args, onProgress),
};

function pick(): IpcClient {
  return isTauriRuntime() ? tauriClient : mockClient;
}

/**
 * Open the native picker. `directory` picks a pack folder instead of a zip;
 * the mock runtime has no picker, so it returns null.
 */
export async function pickPackPath(
  directory = false,
  title?: string,
  extensions?: string[],
): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const selected = await call<string | string[] | null>('plugin:dialog|open', {
    options: {
      title:
        title ?? (directory ? 'Import a pack folder' : 'Import a resource pack'),
      multiple: false,
      directory,
      filters: directory
        ? undefined
        : [{ name: 'Packs', extensions: extensions ?? ['zip'] }],
    },
  });
  if (Array.isArray(selected)) return selected[0] ?? null;
  return selected;
}
