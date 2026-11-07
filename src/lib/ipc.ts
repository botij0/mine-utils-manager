import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { mockClient } from './ipc.mock';
import { IpcError } from './types';
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
  Pack,
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
  listDatapacks(world: string): Promise<DatapackListing>;
  importDatapack(world: string, path: string, choice?: ImportChoice): Promise<DatapackImportOutcome>;
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
export type BackupsClient = Pick<
  PackClient,
  'listOptionsBackups' | 'restoreOptionsBackup' | 'getResourceStack'
> &
  Pick<WorldsClient, 'listLevelBackups' | 'listAllLevelBackups' | 'restoreLevelBackup'>;

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
  installModrinthPack(args: ModrinthInstallArgs): Promise<ModrinthInstallOutcome>;
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
  if (typeof err === 'object' && err !== null && 'code' in err && 'message' in err) {
    const wire = err as { code: unknown; message: unknown };
    if (typeof wire.code === 'string' && typeof wire.message === 'string') {
      return new IpcError(wire.code, wire.message);
    }
  }
  if (err instanceof Error) return new IpcError('unknown', err.message);
  return new IpcError('unknown', String(err));
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toAppError(err);
  }
}

/**
 * The backend stores icon files and returns their paths. The asset protocol
 * turns them into URLs the webview may load.
 */
export interface WirePack extends Omit<Pack, 'icon'> {
  icon: { sha256: string; path: string } | null;
}

export interface WireWorld extends Omit<World, 'icon'> {
  icon: { sha256: string; path: string } | null;
}

/** The import command returns icon paths too; they are converted like listing. */
export type WireImportOutcome =
  | { status: 'imported'; pack: WirePack }
  | Exclude<ImportOutcome, { status: 'imported' }>;

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
  listing: Omit<PackListing, 'packs'> & { packs: WirePack[] },
): PackListing {
  return { ...listing, packs: listing.packs.map(withIconUrl) };
}

export function outcomeWithIconUrl(outcome: WireImportOutcome): ImportOutcome {
  return outcome.status === 'imported'
    ? { ...outcome, pack: withIconUrl(outcome.pack) }
    : outcome;
}

const tauriClient: IpcClient = {
  listGameDirs: () => call<GameDirEntry[]>('list_game_dirs'),
  addGameDir: (path) => call<GameDirState>('add_game_dir', { path }),
  removeGameDir: (path) => call<GameDirState>('remove_game_dir', { path }),
  setActiveGameDir: (path) => call<GameDirState>('set_active_game_dir', { path }),
  getConfig: () => call<Config>('get_config'),
  setTheme: (theme) => call<Config>('set_theme', { theme }),
  getStartupWarning: () => call<string | null>('get_startup_warning'),
  getGameGuard: () => call<GameGuard>('get_game_guard'),
  async listResourcePacks() {
    const listing = await call<Omit<PackListing, 'packs'> & { packs: WirePack[] }>(
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
      Omit<DatapackListing, 'packs'> & {
        packs: Omit<PackListing, 'packs'> & { packs: WirePack[] };
      }
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
  installModrinthPack: async (args) => {
    const outcome = await call<ModrinthInstallOutcome>('install_modrinth_pack', {
      projectId: args.projectId,
      versionId: args.versionId,
      filename: args.filename,
      kind: args.kind,
      world: args.world ?? null,
    });
    // The resource outcome carries a server-side icon path; convert it to a
    // URL exactly like a manual Packs import. The wire kind is `resourcepack`.
    if (outcome.kind === 'resourcepack') {
      return {
        ...outcome,
        outcome: outcomeWithIconUrl(outcome.outcome as WireImportOutcome),
      };
    }
    return outcome;
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
  getConfig: () => pick().getConfig(),
  setTheme: (theme) => pick().setTheme(theme),
  getStartupWarning: () => pick().getStartupWarning(),
  getGameGuard: () => pick().getGameGuard(),
  listResourcePacks: () => pick().listResourcePacks(),
  importResourcePack: (path, choice) => pick().importResourcePack(path, choice),
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
  installModrinthPack: (args) => pick().installModrinthPack(args),
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
