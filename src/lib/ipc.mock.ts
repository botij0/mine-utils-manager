import type { IpcClient } from './ipc';
import { defaultConfig, IpcError } from './types';
import type { Config, GameDirEntry, GameDirState, Theme } from './types';

const STORAGE_KEY = 'mine-utils:config';

const MOCK_DIRS: GameDirEntry[] = [
  {
    path: '/home/demo/.minecraft',
    source: 'std',
    present: true,
    stored: false,
  },
  {
    path: '/home/demo/.var/app/com.mojang.Minecraft/data/minecraft',
    source: 'flatpak',
    present: true,
    stored: false,
  },
  {
    path: '/home/demo/.local/share/PrismLauncher/instances/1.21/.minecraft',
    source: 'prism_launcher',
    present: false,
    stored: false,
  },
];

const LATENCY_MS = 60;

function delay(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, LATENCY_MS));
}

function loadConfig(): Config {
  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (!raw) return defaultConfig();
  try {
    const parsed = JSON.parse(raw) as Partial<Config>;
    const base = defaultConfig();
    return {
      ...base,
      ...parsed,
      game_dirs: Array.isArray(parsed.game_dirs) ? parsed.game_dirs : base.game_dirs,
      backups: { ...base.backups, ...(parsed.backups ?? {}) },
    };
  } catch {
    window.localStorage.removeItem(STORAGE_KEY);
    return defaultConfig();
  }
}

function storeConfig(config: Config): Config {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  return config;
}

function listing(config: Config): GameDirEntry[] {
  const stored = new Set(config.game_dirs.map((dir) => dir.path));
  const seen = new Set(MOCK_DIRS.map((dir) => dir.path));
  const entries = MOCK_DIRS.map((dir) => ({
    ...dir,
    stored: stored.has(dir.path),
  }));
  for (const dir of config.game_dirs) {
    if (seen.has(dir.path)) continue;
    seen.add(dir.path);
    entries.push({
      path: dir.path,
      source: dir.source,
      present: true,
      stored: true,
    });
  }
  return entries;
}

function makeState(config: Config): GameDirState {
  return { config, entries: listing(config) };
}

function looksLikeGameDir(path: string): boolean {
  return (
    path.includes('.minecraft') ||
    /\/(saves|versions|resourcepacks)(\/|$)/.test(path)
  );
}

/**
 * Same interface as the Tauri client, backed by localStorage. The mock can not
 * stat the disk, so it uses the path markers the backend checks.
 */
export const mockClient: IpcClient = {
  async listGameDirs() {
    await delay();
    return listing(loadConfig());
  },
  async addGameDir(path: string) {
    await delay();
    if (!looksLikeGameDir(path)) {
      throw new IpcError(
        'invalid_game_dir',
        `invalid game dir ${path}: expected one of versions, saves, or resourcepacks`,
      );
    }
    const config = loadConfig();
    if (!config.game_dirs.some((dir) => dir.path === path)) {
      config.game_dirs = [...config.game_dirs, { path, source: 'manual' }];
    }
    storeConfig(config);
    return makeState(config);
  },
  async removeGameDir(path: string) {
    await delay();
    const config = loadConfig();
    if (!config.game_dirs.some((dir) => dir.path === path)) {
      throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
    }
    config.game_dirs = config.game_dirs.filter((dir) => dir.path !== path);
    if (config.active_game_dir === path) config.active_game_dir = null;
    storeConfig(config);
    return makeState(config);
  },
  async setActiveGameDir(path: string) {
    await delay();
    const config = loadConfig();
    const entry = listing(config).find((dir) => dir.path === path);
    if (!entry) {
      throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
    }
    if (!entry.present) {
      throw new IpcError(
        'invalid_game_dir',
        `invalid game dir ${path}: expected one of versions, saves, or resourcepacks`,
      );
    }
    config.active_game_dir = path;
    storeConfig(config);
    return makeState(config);
  },
  async getConfig() {
    await delay();
    return loadConfig();
  },
  async setTheme(theme: Theme) {
    await delay();
    const config = loadConfig();
    config.theme = theme;
    storeConfig(config);
    return config;
  },
  async getStartupWarning() {
    await delay();
    return null;
  },
};
