import type { IpcClient, WorldsClient } from '../lib/ipc';
import type {
  BackupInfo,
  Config,
  DataStack,
  GameDirEntry,
  GameGuard,
  GameDirState,
  GameRunning,
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
  ResourceStack,
  Theme,
} from '../lib/types';
import { MOCK_MODRINTH_PAGE_SIZE, defaultConfig, IpcError } from '../lib/types';
import {
  assertDownloadHash,
  datapack,
  fakeHits,
  fakeProject,
  fakeVersions,
  gameDir,
  modrinthTargets,
  pack,
  stack,
  stackEntry,
  withActiveGameDir,
  withGameDirAdded,
  withGameDirRemoved,
  world,
} from './fake-packs';

export interface FakeClient extends IpcClient {
  readonly config: Config;
  readonly entries: GameDirEntry[];
  readonly stack: ResourceStack;
  readonly packs: Pack[];
  readonly choices: ImportChoice[];
  readonly imports: string[];
  readonly toggles: Array<{ filename: string; enabled: boolean }>;
  readonly reorders: string[][];
  readonly restores: string[];
  readonly levelRestores: string[];
  restoredStack: ResourceStack | null;
  getGameGuard(): Promise<GameGuard>;
}

export interface FakeClientOptions {
  gameDirs?: GameDirEntry[];
  packs?: Pack[];
  resource?: string[];
  running?: GameRunning | null;
  staleLocks?: string[];
  backups?: BackupInfo[];
  levelBackups?: LevelBackupInfo[];
  backupsError?: IpcError | null;
  restoredStack?: ResourceStack | null;
}

export { gameDir, pack, datapack, world, stack } from './fake-packs';

function stripZip(name: string): string {
  return name.replace(/\.zip$/i, '');
}

function dataStack(): DataStack {
  return stack({
    enabled: [{ kind: 'foreign', value: 'vanilla' }],
    disabled: [],
  });
}

/**
 * The in-memory client for component tests. It is synchronous and records
 * writes; `ipc.mock` stays the browser demo with localStorage and latency.
 */
