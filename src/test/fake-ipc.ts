import type { IpcClient } from '../lib/ipc';
import type { Config, GameDirEntry, GameDirState, Theme } from '../lib/types';
import { defaultConfig, IpcError } from '../lib/types';

export interface FakeClient extends IpcClient {
  readonly config: Config;
  readonly entries: GameDirEntry[];
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
      .filter((entry) => entry.stored)
      .map((entry) => ({ path: entry.path, source: entry.source })),
  };

  const state = (): GameDirState => ({ config, entries });

  return {
    get config() {
      return config;
    },
    get entries() {
      return entries;
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
      if (!entries.some((entry) => entry.path === path)) {
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
      entries = entries.filter((entry) => entry.path !== path);
      config = {
        ...config,
        game_dirs: config.game_dirs.filter((dir) => dir.path !== path),
        active_game_dir:
          config.active_game_dir === path ? null : config.active_game_dir,
      };
      return state();
    },
    async setActiveGameDir(path: string) {
      const entry = entries.find((candidate) => candidate.path === path);
      if (!entry) {
        throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
      }
      if (!entry.present) {
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
  };
}
