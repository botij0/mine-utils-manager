import type { IpcClient } from './ipc';
import { emptyMeta } from './ipc';
import { defaultConfig, IpcError } from './types';
import type {
  BackupInfo,
  Config,
  DataSnapshot,
  DataStack,
  DataStackEntry,
  DatapackDeleteOutcome,
  DatapackImportOutcome,
  DatapackListing,
  FormatRange,
  GameDirEntry,
  GameDirState,
  ImportChoice,
  ImportOutcome,
  LevelBackupInfo,
  Pack,
  PackKind,
  PackListing,
  ResourceStack,
  StackEntry,
  Theme,
  World,
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
    mockStack = {
      ...mockStack,
      resource: mockStack.resource.filter((entry) => !path.endsWith(`/${entry.slice('file/'.length)}`)),
    };
  },
  async getResourceStack() {
    await delay();
    return mockResourceStack();
  },
  async setResourcePackEnabled(filename: string, enabled: boolean) {
    await delay();
    assertWritable();
    const value = `file/${filename}`;
    if (enabled) {
      if (!mockStack.resource.includes(value)) {
        mockStack = { ...mockStack, resource: [...mockStack.resource, value] };
      }
    } else {
      mockStack = {
        resource: mockStack.resource.filter((entry) => entry !== value),
        incompatible: mockStack.incompatible.filter((entry) => entry !== value),
      };
    }
    return mockResourceStack();
  },
  async reorderResourceStack(order: string[]) {
    await delay();
    assertWritable();
    mockReorder(order);
    return mockResourceStack();
  },
  async listOptionsBackups() {
    await delay();
    if (!loadConfig().active_game_dir) {
      throw new IpcError(
        'no_active_game_dir',
        'no active game dir; pick one in Settings first',
      );
    }
    return mockBackups.map(({ file_name, bytes, display }) => ({
      file_name,
      bytes,
      display,
    }));
  },
  async restoreOptionsBackup(backup: string) {
    await delay();
    assertWritable();
    const found = mockBackups.find((entry) => entry.file_name === backup);
    if (!found) {
      throw new IpcError('backup', `backup failed: no backup named ${backup}`);
    }
    mockBackups = [
      {
        file_name: `${Date.now()}-options.txt`,
        bytes: mockStack.resource.join(',').length,
        display: 'restored copy',
        resource: [...mockStack.resource],
        incompatible: [...mockStack.incompatible],
      },
      ...mockBackups,
    ];
    mockStack = {
      resource: [...found.resource],
      incompatible: [...found.incompatible],
    };
    return mockResourceStack();
  },
  async listWorlds() {
    await delay();
    return mockWorldList();
  },
  async listDatapacks(world: string) {
    await delay();
    return mockDatapackListing(world);
  },
  async importDatapack(
    world: string,
    path: string,
    choice: ImportChoice = 'default',
  ): Promise<DatapackImportOutcome> {
    await delay();
    return mockImportDatapack(world, path, choice);
  },
  async deleteDatapack(world: string, path: string): Promise<DatapackDeleteOutcome> {
    await delay();
    mockDeleteDatapack(world, path);
    return { removed_references: true, stack: mockDataStack(world) };
  },
  async setDatapackEnabled(world: string, filename: string, enabled: boolean) {
    await delay();
    mockAssertDatapackWritable(world);
    mockSetDatapackEnabled(world, filename, enabled);
    return mockDataStack(world);
  },
  async reorderDatapacks(world: string, order: string[], expected: DataSnapshot) {
    await delay();
    mockAssertDatapackWritable(world);
    mockReorderDatapacks(world, order, expected);
    return mockDataStack(world);
  },
  async listLevelBackups(world: string) {
    await delay();
    return mockLevelBackups
      .filter((entry) => entry.world === world)
      .map(({ file_name, bytes, display, world: entryWorld }) => ({
        file_name,
        bytes,
        display,
        world: entryWorld,
      }));
  },
  async restoreLevelBackup(world: string, backup: string) {
    await delay();
    mockAssertDatapackWritable(world);
    const found = mockLevelBackups.find(
      (entry) => entry.world === world && entry.file_name === backup,
    );
    if (!found) {
      throw new IpcError('level_backup', `level backup failed: no backup named ${backup}`);
    }
    mockWorldStacks[world] = {
      enabled: [...found.enabled],
      disabled: [...found.disabled],
    };
    return mockDataStack(world);
  },
};

/** Reset the mock pack list between browser demos or tests. */
export function resetMockPacks() {
  mockPacks = [...INITIAL_PACKS];
}