export function makeFakeClient(
  initial: GameDirEntry[] | FakeClientOptions = [],
): FakeClient {
  const options = Array.isArray(initial) ? { gameDirs: initial } : initial;
  let entries = [...(options.gameDirs ?? [])];
  let config: Config = {
    ...defaultConfig(),
    game_dirs: (options.gameDirs ?? [])
      .filter((item) => item.stored)
      .map((item) => ({ path: item.path, source: item.source })),
  };
  let packs = [...(options.packs ?? [])];
  let resource = [...(options.resource ?? ['vanilla', 'fabric'])];
  let restoredStack = options.restoredStack ?? null;
  const running = options.running ?? null;
  const staleLocks = [...(options.staleLocks ?? [])];
  const backups = [...(options.backups ?? [])];
  const levelBackups = [...(options.levelBackups ?? [])];
  const backupsError = options.backupsError ?? null;
  const choices: ImportChoice[] = [];
  const imports: string[] = [];
  const toggles: Array<{ filename: string; enabled: boolean }> = [];
  const reorders: string[][] = [];
  const restores: string[] = [];
  const levelRestores: string[] = [];

  const state = (): GameDirState => ({ config, entries });
  const currentStack = (): ResourceStack => ({
    entries: resource.map(stackEntry),
    incompatible: [],
    running,
    stale_locks: [...staleLocks],
    detection_supported: true,
  });

  return {
    get config() {
      return config;
    },
    get entries() {
      return entries;
    },
    get stack() {
      return currentStack();
    },
    get packs() {
      return packs;
    },
    get choices() {
      return choices;
    },
    get imports() {
      return imports;
    },
    get toggles() {
      return toggles;
    },
    get reorders() {
      return reorders;
    },
    get restores() {
      return restores;
    },
    get levelRestores() {
      return levelRestores;
    },
    get restoredStack() {
      return restoredStack;
    },
    set restoredStack(value) {
      restoredStack = value;
    },
    async listGameDirs() {
      return entries;
    },
    async addGameDir(path: string) {
      config = withGameDirAdded(config, path);
      if (!entries.some((item) => item.path === path)) {
        entries = [...entries, gameDir(path, true, true)];
      }
      return state();
    },
    async removeGameDir(path: string) {
      config = withGameDirRemoved(config, path);
      entries = entries.filter((item) => item.path !== path);
      return state();
    },
    async setActiveGameDir(path: string) {
      config = withActiveGameDir(config, entries, path);
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
      return {
        packs: packs.map((item) => ({ ...item })),
        count: packs.length,
        bytes_read: packs.reduce((total, item) => total + item.bytes_read, 0),
        elapsed_ms: 4,
      };
    },
    async importResourcePack(
      path: string,
      choice: ImportChoice = 'default',
    ): Promise<ImportOutcome> {
      imports.push(path);
      choices.push(choice);
      const base = path.split('/').filter(Boolean).at(-1) ?? path;
      const lower = base.toLowerCase();
      if (lower.includes('broken') || lower.includes('corrupt')) {
        throw new IpcError(
          'pack_broken_archive',
          `${base} is not a readable pack: invalid Zip archive`,
        );
      }
      if (lower.includes('duplicate')) {
        if (choice === 'skip') return { status: 'skipped' };
        if (choice !== 'replace') {
          return {
            status: 'duplicate',
            name: base,
            sha256: 'abc123',
            existing: packs[0]?.path ?? '/packs/existing.zip',
          };
        }
      }
      const imported = pack(stripZip(base));
      packs = [...packs, imported];
      return { status: 'imported', pack: imported };
    },
    async deleteResourcePack(path: string) {
      if (!packs.some((item) => item.path === path)) {
        throw new IpcError('pack_not_found', `pack ${path} does not exist`);
      }
      packs = packs.filter((item) => item.path !== path);
    },
    async getResourceStack() {
      return currentStack();
    },
    async getGameGuard(): Promise<GameGuard> {
      if (!config.active_game_dir) {
        throw new IpcError('no_active_game_dir', 'no active game dir; pick one in Settings first');
      }
      return { running, stale_locks: [...staleLocks], detection_supported: true };
    },
    async setResourcePackEnabled(filename: string, enabled: boolean) {
      toggles.push({ filename, enabled });
      const value = `file/${filename}`;
      if (enabled) {
        if (!resource.includes(value)) resource = [...resource, value];
      } else {
        resource = resource.filter((item) => item !== value);
      }
      return currentStack();
    },
    async reorderResourceStack(order: string[]) {
      // The fake records the order the UI sends. The demo mock and core
      // enforce the locked foreign entries; this fake does not.
      reorders.push(order);
      resource = [...order];
      return currentStack();
    },
    async listOptionsBackups() {
      if (backupsError) throw backupsError;
      return backups;
    },
    async restoreOptionsBackup(backup: string) {
      restores.push(backup);
      return restoredStack ?? currentStack();
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
    async listLevelBackups(worldName: string): Promise<LevelBackupInfo[]> {
      return levelBackups.filter((entry) => entry.world === worldName);
    },
    async listAllLevelBackups(): Promise<LevelBackupInfo[]> {
      return [...levelBackups];
    },
    async restoreLevelBackup(worldName: string, backup: string) {
      levelRestores.push(`${worldName}/${backup}`);
      return stack({ enabled: [], disabled: [] });
    },
    async searchModrinth(
      text: string,
      kind: ModrinthKind,
      gameVersion?: string | null,
      sort?: ModrinthSort,
      offset = 0,
    ): Promise<ModrinthSearchPage> {
      void gameVersion;
      void sort;
      // Exact sentinels leave a real query containing "offline" searchable;
      // the demo mock matches the substring so it can be poked by typing.
      if (text === '__offline__') {
        throw new IpcError('modrinth_offline', 'Modrinth is unreachable');
      }
      if (text === '__boom__') {
        throw new IpcError('modrinth_status', 'Modrinth request failed with status 500');
      }
      const hits = fakeHits(kind);
      const page = hits.slice(offset, offset + MOCK_MODRINTH_PAGE_SIZE);
      return { hits: page, has_more: offset + page.length < hits.length };
    },
    async getModrinthProject(projectId: string): Promise<ModrinthProject> {
      return fakeProject(projectId);
    },
    async listModrinthVersions(projectId: string): Promise<ModrinthVersion[]> {
      return fakeVersions(projectId);
    },
    async modrinthTargets(): Promise<ModrinthTargets> {
      return modrinthTargets();
    },
    async installModrinthPack(args: ModrinthInstallArgs): Promise<ModrinthInstallOutcome> {
      assertDownloadHash(args.filename);
      if (args.kind === 'datapack') {
        return { kind: 'datapack', outcome: { status: 'imported', filename: args.filename } };
      }
      const installed = pack(args.filename, {
        id: `/home/demo/.minecraft/resourcepacks/${args.filename}`,
        name: stripZip(args.filename),
        path: `/home/demo/.minecraft/resourcepacks/${args.filename}`,
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
        bytes_read: 0,
      });
      return {
        kind: 'resourcepack',
        outcome: { status: 'imported', pack: installed },
      };
    },
  };
}

export function worldsClient(
  overrides: Partial<WorldsClient> = {},
): WorldsClient {
  return {
    listWorlds: async () => [world()],
    listDatapacks: async () => ({
      packs: { packs: [datapack('a.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
      stack: stack(),
    }),
    importDatapack: async () => ({ status: 'skipped' }),
    deleteDatapack: async () => ({ removed_references: true, stack: stack() }),
    removeDatapackReference: async () => stack(),
    setDatapackEnabled: async () => stack(),
    reorderDatapacks: async () => stack(),
    listLevelBackups: async () => [],
    listAllLevelBackups: async () => [],
    restoreLevelBackup: async () => stack(),
    ...overrides,
  };
}
