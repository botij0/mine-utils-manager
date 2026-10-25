import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { mockClient } from './ipc.mock';
import { IpcError } from './types';
import type {
  BackupInfo,
  Config,
  GameDirEntry,
  GameDirState,
  ImportChoice,
  ImportOutcome,
  Pack,
  PackListing,
  PackMeta,
  ResourceStack,
  StackSnapshot,
  Theme,
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

export interface IpcClient extends PackClient {
  listGameDirs(): Promise<GameDirEntry[]>;
  addGameDir(path: string): Promise<GameDirState>;
  removeGameDir(path: string): Promise<GameDirState>;
  setActiveGameDir(path: string): Promise<GameDirState>;
  getConfig(): Promise<Config>;
  setTheme(theme: Theme): Promise<Config>;
  getStartupWarning(): Promise<string | null>;
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

/** The import command returns icon paths too; they are converted like listing. */
export type WireImportOutcome =
  | { status: 'imported'; pack: WirePack }
  | Exclude<ImportOutcome, { status: 'imported' }>;

export function packWithIconUrl(pack: WirePack): Pack {
  return {
    ...pack,
    icon: pack.icon
      ? { sha256: pack.icon.sha256, url: convertFileSrc(pack.icon.path) }
      : null,
  };
}

export function listingWithIconUrls(
  listing: Omit<PackListing, 'packs'> & { packs: WirePack[] },
): PackListing {
  return { ...listing, packs: listing.packs.map(packWithIconUrl) };
}

export function outcomeWithIconUrl(outcome: WireImportOutcome): ImportOutcome {
  return outcome.status === 'imported'
    ? { ...outcome, pack: packWithIconUrl(outcome.pack) }
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
};

function pick(): IpcClient {
  return isTauriRuntime() ? tauriClient : mockClient;
}

/**
 * Open the native picker. `directory` picks a pack folder instead of a zip;
 * the mock runtime has no picker, so it returns null.
 */
export async function pickPackPath(directory = false): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const selected = await call<string | string[] | null>('plugin:dialog|open', {
    options: {
      title: directory ? 'Import a pack folder' : 'Import a resource pack',
      multiple: false,
      directory,
      filters: directory
        ? undefined
        : [{ name: 'Resource packs', extensions: ['zip'] }],
    },
  });
  if (Array.isArray(selected)) return selected[0] ?? null;
  return selected;
}

/** Empty meta for mock fixtures and tests. */
export function emptyMeta(description = ''): PackMeta {
  return {
    pack_format: null,
    min_format: null,
    max_format: null,
    supported_formats: null,
    description,
  };
}