type MockArrayState = { resource: string[]; incompatible: string[] };

const INITIAL_STACK: MockArrayState = {
  resource: [
    'vanilla',
    'fabric',
    'file/VanillaTweaks_r656611_MC1.21.x.zip',
    'file/Re-covered.zip',
  ],
  incompatible: [],
};

interface MockBackup extends BackupInfo {
  resource: string[];
  incompatible: string[];
}

const INITIAL_BACKUPS: MockBackup[] = [
  {
    file_name: '20261025T101500.000000000-options.txt',
    bytes: 4066,
    display: '2026-10-25 10:15:00 UTC',
    resource: [...INITIAL_STACK.resource],
    incompatible: [],
  },
  {
    file_name: '20261025T100000.000000000-options.txt',
    bytes: 4024,
    display: '2026-10-25 10:00:00 UTC',
    resource: ['vanilla', 'fabric', 'file/Re-covered.zip'],
    incompatible: [],
  },
];

let mockStack: MockArrayState = {
  resource: [...INITIAL_STACK.resource],
  incompatible: [],
};
let mockBackups: MockBackup[] = [...INITIAL_BACKUPS];
let mockGameState: MockGameState = 'none';

/**
 * What the game guard reports in the browser mock. `running` blocks writes
 * with a banner naming the session lock. `stale-lock` keeps a session.lock
 * nobody holds, which never blocks and is shown as an informational note.
 */
export type MockGameState = 'none' | 'running' | 'stale-lock';

export function setMockGameState(state: MockGameState) {
  mockGameState = state;
}

/** Reset the mock stack and backups between browser demos or tests. */
export function resetMockStack() {
  mockStack = { resource: [...INITIAL_STACK.resource], incompatible: [] };
  mockBackups = [...INITIAL_BACKUPS];
  mockGameState = 'none';
  mockWorldStacks = JSON.parse(JSON.stringify(INITIAL_WORLD_STACKS)) as typeof INITIAL_WORLD_STACKS;
  mockLevelBackups = [...INITIAL_LEVEL_BACKUPS];
  mockLockedWorld = null;
  mockWorldPacks = JSON.parse(JSON.stringify(INITIAL_WORLD_PACKS)) as typeof INITIAL_WORLD_PACKS;
}

/** A datapack installed in a mock world. The format range drives the warning chip. */
export interface MockWorldPack {
  filename: string;
  kind: PackKind;
  minFormat: number | null;
  maxFormat: number | null;
  gameLabel: string;
}

const SURVIVAL_PACKS: MockWorldPack[] = [
  { filename: 'armored elytra v1.0.4 (MC 1.17.x).zip', kind: 'data', minFormat: 7, maxFormat: 7, gameLabel: '1.17 to 1.17.1' },
  { filename: 'double shulker shells v1.3.1 (MC 1.17.x).zip', kind: 'data', minFormat: 7, maxFormat: 7, gameLabel: '1.17 to 1.17.1' },
  { filename: 'dragon drops v1.3.1 (MC 1.17.x).zip', kind: 'data', minFormat: 7, maxFormat: 7, gameLabel: '1.17 to 1.17.1' },
  { filename: 'fast leaf decay v2.0.4 (MC 1.17.x).zip', kind: 'data', minFormat: 7, maxFormat: 7, gameLabel: '1.17 to 1.17.1' },
  { filename: 'graves v2.8.5 (MC 1.17.x).zip', kind: 'data', minFormat: 7, maxFormat: 7, gameLabel: '1.17 to 1.17.1' },
];

const INITIAL_WORLD_STACKS: Record<string, { enabled: string[]; disabled: string[] }> = {
  'Survival 1_17': {
    enabled: [
      'vanilla',
      ...SURVIVAL_PACKS.map((pack) => `file/${pack.filename}`),
      'fabric',
      'fabric-convention-tags-v2',
    ],
    disabled: ['minecart_improvements', 'redstone_experiments', 'trade_rebalance'],
  },
  'Creative Flat': {
    enabled: ['vanilla', 'fabric'],
    disabled: [],
  },
  'Locked World': {
    enabled: ['vanilla'],
    disabled: [],
  },
};

const INITIAL_WORLD_PACKS: Record<string, MockWorldPack[]> = {
  'Survival 1_17': [...SURVIVAL_PACKS],
  'Creative Flat': [],
  'Locked World': [],
};

interface MockLevelBackup extends LevelBackupInfo {
  enabled: string[];
  disabled: string[];
}

