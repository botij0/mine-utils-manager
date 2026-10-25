import type { IpcClient } from '../lib/ipc';
import type {
  BackupInfo,
  Config,
  GameDirEntry,
  GameDirState,
  ResourceStack,
  StackEntry,
  Theme,
} from '../lib/types';
import { defaultConfig, IpcError } from '../lib/types';

export interface FakeClient extends IpcClient {
  readonly config: Config;
  readonly entries: GameDirEntry[];
  readonly stack: ResourceStack;
}

export function gameDir(
  path: string,
  present = true,
  stored = true,
): GameDirEntry {
  return {
    path,
    source: stored ? 'manual' : 'std',
    present,
    stored,
  };
}

function entry(value: string): StackEntry {
  return value.startsWith('file/')
    ? { kind: 'file', value, filename: value.slice('file/'.length) }
    : { kind: 'foreign', value };
}

function looksLikeGameDir(path: string): boolean {
  return (
    path.includes('.minecraft') ||
    /\/(saves|versions|resourcepacks)(\/|$)/.test(path)
  );
}

export function makeFakeClient(initial: GameDirEntry[]): FakeClient {
  let entries = [...initial];
  let config: Config = {
    ...defaultConfig(),
    game_dirs: initial
      .filter((item) => item.stored)
      .map((item) => ({ path: item.path, source: item.source })),
  };
  let stack: ResourceStack = {
    entries: ['vanilla', 'fabric'].map(entry),
    incompatible: [],
    running: null,
    stale_locks: [],
    detection_supported: true,
  };
  const backups: BackupInfo[] = [];

  const state = (): GameDirState => ({ config, entries });

  return {
    get config() {
      return config;
    },
    get entries() {
      return entries;
    },
    get stack() {
      return stack;
    },
    async listGameDirs() {
      return entries;
    },
    async addGameDir(path: string) {
      if (!looksLikeGameDir(path)) {
        throw new IpcError(
          'invalid_game_dir',
          `invalid game dir ${path}: expected one of versions, saves, or resourcepacks`,
        );
      }
      if (!entries.some((item) => item.path === path)) {
        entries = [...entries, gameDir(path, true, true)];
      }
      if (!config.game_dirs.some((dir) => dir.path === path)) {
        config = {
          ...config,
          game_dirs: [...config.game_dirs, { path, source: 'manual' }],
        };
      }
      return state();
    },
    async removeGameDir(path: string) {
      if (!config.game_dirs.some((dir) => dir.path === path)) {
        throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
      }
      entries = entries.filter((item) => item.path !== path);
      config = {
        ...config,
        game_dirs: config.game_dirs.filter((dir) => dir.path !== path),
        active_game_dir:
          config.active_game_dir === path ? null : config.active_game_dir,
      };
      return state();
    },
    async setActiveGameDir(path: string) {
      const target = entries.find((candidate) => candidate.path === path);
      if (!target) {
        throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
      }
      if (!target.present) {
        throw new IpcError(
          'invalid_game_dir',
          `invalid game dir ${path}: expected one of versions, saves, or resourcepacks`,
        );
      }
      config = { ...config, active_game_dir: path };
      return state();
    },
    async getConfig() {
      return config;
    },
    async setTheme(theme: Theme) {
      config = { ...config, theme };
      return config;
    },
    async getStartupWarning() {
      return null;
    },
    async listResourcePacks() {
      return { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 };
    },
    async importResourcePack() {
      throw new IpcError('pack_unknown_kind', 'fake client has no pack importer');
    },
    async deleteResourcePack() {
      throw new IpcError('pack_not_found', 'fake client has no pack store');
    },
    async getResourceStack() {
      return stack;
    },
    async setResourcePackEnabled(filename: string, enabled: boolean) {
      const value = `file/${filename}`;
      if (enabled) {
        stack = {
          ...stack,
          entries: [...stack.entries, entry(value)],
        };
      } else {
        stack = {
          ...stack,
          entries: stack.entries.filter((item) => item.value !== value),
          incompatible: stack.incompatible.filter((item) => item !== value),
        };
      }
      return stack;
    },
    async reorderResourceStack(order: string[]) {
      stack = {
        ...stack,
        entries: order.map(entry),
      };
      return stack;
    },
    async listOptionsBackups() {
      return backups;
    },
    async restoreOptionsBackup() {
      return stack;
    },
  };
}
