import { beforeEach, describe, expect, it } from 'vitest';
import { ipc, isTauriRuntime, toAppError } from './ipc';
import { resetMockPacks, resetMockWorlds, setMockGameState, setMockLockedWorld } from './ipc.mock';

beforeEach(() => {
  window.localStorage.clear();
});

describe('transport switch', () => {
  it('serves the mock dirs when the tauri runtime is absent', async () => {
    expect(isTauriRuntime(window)).toBe(false);
    const dirs = await ipc.listGameDirs();
    expect(dirs.map((dir) => dir.path)).toEqual([
      '/home/demo/.minecraft',
      '/home/demo/.var/app/com.mojang.Minecraft/data/minecraft',
      '/home/demo/.local/share/PrismLauncher/instances/1.21/.minecraft',
    ]);
    expect(dirs[0].source).toBe('std');
    expect(dirs[0].stored).toBe(false);
  });

  it('detects the tauri runtime from window internals', () => {
    const withTauri = { __TAURI_INTERNALS__: {} } as unknown as Window;
    const withoutTauri = {} as Window;
    expect(isTauriRuntime(withTauri)).toBe(true);
    expect(isTauriRuntime(withoutTauri)).toBe(false);
  });
});

describe('mock client behavior', () => {
  it('rejects an absolute path without a game-dir marker', async () => {
    await expect(ipc.addGameDir('/tmp/not-a-game-dir')).rejects.toMatchObject({
      code: 'invalid_game_dir',
    });
  });

  it('keeps added dirs when the theme changes', async () => {
    const added = await ipc.addGameDir('/tmp/saves/one');
    expect(added.config.game_dirs.map((dir) => dir.path)).toContain(
      '/tmp/saves/one',
    );

    const config = await ipc.setTheme('dark');

    expect(config.theme).toBe('dark');
    expect(config.game_dirs.map((dir) => dir.path)).toContain('/tmp/saves/one');
  });

  it('returns a null startup warning', async () => {
    expect(await ipc.getStartupWarning()).toBeNull();
  });

  it('pages the demo catalog for Load more without the network', async () => {
    const first = await ipc.searchModrinth('demo', 'resourcepack');
    expect(first.hits).toHaveLength(20);
    expect(first.has_more).toBe(true);
    expect(first.hits[0].title).toBe('Demo Pack 01');
    expect(first.hits[19].title).toBe('Demo Pack 20');

    const next = await ipc.searchModrinth('demo', 'resourcepack', null, 'relevance', 20);
    expect(next.hits.map((hit) => hit.title)).toEqual([
      'Demo Pack 21',
      'Demo Pack 22',
      'Demo Pack 23',
      'Demo Pack 24',
      'Demo Pack 25',
    ]);
    expect(next.has_more).toBe(false);

    const ids = [...first.hits, ...next.hits].map((hit) => hit.project_id);
    expect(new Set(ids).size).toBe(25);
  });

  it('pages the datapack demo catalog and keeps it out of other queries', async () => {
    const datapacks = await ipc.searchModrinth('demo', 'datapack');
    expect(datapacks.hits).toHaveLength(20);
    expect(datapacks.has_more).toBe(true);
    expect(datapacks.hits[0].title).toBe('Demo Datapack 01');

    const packs = await ipc.searchModrinth('pack', 'resourcepack');
    expect(packs.hits.some((hit) => hit.title.startsWith('Demo Pack'))).toBe(false);
  });

  it('sorts the mock catalog by the requested order', async () => {
    const page = await ipc.searchModrinth('demo', 'resourcepack', null, 'downloads');
    expect(page.hits[0].downloads).toBeGreaterThanOrEqual(page.hits[1].downloads);
  });

  it('opens detail and versions for a demo catalog hit', async () => {
    const project = await ipc.getModrinthProject('demo-pack-01');
    const versions = await ipc.listModrinthVersions('demo-pack-01');

    expect(project.title).toBe('Demo Pack 01');
    expect(project.versions).toEqual(['demo-pack-01-1.0']);
    expect(versions.map((version) => version.files[0]?.filename)).toEqual([
      'demo-pack-01-1.0.zip',
    ]);
  });
});

describe('typed error mapping', () => {
  it('keeps the code and message of a wire error', () => {
    const mapped = toAppError({
      code: 'invalid_game_dir',
      message: 'invalid game dir /bad: not a game dir',
    });
    expect(mapped.code).toBe('invalid_game_dir');
    expect(mapped.message).toBe('invalid game dir /bad: not a game dir');
    expect(mapped).toBeInstanceOf(Error);
  });

  it('maps a thrown string to the unknown code', () => {
    const mapped = toAppError('boom');
    expect(mapped.code).toBe('unknown');
    expect(mapped.message).toBe('boom');
  });

  it('maps a thrown error to the unknown code', () => {
    const mapped = toAppError(new Error('nope'));
    expect(mapped.code).toBe('unknown');
    expect(mapped.message).toBe('nope');
  });
});

describe('mock world import guards', () => {
  beforeEach(() => {
    resetMockWorlds();
    setMockGameState('none');
    setMockLockedWorld(null);
  });

  it('copies but disables a new pack when the game runs and the world is locked', async () => {
    setMockGameState('running');
    setMockLockedWorld('Survival 1_17');
    const outcome = await ipc.importDatapack('Survival 1_17', '/tmp/fresh-pack.zip', 'default');
    expect(outcome).toMatchObject({ status: 'locked', filename: 'fresh-pack.zip' });
  });

  it('refuses any import while the game runs and the world is not locked', async () => {
    setMockGameState('running');
    await expect(ipc.importDatapack('Survival 1_17', '/tmp/fresh-pack.zip', 'default')).rejects.toMatchObject({
      code: 'game_running',
    });
  });

  it('refuses a locked Replace before anything is trashed', async () => {
    setMockLockedWorld('Survival 1_17');
    const existing = 'armored elytra v1.0.4 (MC 1.17.x).zip';
    await expect(
      ipc.importDatapack('Survival 1_17', `/tmp/${existing}`, 'replace'),
    ).rejects.toMatchObject({ code: 'level_dat' });
    const listing = await ipc.listDatapacks('Survival 1_17');
    expect(listing.packs.packs.map((pack) => pack.path)).toContain(
      `/home/demo/.minecraft/saves/Survival 1_17/datapacks/${existing}`,
    );
  });
});

describe('mock resource import guards', () => {
  beforeEach(() => {
    resetMockPacks();
    setMockGameState('none');
  });

  it('refuses local resource imports while Minecraft runs', async () => {
    setMockGameState('running');
    await expect(ipc.importResourcePack('/tmp/fresh-pack.zip')).rejects.toMatchObject({ code: 'game_running' });
    setMockGameState('none');
    expect(await ipc.importResourcePack('/tmp/fresh-pack.zip')).toMatchObject({ status: 'imported', pack: { name: 'fresh-pack' } });
  });

  it('refuses Modrinth resource installs while Minecraft runs', async () => {
    const args = { projectId: 'demo-pack-01', versionId: 'demo-pack-01-1.0', filename: 'demo-pack-01-1.0.zip', kind: 'resourcepack' } as const;
    setMockGameState('running');
    await expect(ipc.installModrinthPack(args)).rejects.toMatchObject({ code: 'game_running' });
    setMockGameState('none');
    expect(await ipc.installModrinthPack(args)).toMatchObject({ kind: 'resourcepack', outcome: { status: 'imported', pack: { name: 'demo-pack-01-1.0' } } });
  });
});
