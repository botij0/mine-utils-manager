import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { WorldsClient } from '../../lib/ipc';
import type { DataStack, Pack, World } from '../../lib/types';
import { compatibilityWarning, isCompatible, useWorlds } from './useWorlds';

function world(overrides: Partial<World> = {}): World {
  return {
    dir_name: 'Survival 1_17',
    path: '/home/demo/.minecraft/saves/Survival 1_17',
    level_name: 'Survival 1.17',
    version: { name: '1.21.10', data_format: { major: 88, minor: 0 } },
    icon: null,
    running: null,
    world_locked: null,
    stale_lock: false,
    detection_supported: true,
    issue: null,
    ...overrides,
  };
}

function stack(overrides: Partial<DataStack> = {}): DataStack {
  return {
    enabled: [
      { kind: 'foreign', value: 'vanilla' },
      { kind: 'file', value: 'file/a.zip', filename: 'a.zip' },
      { kind: 'foreign', value: 'fabric' },
    ],
    disabled: [{ kind: 'file', value: 'file/b.zip', filename: 'b.zip' }],
    running: null,
    world_locked: null,
    stale_locks: [],
    detection_supported: true,
    ...overrides,
  };
}

function pack(name: string, min: number | null = 7, max: number | null = 7): Pack {
  return {
    id: `/saves/Survival 1_17/datapacks/${name}`,
    name,
    path: `/saves/Survival 1_17/datapacks/${name}`,
    location: 'zip',
    kind: 'data',
    description: 'A datapack',
    meta: {
      pack_format: null,
      min_format: min === null ? null : { major: min, minor: 0 },
      max_format: max === null ? null : { major: max, minor: 0 },
      supported_formats: null,
      description: 'A datapack',
    },
    format: min === null ? null : { major: min, minor: 0 },
    format_range: {
      min: min === null ? null : { major: min, minor: 0 },
      max: max === null ? null : { major: max, minor: 0 },
    },
    game_label: '1.17 to 1.17.1',
    icon: null,
    error: null,
    bytes_read: 128,
  };
}