const INITIAL_LEVEL_BACKUPS: MockLevelBackup[] = [
  {
    file_name: '20261025T101500.000000000-Survival 1_17-level.dat',
    world: 'Survival 1_17',
    bytes: 10258,
    display: '2026-10-25 10:15:00 UTC',
    enabled: [...INITIAL_WORLD_STACKS['Survival 1_17'].enabled],
    disabled: [...INITIAL_WORLD_STACKS['Survival 1_17'].disabled],
  },
];

let mockWorldStacks: Record<string, { enabled: string[]; disabled: string[] }> = JSON.parse(
  JSON.stringify(INITIAL_WORLD_STACKS),
);
let mockWorldPacks: Record<string, MockWorldPack[]> = JSON.parse(
  JSON.stringify(INITIAL_WORLD_PACKS),
);
let mockLevelBackups: MockLevelBackup[] = [...INITIAL_LEVEL_BACKUPS];
let mockLockedWorld: string | null = null;

/** Name the world whose session.lock the mock reports as held. */
export function setMockLockedWorld(world: string | null) {
  mockLockedWorld = world;
}

/** Reset the mock worlds between browser demos or tests. */
export function resetMockWorlds() {
  mockWorldStacks = JSON.parse(JSON.stringify(INITIAL_WORLD_STACKS));
  mockWorldPacks = JSON.parse(JSON.stringify(INITIAL_WORLD_PACKS));
  mockLevelBackups = [...INITIAL_LEVEL_BACKUPS];
  mockLockedWorld = null;
}

function mockWorldList(): World[] {
  const running =
    mockGameState === 'running'
      ? {
          kind: 'session_lock' as const,
          detail: '/home/demo/.minecraft/saves/Survival 1_17/session.lock is locked by pid 4242',
          lock_path: '/home/demo/.minecraft/saves/Survival 1_17/session.lock',
          pid: 4242,
          command: null,
        }
      : null;
  const lockedDetail = (world: string) => ({
    kind: 'session_lock' as const,
    detail: `/home/demo/.minecraft/saves/${world}/session.lock is locked by pid 4242`,
    lock_path: `/home/demo/.minecraft/saves/${world}/session.lock`,
    pid: 4242,
    command: null,
  });
  return [
    {
      dir_name: 'Survival 1_17',
      path: '/home/demo/.minecraft/saves/Survival 1_17',
      level_name: 'Survival 1.17',
      version: { name: '1.21.10', data_format: { major: 88, minor: 0 } },
      icon: { sha256: 'mock-survival', url: iconDataUrl('S1', '#3f7d5a') },
      running,
      world_locked: mockLockedWorld === 'Survival 1_17' ? lockedDetail('Survival 1_17') : null,
      stale_lock: false,
      detection_supported: true,
      issue: null,
    },
    {
      dir_name: 'Creative Flat',
      path: '/home/demo/.minecraft/saves/Creative Flat',
      level_name: 'Creative Flat',
      version: { name: '1.21.10', data_format: { major: 88, minor: 0 } },
      icon: null,
      running,
      world_locked: mockLockedWorld === 'Creative Flat' ? lockedDetail('Creative Flat') : null,
      stale_lock: mockGameState === 'stale-lock',
      detection_supported: true,
      issue: null,
    },
    {
      dir_name: 'Broken World',
      path: '/home/demo/.minecraft/saves/Broken World',
      level_name: 'Broken World',
      version: null,
      icon: null,
      running,
      world_locked: null,
      stale_lock: false,
      detection_supported: true,
      issue: {
        code: 'level_dat',
        message: 'level.dat is not valid gzip: invalid gzip header',
      },
    },
  ];
}

function dataEntry(value: string): DataStackEntry {
  return value.startsWith('file/')
    ? { kind: 'file', value, filename: value.slice('file/'.length) }
    : { kind: 'foreign', value };
}

function mockDataStack(world: string): DataStack {
  const state = mockWorldStacks[world] ?? { enabled: ['vanilla'], disabled: [] };
  return {
    enabled: state.enabled.map(dataEntry),
    disabled: state.disabled.map(dataEntry),
    running:
      mockGameState === 'running'
        ? {
            kind: 'session_lock' as const,
            detail: `/home/demo/.minecraft/saves/${world}/session.lock is locked by pid 4242`,
            lock_path: `/home/demo/.minecraft/saves/${world}/session.lock`,
            pid: 4242,
            command: null,
          }
        : null,
    world_locked:
      mockLockedWorld === world
        ? {
            kind: 'session_lock' as const,
            detail: `/home/demo/.minecraft/saves/${world}/session.lock is locked by pid 4242`,
            lock_path: `/home/demo/.minecraft/saves/${world}/session.lock`,
            pid: 4242,
            command: null,
          }
        : null,
    stale_locks: [],
    detection_supported: true,
  };
}

