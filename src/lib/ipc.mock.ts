import type { IpcClient } from './ipc';
import { emptyMeta } from './ipc';
import { defaultConfig, IpcError } from './types';
import type {
  Config,
  FormatRange,
  GameDirEntry,
  GameDirState,
  ImportChoice,
  ImportOutcome,
  Pack,
  PackKind,
  PackListing,
  Theme,
} from './types';

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

function iconDataUrl(label: string, background: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="10" fill="${background}"/><text x="32" y="40" font-family="sans-serif" font-size="26" font-weight="700" text-anchor="middle" fill="#fff">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

function version(major: number, minor = 0) {
  return { major, minor };
}

function range(min: number, max: number): FormatRange {
  return { min: version(min), max: version(max) };
}

function makePack(
  name: string,
  kind: PackKind,
  formatRange: FormatRange,
  gameLabel: string,
  description: string,
  icon: string | null,
): Pack {
  const location = name.endsWith('.zip') ? 'zip' : 'directory';
  return {
    id: `/home/demo/.minecraft/resourcepacks/${name}`,
    name: stripZip(name),
    path: `/home/demo/.minecraft/resourcepacks/${name}`,
    location,
    kind,
    description,
    meta: { ...emptyMeta(description), supported_formats: formatRange },
    format: formatRange.min,
    format_range: formatRange,
    game_label: gameLabel,
    icon: icon ? { sha256: `mock-${name}`, url: icon } : null,
    error: null,
    bytes_read: 4096,
  };
}

const INITIAL_PACKS: Pack[] = [
  makePack(
    'VanillaTweaks_r656611_MC1.21.x.zip',
    'resource',
    range(34, 69),
    '1.21 to 1.21.10',
    'Vanilla Tweaks 1.21, small tweaks for blocks, items, and UI.',
    iconDataUrl('VT', '#3f7d5a'),
  ),
  makePack(
    'Re-covered.zip',
    'resource',
    range(48, 70),
    '1.21.2 to 1.21.5',
    'New book covers for enchanted books.',
    iconDataUrl('RC', '#8a5a3f'),
  ),
  makePack(
    'AutumnPack',
    'data',
    range(48, 57),
    '1.21 to 1.21.3',
    'Autumn recipes and loot tables for a seasonal server.',
    iconDataUrl('AP', '#a05a2c'),
  ),
  makePack(
    'OverlayTools',
    'both',
    range(34, 42),
    '1.21 to 1.21.3',
    'Resource and data roots in one folder pack.',
    null,
  ),
  {
    ...makePack(
      'BrokenPack.zip',
      'unknown',
      range(0, 0),
      '',
      '',
      null,
    ),
    location: 'zip',
    format: null,
    format_range: null,
    game_label: null,
    error: {
      code: 'pack_broken_archive',
      message: 'BrokenPack.zip is not a readable pack: invalid Zip archive',
    },
  },
];

let mockPacks = [...INITIAL_PACKS];

function stripZip(name: string): string {
  return name.replace(/\.zip$/i, '');
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

function packListing(): PackListing {
  return {
    packs: mockPacks.map((pack) => ({ ...pack })),
    count: mockPacks.length,
    bytes_read: mockPacks.reduce((total, pack) => total + pack.bytes_read, 0),
    elapsed_ms: 3.2,
  };
}

/**
 * Same interface as the Tauri client, backed by localStorage and an in-memory
 * pack list. The mock can not stat the disk, so it uses path markers and a few
 * synthetic packs to exercise every UI state.
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
  async listResourcePacks() {
    await delay();
    return packListing();
  },
  async importResourcePack(path: string, choice: ImportChoice = 'default'): Promise<ImportOutcome> {
    await delay();
    const base = path.split('/').filter(Boolean).at(-1) ?? path;
    const lower = base.toLowerCase();
    if (lower.includes('broken') || lower.includes('corrupt')) {
      throw new IpcError(
        'pack_broken_archive',
        `${base} is not a readable pack: invalid Zip archive`,
      );
    }
    const name = stripZip(base);
    if (lower.includes('duplicate')) {
      if (choice === 'skip') return { status: 'skipped' };
      if (choice !== 'replace') {
        return {
          status: 'duplicate',
          name: base,
          sha256: 'mock-duplicate',
          existing: mockPacks[0]?.path ?? path,
        };
      }
    }
    const existing = mockPacks.find((pack) => stripZip(pack.name) === name && !pack.error);
    if (existing) {
      if (choice === 'skip') return { status: 'skipped' };
      if (choice !== 'replace') {
        return {
          status: 'duplicate',
          name: base,
          sha256: `mock-${name}`,
          existing: existing.path,
        };
      }
      return { status: 'imported', pack: existing };
    }
    if (name === 'Collision') {
      if (choice !== 'replace') return { status: 'collision', name: base };
    }
    const formatRange = range(34, 42);
    const pack: Pack = {
      ...makePack(
        name,
        'resource',
        formatRange,
        '1.21 to 1.21.3',
        'Imported pack from the browser mock.',
        iconDataUrl(name.slice(0, 2).toUpperCase() || 'PK', '#4a5f8a'),
      ),
      path: `/home/demo/.minecraft/resourcepacks/${base}`,
      location: 'zip',
    };
    mockPacks = [...mockPacks, pack];
    return { status: 'imported', pack };
  },
  async deleteResourcePack(path: string) {
    await delay();
    if (!mockPacks.some((pack) => pack.path === path)) {
      throw new IpcError('pack_not_found', `pack ${path} does not exist`);
    }
    mockPacks = mockPacks.filter((pack) => pack.path !== path);
  },
};

/** Reset the mock pack list between browser demos or tests. */
export function resetMockPacks() {
  mockPacks = [...INITIAL_PACKS];
}
