import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createElement } from 'react';
import { beforeEach, describe, expect, it } from 'vitest';
import App from '../App';
import { ipc, isTauriRuntime, toAppError } from './ipc';
import { resetMockPacks, resetMockWorlds, setMockGameState, setMockLockedWorld, setMockOptionsFile, setMockWorldSeed, resetMockStack } from './ipc.mock';

const NO_ACTIVE_GAME_DIR = {
  code: 'no_active_game_dir',
  message: 'no active game dir; pick one in Settings first',
};

beforeEach(() => {
  window.localStorage.clear();
  window.location.hash = '';
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
  beforeEach(async () => {
    resetMockWorlds();
    setMockGameState('none');
    setMockLockedWorld(null);
    await ipc.setActiveGameDir('/home/demo/.minecraft');
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
  beforeEach(async () => {
    resetMockPacks();
    setMockGameState('none');
    await ipc.setActiveGameDir('/home/demo/.minecraft');
  });

  it('refuses local resource imports while Minecraft runs', async () => {
    setMockGameState('running');
    await expect(ipc.importResourcePack('/tmp/fresh-pack.zip')).rejects.toMatchObject({ code: 'game_running' });
    setMockGameState('none');
    expect(await ipc.importResourcePack('/tmp/fresh-pack.zip')).toMatchObject({ status: 'imported', pack: { name: 'fresh-pack' } });
  });

  it('refuses Modrinth resource installs while Minecraft runs', async () => {
    await ipc.setActiveGameDir('/home/demo/.minecraft');
    const args = { projectId: 'demo-pack-01', versionId: 'demo-pack-01-1.0', filename: 'demo-pack-01-1.0.zip', kind: 'resourcepack' } as const;
    setMockGameState('running');
    await expect(ipc.installModrinthPack(args)).rejects.toMatchObject({ code: 'game_running' });
    setMockGameState('none');
    expect(await ipc.installModrinthPack(args)).toMatchObject({ kind: 'resourcepack', outcome: { status: 'imported', pack: { name: 'demo-pack-01-1.0' } } });
  });
});

describe('mock backend alignment', () => {
  beforeEach(async () => {
    resetMockPacks();
    resetMockStack();
    resetMockWorlds();
    setMockGameState('none');
    await ipc.setActiveGameDir('/home/demo/.minecraft');
  });

  it('rejects a short resource order with resource_stack', async () => {
    const stack = await ipc.getResourceStack();
    const expected = { resource: stack.entries.map((entry) => entry.value), incompatible: stack.incompatible };
    await expect(ipc.reorderResourceStack([], expected)).rejects.toMatchObject({ code: 'resource_stack' });
  });

  it('refuses a last world outside the active game dir with the Rust guard message', async () => {
    await expect(ipc.setLastWorld('/home/demo/.var/app/com.mojang.Minecraft/data/minecraft/saves/Survival 1_17')).rejects.toMatchObject({
      code: 'world',
      message: 'world error: world is not in the active game dir',
    });
  });

  it('reports a missing world with its resolved path', async () => {
    await expect(ipc.setLastWorld('/home/demo/.minecraft/saves/Missing')).rejects.toMatchObject({
      code: 'world',
      message: 'world error: /home/demo/.minecraft/saves/Missing is not a world in this game dir',
    });
  });

  it('accepts a trailing slash on a world path', async () => {
    await ipc.setLastWorld('/home/demo/.minecraft/saves/Survival 1_17/');
    expect((await ipc.getConfig()).last_world).toBe('/home/demo/.minecraft/saves/Survival 1_17');
  });

  it('rejects reads without an active game directory', async () => {
    window.localStorage.clear();
    await expect(ipc.listResourcePacks()).rejects.toMatchObject({ code: 'no_active_game_dir' });
    await expect(ipc.getResourceStack()).rejects.toMatchObject({ code: 'no_active_game_dir' });
    await expect(ipc.listWorlds()).rejects.toMatchObject({ code: 'no_active_game_dir' });
  });

  it('reuses and re-enables a pack on a second Modrinth install, as Rust does', async () => {
    const args = { projectId: 'demo-pack-01', versionId: 'demo-pack-01-1.0', filename: 'demo-pack-01-1.0.zip', kind: 'resourcepack' } as const;
    expect(await ipc.installModrinthPack(args)).toMatchObject({ outcome: { status: 'imported' } });
    await ipc.setResourcePackEnabled('demo-pack-01-1.0.zip', false);
    expect(await ipc.installModrinthPack(args)).toMatchObject({ outcome: { status: 'imported' } });
    const entries = (await ipc.getResourceStack()).entries.map((entry) => entry.value);
    expect(entries.filter((value) => value === 'file/demo-pack-01-1.0.zip')).toHaveLength(1);
  });

  it('rebases valid orders and rejects invalid stale orders', async () => {
    const stack = await ipc.getResourceStack();
    const current = stack.entries.map((entry) => entry.value);
    const stale = { resource: ['old'], incompatible: [] };
    expect((await ipc.reorderResourceStack(current, stale)).entries.map((entry) => entry.value)).toEqual(current);
    await expect(ipc.reorderResourceStack([], stale)).rejects.toMatchObject({ code: 'stale_stack' });
    const data = await ipc.listDatapacks('Survival 1_17');
    const enabled = data.stack.enabled.map((entry) => entry.value);
    expect((await ipc.reorderDatapacks('Survival 1_17', enabled, { enabled: ['old'], disabled: [] })).enabled.map((entry) => entry.value)).toEqual(enabled);
    await expect(ipc.reorderDatapacks('Survival 1_17', [], { enabled: ['old'], disabled: [] })).rejects.toMatchObject({ code: 'stale_stack' });
  });

  it('guards deletes and rejects datapack installs without a world', async () => {
    setMockGameState('running');
    await expect(ipc.deleteResourcePack('/home/demo/.minecraft/resourcepacks/Re-covered.zip')).rejects.toMatchObject({ code: 'game_running' });
    setMockGameState('none');
    await expect(ipc.installModrinthPack({ projectId: 'demo', versionId: '1', filename: 'demo.zip', kind: 'datapack' })).rejects.toMatchObject({ code: 'modrinth_parse' });
  });

  it('refuses a level backup with a different readable world seed', async () => {
    setMockWorldSeed('Survival 1_17', 99);
    await expect(ipc.restoreLevelBackup('Survival 1_17', '20261025T101500.000000000-Survival 1_17-level.dat')).rejects.toMatchObject({ code: 'world_mismatch' });
    setMockWorldSeed('Survival 1_17', null);
    expect((await ipc.restoreLevelBackup('Survival 1_17', '20261025T101500.000000000-Survival 1_17-level.dat')).enabled[0].value).toBe('vanilla');
  });

});

describe('mock install preflight', () => {
  beforeEach(async () => {
    resetMockPacks();
    resetMockWorlds();
    setMockGameState('none');
    await ipc.setActiveGameDir('/home/demo/.minecraft');
  });

  it('checks the game guard before attempting a resource pack download', async () => {
    setMockGameState('running');
    await expect(ipc.installModrinthPack({ projectId: 'demo', versionId: '1', filename: 'offline.zip', kind: 'resourcepack' })).rejects.toMatchObject({ code: 'game_running' });
  });

  it('checks the target world before attempting a datapack download', async () => {
    await expect(ipc.installModrinthPack({ projectId: 'demo', versionId: '1', filename: 'offline.zip', kind: 'datapack' })).rejects.toMatchObject({ code: 'modrinth_parse' });
  });
});

it('normalizes unrecognized transport codes and preserves the raw code', () => {
  expect(toAppError({ code: 'plugin_failure', message: 'Plugin failed' })).toMatchObject({
    code: 'unknown', rawCode: 'plugin_failure', message: 'Plugin failed', name: 'IpcError',
  });
});

it('preserves raw transport codes through repeated normalization', () => {
  const mapped = toAppError({ code: 'plugin_failure', message: 'Plugin failed' });
  expect(toAppError(mapped)).toMatchObject({
    code: 'unknown', rawCode: 'plugin_failure', message: 'Plugin failed',
  });
});

it('preserves the options symlink refusal for display', () => {
  expect(toAppError({ code: 'options_symlink', message: '/game/options.txt is a symlink and cannot be written' })).toMatchObject({
    code: 'options_symlink', message: '/game/options.txt is a symlink and cannot be written',
  });
});

describe('mock parity with the Rust backend', () => {
  const datapackInstall = {
    projectId: 'demo',
    versionId: '1',
    filename: 'fresh-data.zip',
    kind: 'datapack' as const,
    world: 'Survival 1_17',
  };

  beforeEach(async () => {
    resetMockPacks();
    resetMockStack();
    resetMockWorlds();
    setMockGameState('none');
    setMockLockedWorld(null);
    setMockOptionsFile('present');
    await ipc.setActiveGameDir('/home/demo/.minecraft');
  });

  it('rejects mutating and datapack calls when no folder is selected', async () => {
    window.localStorage.clear();
    await expect(ipc.setResourcePackEnabled('Re-covered.zip', false)).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.importResourcePack('/tmp/fresh-pack.zip')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.deleteResourcePack('/home/demo/.minecraft/resourcepacks/Re-covered.zip')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.reorderResourceStack([], { resource: [], incompatible: [] })).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.restoreOptionsBackup('20261025T101500.000000000-options.txt')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.listDatapacks('Survival 1_17')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.importDatapack('Survival 1_17', '/tmp/fresh-pack.zip')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.deleteDatapack('Survival 1_17', '/tmp/fresh-pack.zip')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.removeDatapackReference('Survival 1_17', 'fresh-pack.zip')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.setDatapackEnabled('Survival 1_17', 'fresh-pack.zip', false)).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.reorderDatapacks('Survival 1_17', [], { enabled: [], disabled: [] })).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.listLevelBackups('Survival 1_17')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.listAllLevelBackups()).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.restoreLevelBackup('Survival 1_17', '20261025T101500.000000000-Survival 1_17-level.dat')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.restoreDatapackHolding('Survival 1_17', 'missing')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.restoreResourceHolding('missing')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.modrinthTargets()).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.setLastWorld('/home/demo/.minecraft/saves/Survival 1_17')).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
    await expect(ipc.installModrinthPack(datapackInstall)).rejects.toMatchObject(NO_ACTIVE_GAME_DIR);
  });

  it('refuses a resource-pack install before download when options.txt is missing', async () => {
    setMockOptionsFile('missing');
    await expect(ipc.installModrinthPack({
      projectId: 'demo',
      versionId: '1',
      filename: 'offline.zip',
      kind: 'resourcepack',
    })).rejects.toMatchObject({
      code: 'options_txt',
      message: 'options.txt is missing at /home/demo/.minecraft/options.txt; launch Minecraft once so it writes the file, then try again',
    });
  });

  it('returns world when a datapack import targets the broken world', async () => {
    await expect(ipc.importDatapack('Broken World', '/tmp/fresh-pack.zip')).rejects.toMatchObject({
      code: 'world',
      message: 'world error: world Broken World has no readable level.dat',
    });
  });

  it('reuses identical datapack bytes and returns imported', async () => {
    expect(await ipc.installModrinthPack(datapackInstall)).toMatchObject({
      kind: 'datapack',
      outcome: { status: 'imported', filename: 'fresh-data.zip' },
    });
    await ipc.setDatapackEnabled('Survival 1_17', 'fresh-data.zip', false);
    expect(await ipc.installModrinthPack(datapackInstall)).toMatchObject({
      kind: 'datapack',
      outcome: { status: 'imported', filename: 'fresh-data.zip' },
    });
    const listing = await ipc.listDatapacks('Survival 1_17');
    expect(listing.stack.enabled.map((entry) => entry.value)).toContain('file/fresh-data.zip');
    expect(listing.stack.disabled.map((entry) => entry.value)).not.toContain('file/fresh-data.zip');
  });

  it('leaves a locked datapack disabled when the same bytes are installed again', async () => {
    expect(await ipc.installModrinthPack(datapackInstall)).toMatchObject({
      kind: 'datapack',
      outcome: { status: 'imported', filename: 'fresh-data.zip' },
    });
    await ipc.setDatapackEnabled('Survival 1_17', 'fresh-data.zip', false);
    setMockLockedWorld('Survival 1_17');
    expect(await ipc.installModrinthPack(datapackInstall)).toMatchObject({
      kind: 'datapack',
      outcome: {
        status: 'locked',
        filename: 'fresh-data.zip',
        detail: '/home/demo/.minecraft/saves/Survival 1_17/session.lock is locked by pid 4242',
      },
    });
    const listing = await ipc.listDatapacks('Survival 1_17');
    expect(listing.stack.enabled.map((entry) => entry.value)).not.toContain('file/fresh-data.zip');
    expect(listing.stack.disabled.map((entry) => entry.value)).toContain('file/fresh-data.zip');
  });

  it('refuses options.txt writes when the file is a symlink', async () => {
    setMockOptionsFile('symlink');
    await expect(ipc.setResourcePackEnabled('Re-covered.zip', false)).rejects.toMatchObject({
      code: 'options_symlink',
      message: '/home/demo/.minecraft/options.txt is a symlink and cannot be written',
    });
    await expect(ipc.restoreOptionsBackup('20261025T101500.000000000-options.txt')).rejects.toMatchObject({
      code: 'options_symlink',
      message: '/home/demo/.minecraft/options.txt is a symlink and cannot be written',
    });
    const entries = (await ipc.getResourceStack()).entries.map((entry) => entry.value);
    expect(entries).toContain('file/Re-covered.zip');
  });

  it('does not list level.dat backups in Settings when no folder is selected', async () => {
    window.localStorage.clear();
    window.location.hash = '#settings';
    const user = userEvent.setup();
    render(createElement(App));
    await user.click(await screen.findByRole('button', { name: 'Skip for now' }));
    await waitFor(() => {
      expect(screen.queryByText('20261025T101500.000000000-Survival 1_17-level.dat')).not.toBeInTheDocument();
      expect(screen.getByRole('alert')).toHaveTextContent('no active game dir; pick one in Settings first');
    });
  });
});