function mockDatapackListing(world: string): DatapackListing {
  const entries = mockWorldPacks[world];
  if (entries === undefined && mockWorldStacks[world] === undefined) {
    throw new IpcError('world', `world ${world} does not exist`);
  }
  const stacks = mockWorldStacks[world] ?? { enabled: ['vanilla'], disabled: [] };
  if (world === 'Broken World' || (entries === undefined && world !== 'Creative Flat' && world !== 'Locked World' && world !== 'Survival 1_17')) {
    throw new IpcError('level_dat', 'level.dat is not valid gzip: invalid gzip header');
  }
  const packs: Pack[] = (entries ?? []).map((item) => {
    const packRange = {
      min: item.minFormat === null ? null : { major: item.minFormat, minor: 0 },
      max: item.maxFormat === null ? null : { major: item.maxFormat, minor: 0 },
    };
    return {
      ...makePack(
        item.filename,
        item.kind,
        packRange,
        item.gameLabel,
        `A datapack for ${world}.`,
        iconDataUrl(item.filename.slice(0, 2).toUpperCase(), '#4a5f8a'),
      ),
      path: `/home/demo/.minecraft/saves/${world}/datapacks/${item.filename}`,
      id: `/home/demo/.minecraft/saves/${world}/datapacks/${item.filename}`,
    };
  });
  void stacks;
  return {
    packs: {
      packs,
      count: packs.length,
      bytes_read: packs.reduce((total, pack) => total + pack.bytes_read, 0),
      elapsed_ms: 2.1,
    },
    stack: mockDataStack(world),
  };
}

function stackStateOf(world: string): { enabled: string[]; disabled: string[] } {
  const state = mockWorldStacks[world];
  if (!state) throw new IpcError('world', `world ${world} does not exist`);
  if (world === 'Broken World') throw new IpcError('level_dat', 'level.dat is read-only');
  return state;
}

function mockAssertDatapackWritable(world: string) {
  if (world === 'Broken World') {
    throw new IpcError('level_dat', 'level.dat is read-only');
  }
  if (mockGameState === 'running' || mockLockedWorld === world) {
    throw new IpcError(
      'game_running',
      `Minecraft is running (/home/demo/.minecraft/saves/${world}/session.lock is locked by pid 4242); close it before Mine Utils writes level.dat`,
    );
  }
}

function mockSetDatapackEnabled(world: string, filename: string, enabled: boolean) {
  const state = stackStateOf(world);
  const value = `file/${filename}`;
  if (enabled) {
    state.disabled = state.disabled.filter((entry) => entry !== value);
    if (!state.enabled.includes(value)) state.enabled = [...state.enabled, value];
  } else {
    state.enabled = state.enabled.filter((entry) => entry !== value);
    state.disabled = state.disabled.filter((entry) => entry !== value);
    state.disabled = [...state.disabled, value];
  }
  snapshotLevelBackup(world);
}

function mockReorderDatapacks(world: string, order: string[], expected: DataSnapshot) {
  const state = stackStateOf(world);
  if (order.length !== state.enabled.length) {
    throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
  }
  for (let index = 0; index < state.enabled.length; index += 1) {
    if (!isFileEntry(state.enabled[index]) && state.enabled[index] !== order[index]) {
      throw new IpcError('level_dat', `the locked entry ${state.enabled[index]} cannot move`);
    }
  }
  const sorted = (values: string[]) => values.filter(isFileEntry).sort().join('\u0000');
  if (sorted(state.enabled) !== sorted(order)) {
    throw new IpcError('level_dat', 'the requested order is not a reorder of the current stack');
  }
  if (expected.enabled.join('\u0000') !== state.enabled.join('\u0000')) {
    throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
  }
  state.enabled = [...order];
  snapshotLevelBackup(world);
}

function snapshotLevelBackup(world: string) {
  const state = mockWorldStacks[world];
  if (!state) return;
  mockLevelBackups = [
    {
      file_name: `${Date.now()}-${world}-level.dat`,
      world,
      bytes: state.enabled.join(',').length,
      display: 'restored copy',
      enabled: [...state.enabled],
      disabled: [...state.disabled],
    },
    ...mockLevelBackups,
  ];
}

