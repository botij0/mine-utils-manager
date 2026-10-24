import type { IpcClient } from './ipc';
import { defaultConfig, IpcError } from './types';
import type { Config, GameDirEntry } from './types';

const STORAGE_KEY = 'mine-utils:config';

const MOCK_DIRS: GameDirEntry[] = [
  {
    path: '/home/demo/.minecraft',
    source: 'std',
    present: true,
  },
  {
    path: '/home/demo/.var/app/com.mojang.Minecraft/data/minecraft',
    source: 'flatpak',
    present: true,
  },
  {
    path: '/home/demo/.local/share/PrismLauncher/instances/1.21/.minecraft',
    source: 'prism_launcher',
    present: false,
  },
];

const LATENCY_MS = 60;

function delay(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, LATENCY_MS));
}

function loadConfig(): Config {
  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (!raw) return defaultConfig();
  return JSON.parse(raw) as Config;
}

function storeConfig(config: Config): Config {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  return config;
}

function listing(config: Config): GameDirEntry[] {
  const seen = new Set(MOCK_DIRS.map((dir) => dir.path));
  const entries = [...MOCK_DIRS];
  for (const dir of config.game_dirs) {
    if (seen.has(dir.path)) continue;
    seen.add(dir.path);
    entries.push({ ...dir, present: true });
  }
  return entries;
}

/**
 * Same interface as the Tauri client, backed by localStorage. The mock can not
 * stat the disk, so an add is valid when the path is absolute.
 */
export const mockClient: IpcClient = {
  async listGameDirs() {
    await delay();
    return listing(loadConfig());
  },
  async addGameDir(path: string) {
    await delay();
    if (!path.startsWith('/')) {
      throw new IpcError(
        'invalid_game_dir',
        `invalid game dir ${path}: path must be absolute`,
      );
    }
    const config = loadConfig();
    config.game_dirs = [
      ...config.game_dirs.filter((dir) => dir.path !== path),
      { path, source: 'manual' },
    ];
    storeConfig(config);
    return listing(config);
  },
  async removeGameDir(path: string) {
    await delay();
    const config = loadConfig();
    config.game_dirs = config.game_dirs.filter((dir) => dir.path !== path);
    if (config.active_game_dir === path) config.active_game_dir = null;
    storeConfig(config);
    return listing(config);
  },
  async setActiveGameDir(path: string) {
    await delay();
    const config = loadConfig();
    config.active_game_dir = path;
    return storeConfig(config);
  },
  async getConfig() {
    await delay();
    return loadConfig();
  },
  async saveConfig(config: Config) {
    await delay();
    return storeConfig(config);
  },
};