function client(overrides: Partial<WorldsClient> = {}): WorldsClient {
  return {
    listWorlds: async () => [world()],
    listDatapacks: async () => ({
      packs: { packs: [pack('a.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
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

describe('useWorlds', () => {
  it('selects the first world and loads its datapacks', async () => {
    const stable = client();
    const { result } = renderHook(() => useWorlds(stable));
    await act(async () => {});
    expect(result.current.worlds).toHaveLength(1);
    expect(result.current.selected?.dir_name).toBe('Survival 1_17');
    expect(result.current.packs).toHaveLength(1);
    expect(result.current.stack?.enabled).toHaveLength(3);
  });

  it('shows compatibility chips for 1.17 packs on a 1.21.10 world', async () => {
    const stable = client();
    const { result } = renderHook(() => useWorlds(stable));
    await act(async () => {});
    const format = result.current.selected?.version?.data_format ?? null;
    expect(
      compatibilityWarning(pack('a.zip'), format, 'Survival 1.17'),
    ).toMatch(/1\.17/);
    expect(isCompatible({ min: null, max: null }, format)).toBe(true);
    expect(isCompatible({ min: { major: 88, minor: 0 }, max: null }, format)).toBe(true);
  });

  it('reports a read-only error for a corrupt level.dat', async () => {
    const broken = world({ issue: { code: 'level_dat', message: 'corrupt' }, version: null });
    const failing: WorldsClient = {
      ...client(),
      listWorlds: async () => [broken],
      listDatapacks: async () => {
        throw new Error('level.dat is not valid gzip');
      },
    };
    const { result } = renderHook(() => useWorlds(failing));
    await act(async () => {});
    expect(result.current.selected?.issue?.code).toBe('level_dat');
    expect(result.current.stackError).not.toBeNull();
  });

  it('surfaces the locked banner state', async () => {
    const locked = stack({
      world_locked: {
        kind: 'session_lock',
        detail: 'locked by pid 1',
        lock_path: 'session.lock',
        pid: 1,
        command: null,
      },
    });
    const lockedClient = { ...client(), listDatapacks: async () => ({
      packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
      stack: locked,
    }) };
    const hook = renderHook(() => useWorlds(lockedClient));
    await act(async () => {});
    expect(hook.result.current.stack?.world_locked).not.toBeNull();
  });

  it('round-trips toggle, reorder, import, delete, and restore', async () => {
    const setEnabled = vi.fn(async () => stack());
    const reorder = vi.fn(async () => stack());
    const importPack = vi.fn(async () => ({ status: 'imported' as const, filename: 'c.zip' }));
    const remove = vi.fn(async () => ({ removed_references: true, stack: stack() }));
    const restore = vi.fn(async () => stack());
    const roundTrip = client({
      setDatapackEnabled: setEnabled,
      reorderDatapacks: reorder,
      importDatapack: importPack,
      deleteDatapack: remove,
      restoreLevelBackup: restore,
    });
    const { result } = renderHook(() => useWorlds(roundTrip));
    await act(async () => {});
    await act(async () => {
      await result.current.toggleDatapack('a.zip', false);
    });
    expect(setEnabled).toHaveBeenCalledWith('Survival 1_17', 'a.zip', false);
    await act(async () => {
      await result.current.reorderDatapacks(['vanilla', 'file/a.zip', 'fabric']);
    });
    expect(reorder).toHaveBeenCalled();
    await act(async () => {
      await result.current.importDatapack('Survival 1_17', '/tmp/c.zip');
    });
    expect(importPack).toHaveBeenCalled();
    await act(async () => {
      await result.current.deleteDatapack('/saves/a.zip');
    });
    expect(remove).toHaveBeenCalled();
  });

  it('ignores a toggle response after the selection changes', async () => {
    let resolveToggle!: (value: DataStack) => void;
    const pending = new Promise<DataStack>((resolve) => {
      resolveToggle = resolve;
    });
    const stale = stack({ enabled: [] });
    const second = world({ dir_name: 'Creative Flat', path: '/saves/Creative Flat' });
    const twoWorlds = client({
      listWorlds: async () => [world(), second],
      setDatapackEnabled: () => pending,
    });
    const { result } = renderHook(() => useWorlds(twoWorlds));
    await act(async () => {});
    const flight = result.current.toggleDatapack('a.zip', false);
    await act(async () => {
      result.current.select('Creative Flat');
    });
    await act(async () => {
      resolveToggle(stale);
      await flight;
    });
    await act(async () => {});
    // The stale response for Survival 1_17 never touches the new selection.
    expect(result.current.selected?.dir_name).toBe('Creative Flat');
    expect(result.current.stack).not.toBe(stale);
    expect(result.current.actionError).toBeNull();
  });

  it('drops the old rows and releases busy when the selection changes mid-listing', async () => {
    const second = world({ dir_name: 'Creative Flat', path: '/saves/Creative Flat' });
    let resolveCreative!: (value: { packs: { packs: Pack[]; count: number; bytes_read: number; elapsed_ms: number }; stack: DataStack }) => void;
    const creativeListing = new Promise<{
      packs: { packs: Pack[]; count: number; bytes_read: number; elapsed_ms: number };
      stack: DataStack;
    }>((resolve) => {
      resolveCreative = resolve;
    });
    const switching = client({
      listWorlds: async () => [world(), second],
      listDatapacks: async (name: string) => {
        if (name === 'Creative Flat') return creativeListing;
        return {
          packs: { packs: [pack('a.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
          stack: stack(),
        };
      },
    });
    const { result } = renderHook(() => useWorlds(switching));
    await act(async () => {});
    expect(result.current.packs).toHaveLength(1);
    // Switch while the new world's listing is still in flight.
    act(() => {
      result.current.select('Creative Flat');
    });
    // The old world's cards are gone synchronously: the grid shows the new
    // world's loading state, so no stale filename is actionable.
    expect(result.current.packs).toHaveLength(0);
    expect(result.current.stack).toBeNull();
    expect(result.current.listingWorld).toBeNull();
    expect(result.current.packsLoading).toBe(true);
    expect(result.current.busy).toBe(false);
    await act(async () => {
      resolveCreative({
        packs: { packs: [pack('c.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
        stack: stack({
          enabled: [{ kind: 'foreign', value: 'vanilla' }],
          disabled: [{ kind: 'file', value: 'file/c.zip', filename: 'c.zip' }],
        }),
      });
      await creativeListing;
    });
    await act(async () => {});
    expect(result.current.packs.map((entry) => entry.name)).toEqual(['c.zip']);
    expect(result.current.listingWorld).toBe('Creative Flat');
    expect(result.current.busy).toBe(false);
  });

  it('ignores a late listing for the previous world', async () => {
    const second = world({ dir_name: 'Creative Flat', path: '/saves/Creative Flat' });
    let resolveSurvival!: (value: { packs: { packs: Pack[]; count: number; bytes_read: number; elapsed_ms: number }; stack: DataStack }) => void;
    const survivalListing = new Promise<{
      packs: { packs: Pack[]; count: number; bytes_read: number; elapsed_ms: number };
      stack: DataStack;
    }>((resolve) => {
      resolveSurvival = resolve;
    });
    const creativeStack = stack({
      enabled: [{ kind: 'foreign', value: 'vanilla' }],
      disabled: [],
    });
    const racing = client({
      listWorlds: async () => [world(), second],
      listDatapacks: async (name: string) => {
        if (name === 'Survival 1_17') return survivalListing;
        return {
          packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
          stack: creativeStack,
        };
      },
    });
    const { result } = renderHook(() => useWorlds(racing));
    await act(async () => {});
    // The first world's listing is still pending; switch before it lands.
    expect(result.current.packs).toHaveLength(0);
    act(() => {
      result.current.select('Creative Flat');
    });
    await act(async () => {
      resolveSurvival({
        packs: { packs: [pack('a.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
        stack: stack(),
      });
      await survivalListing;
    });
    await act(async () => {});
    // The late Survival listing never repopulates the new selection.
    expect(result.current.packs).toHaveLength(0);
    expect(result.current.stack).toBe(creativeStack);
    expect(result.current.listingWorld).toBe('Creative Flat');
  });

  it('releases busy when the selection changes mid-mutation', async () => {
    let resolveToggle!: (value: DataStack) => void;
    const pending = new Promise<DataStack>((resolve) => {
      resolveToggle = resolve;
    });
    const second = world({ dir_name: 'Creative Flat', path: '/saves/Creative Flat' });
    const setEnabled = vi.fn(() => pending);
    const twoWorlds = client({
      listWorlds: async () => [world(), second],
      setDatapackEnabled: setEnabled,
    });
    const { result } = renderHook(() => useWorlds(twoWorlds));
    await act(async () => {});
    let flight!: Promise<void>;
    act(() => {
      flight = result.current.toggleDatapack('a.zip', false);
    });
    expect(result.current.busy).toBe(true);
    act(() => {
      result.current.select('Creative Flat');
    });
    // The new world is never stranded: select releases busy, and the guarded
    // finally skips the stale request instead of clearing a newer one.
    expect(result.current.busy).toBe(false);
    await act(async () => {
      resolveToggle(stack());
      await flight;
    });
    await act(async () => {});
    expect(result.current.busy).toBe(false);
    expect(setEnabled).toHaveBeenCalledTimes(1);
    expect(setEnabled).toHaveBeenCalledWith('Survival 1_17', 'a.zip', false);
  });
});