function mockImportDatapack(
  world: string,
  path: string,
  choice: ImportChoice,
): DatapackImportOutcome {
  const base = path.split('/').filter(Boolean).at(-1) ?? path;
  if (world === 'Broken World') {
    throw new IpcError('level_dat', 'level.dat is read-only');
  }
  if (mockWorldStacks[world] === undefined) {
    throw new IpcError('world', `world ${world} does not exist`);
  }
  const lower = base.toLowerCase();
  if (lower.includes('resource-only') || lower.includes('resource_only')) {
    throw new IpcError('level_dat', `${base} is a resource pack, not a datapack: it has no data root`);
  }
  const packs = mockWorldPacks[world] ?? [];
  const existing = packs.find((item) => item.filename === base);
  if (existing) {
    if (choice === 'skip') return { status: 'skipped' };
    if (choice !== 'replace') {
      return {
        status: 'duplicate',
        name: base,
        sha256: 'mock-duplicate',
        existing: `/home/demo/.minecraft/saves/${world}/datapacks/${base}`,
      };
    }
    return { status: 'imported', filename: base };
  }
  const taken = packs.some((item) => item.filename.toLowerCase() === lower);
  if (taken && choice !== 'replace') return { status: 'collision', name: base };
  mockWorldPacks[world] = [
    ...packs,
    { filename: base, kind: 'data', minFormat: 7, maxFormat: 7, gameLabel: '1.17 to 1.17.1' },
  ];
  if (mockGameState === 'running' || mockLockedWorld === world) {
    return {
      status: 'locked',
      filename: base,
      detail: `/home/demo/.minecraft/saves/${world}/session.lock is locked by pid 4242`,
    };
  }
  const state = stackStateOf(world);
  const value = `file/${base}`;
  if (!state.enabled.includes(value)) state.enabled = [...state.enabled, value];
  state.disabled = state.disabled.filter((entry) => entry !== value);
  snapshotLevelBackup(world);
  return { status: 'imported', filename: base };
}

function mockDeleteDatapack(world: string, path: string) {
  const filename = path.split('/').filter(Boolean).at(-1) ?? path;
  mockAssertDatapackWritable(world);
  const state = stackStateOf(world);
  const value = `file/${filename}`;
  state.enabled = state.enabled.filter((entry) => entry !== value);
  state.disabled = state.disabled.filter((entry) => entry !== value);
  mockWorldPacks[world] = (mockWorldPacks[world] ?? []).filter((item) => item.filename !== filename);
  snapshotLevelBackup(world);
}

function stackEntry(value: string): StackEntry {
  return value.startsWith('file/')
    ? { kind: 'file', value, filename: value.slice('file/'.length) }
    : { kind: 'foreign', value };
}

function mockResourceStack(): ResourceStack {
  const running =
    mockGameState === 'running'
      ? {
          kind: 'session_lock' as const,
          detail: '/home/demo/.minecraft/saves/world/session.lock is locked by pid 4242',
          lock_path: '/home/demo/.minecraft/saves/world/session.lock',
          pid: 4242,
          command: null,
        }
      : null;
  return {
    entries: mockStack.resource.map(stackEntry),
    incompatible: [...mockStack.incompatible],
    running,
    stale_locks:
      mockGameState === 'stale-lock'
        ? ['/home/demo/.minecraft/saves/world/session.lock']
        : [],
    detection_supported: true,
  };
}

function assertWritable() {
  if (mockGameState === 'running') {
    throw new IpcError(
      'game_running',
      'Minecraft is running (/home/demo/.minecraft/saves/world/session.lock is locked by pid 4242); close it before Mine Utils writes options.txt',
    );
  }
}

function isFileEntry(value: string): boolean {
  return value.startsWith('file/');
}

/** The mock enforces the same request rules as the core: foreign entries stay
 *  at their indices and the file multiset is preserved. */
function mockReorder(order: string[]) {
  const current = mockStack.resource;
  if (order.length !== current.length) {
    throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
  }
  for (let index = 0; index < current.length; index += 1) {
    if (!isFileEntry(current[index]) && current[index] !== order[index]) {
      throw new IpcError('resource_stack', `the locked entry ${current[index]} cannot move`);
    }
  }
  const sorted = (values: string[]) =>
    values.filter(isFileEntry).sort().join('\u0000');
  if (sorted(current) !== sorted(order)) {
    throw new IpcError('resource_stack', 'the requested order is not a reorder of the current stack');
  }
  mockStack = { ...mockStack, resource: [...order] };
}
