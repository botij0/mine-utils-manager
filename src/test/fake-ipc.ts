import type { IpcClient } from '../lib/ipc';
import type {
  BackupInfo,
  Config,
  DataStack,
  GameDirEntry,
  GameDirState,
  LevelBackupInfo,
  ModrinthInstallArgs,
  ModrinthInstallOutcome,
  ModrinthKind,
  ModrinthProject,
  ModrinthProvenanceEntry,
  ModrinthSearchHit,
  ModrinthSort,
  ModrinthTargets,
  ModrinthVersion,
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

function dataStack(): DataStack {
  return {
    enabled: ['vanilla'].map((value) => ({
      kind: 'foreign' as const,
      value,
    })),
    disabled: [],
    running: null,
    world_locked: null,
    stale_locks: [],
    detection_supported: true,
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
    async listWorlds() {
      return [];
    },
    async listDatapacks() {
      return {
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: dataStack(),
      };
    },
    async importDatapack() {
      return { status: 'skipped' as const };
    },
    async deleteDatapack() {
      return { removed_references: false, stack: dataStack() };
    },
    async removeDatapackReference() {
      return dataStack();
    },
    async setDatapackEnabled() {
      return dataStack();
    },
    async reorderDatapacks() {
      return dataStack();
    },
    async listLevelBackups(): Promise<LevelBackupInfo[]> {
      return [];
    },
    async listAllLevelBackups(): Promise<LevelBackupInfo[]> {
      return [];
    },
    async restoreLevelBackup() {
      return dataStack();
    },
    async searchModrinth(
      text: string,
      kind: ModrinthKind,
      gameVersion?: string | null,
      sort?: ModrinthSort,
    ): Promise<ModrinthSearchHit[]> {
      void gameVersion;
      void sort;
      if (text === '__offline__') {
        throw new IpcError('modrinth_offline', 'Modrinth is unreachable');
      }
      if (text === '__boom__') {
        throw new IpcError('modrinth_status', 'Modrinth request failed with status 500');
      }
      return fakeHits(kind);
    },
    async getModrinthProject(projectId: string): Promise<ModrinthProject> {
      return fakeProject(projectId);
    },
    async listModrinthVersions(projectId: string): Promise<ModrinthVersion[]> {
      return fakeVersions(projectId);
    },
    async modrinthTargets(): Promise<ModrinthTargets> {
      return {
        installed_versions: ['1.21', '1.21.1'],
        preferred_launcher: '1.21.1',
        world_version: '1.21.1',
      };
    },
    async installModrinthPack(args: ModrinthInstallArgs): Promise<ModrinthInstallOutcome> {
      if (args.filename.includes('mismatch')) {
        throw new IpcError(
          'download_hash_mismatch',
          'download failed: SHA-512 mismatch (expected 00aa, got ffbb)',
        );
      }
      if (args.kind === 'datapack') {
        return { kind: 'datapack', outcome: { status: 'imported', filename: args.filename } };
      }
      return {
        kind: 'resourcepack',
        outcome: {
          status: 'imported',
          pack: {
            id: args.filename,
            name: args.filename.replace(/\.zip$/i, ''),
            path: `/home/demo/.minecraft/resourcepacks/${args.filename}`,
            location: 'zip',
            kind: 'resource',
            description: 'Installed from Modrinth.',
            meta: {
              pack_format: null,
              min_format: null,
              max_format: null,
              supported_formats: null,
              description: 'Installed from Modrinth.',
            },
            format: null,
            format_range: null,
            game_label: null,
            icon: null,
            error: null,
            bytes_read: 0,
          },
        },
      };
    },
    async getModrinthProvenance(): Promise<ModrinthProvenanceEntry | null> {
      return null;
    },
  };
}

function fakeHits(kind: ModrinthKind): ModrinthSearchHit[] {
  if (kind === 'datapack') {
    return [
      {
        project_id: 'dp1',
        slug: 'pure-datapack',
        title: 'Pure Datapack',
        description: 'Only datapack files.',
        icon_url: null,
        downloads: 500,
        follows: 5,
        project_type: 'datapack',
        versions: ['1.21.1'],
        latest_version: 'd2',
      },
    ];
  }
  return [
    {
      project_id: '7q7JnQD0',
      slug: 'fresh-animations',
      title: 'Fresh Animations',
      description: 'Fresh animations.',
      icon_url: 'https://cdn.modrinth.com/icons/fresh.webp',
      downloads: 100,
      follows: 10,
      project_type: 'resourcepack',
      versions: ['1.21.1'],
      latest_version: 'v2',
    },
  ];
}

function fakeProject(projectId: string): ModrinthProject {
  const hit = fakeHits('resourcepack').find(
    (item) => item.project_id === projectId || item.slug === projectId,
  ) ?? {
    project_id: projectId,
    slug: projectId,
    title: projectId,
    description: '',
    icon_url: null,
    downloads: 0,
    follows: 0,
    project_type: 'resourcepack',
    versions: [] as string[],
    latest_version: null,
  };
  return {
    id: hit.project_id,
    slug: hit.slug,
    title: hit.title,
    description: hit.description,
    body: hit.description,
    icon_url: hit.icon_url,
    downloads: hit.downloads,
    follows: hit.follows,
    project_type: hit.project_type,
    game_versions: [...hit.versions],
    versions: fakeVersions(hit.project_id).map((version) => version.id),
  };
}

function fakeVersions(projectId: string): ModrinthVersion[] {
  if (projectId === 'mm1' || projectId === 'mismatch-pack') {
    return [
      {
        id: 'm1',
        project_id: 'mm1',
        name: 'Mismatch Pack 1.0',
        version_number: '1.0',
        game_versions: ['1.21.1'],
        loaders: ['minecraft'],
        files: [
          {
            url: 'https://cdn.modrinth.com/data/mm1/mismatch.zip',
            filename: 'mismatch.zip',
            size: 128,
            primary: true,
            hashes: { sha512: '00aa', sha1: '00' },
          },
        ],
      },
    ];
  }
  if (projectId === 'dp1' || projectId === 'pure-datapack') {
    return [
      {
        id: 'd2',
        project_id: 'dp1',
        name: 'Pure Datapack 2.0',
        version_number: '2.0',
        game_versions: ['1.21.1'],
        loaders: ['minecraft'],
        files: [
          {
            url: 'https://cdn.modrinth.com/data/dp1/pure-2.0.zip',
            filename: 'pure-2.0.zip',
            size: 256,
            primary: true,
            hashes: { sha512: 'dp', sha1: 'dp1' },
          },
        ],
      },
    ];
  }
  return [
    {
      id: 'v2',
      project_id: '7q7JnQD0',
      name: 'Fresh Animations 1.9',
      version_number: '1.9',
      game_versions: ['1.21.1'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/7q7/fresh-1.9.zip',
          filename: 'fresh-1.9.zip',
          size: 1024,
          primary: true,
          hashes: { sha512: 'aa', sha1: 'a' },
        },
      ],
    },
    {
      id: 'v1',
      project_id: '7q7JnQD0',
      name: 'Fresh Animations 1.8',
      version_number: '1.8',
      game_versions: ['1.20.1'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/7q7/fresh-1.8.zip',
          filename: 'fresh-1.8.zip',
          size: 900,
          primary: true,
          hashes: { sha512: 'bb', sha1: 'b' },
        },
      ],
    },
  ];
}
