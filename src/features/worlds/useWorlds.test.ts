import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { WorldsClient } from '../../lib/ipc';
import { defaultConfig } from '../../lib/types';
import type { DatapackListing, DataStack } from '../../lib/types';
import { worldsClient } from '../../test/fake-ipc';
import { datapack, datapackListing, stack, world } from '../../test/fake-packs';
import { checkCompatibility, compatibilityWarning, useWorlds } from './useWorlds';

describe('useWorlds', () => {
  it('selects the first world and loads its datapacks', async () => {
    const stable = worldsClient();
    const { result } = renderHook(() => useWorlds(stable));
    await act(async () => {});
    expect(result.current.worlds).toHaveLength(1);
    expect(result.current.selected?.dir_name).toBe('Survival 1_17');
    expect(result.current.packs).toHaveLength(1);
    expect(result.current.stack?.enabled).toHaveLength(3);
  });

  it('restores the remembered world and falls back when it is missing', async () => {
    const second = world({ dir_name: 'Creative Flat', path: '/saves/Creative Flat' });
    let remembered: string | null = null;
    let available = [world(), second];
    const client = worldsClient({
      listWorlds: async () => available,
      getConfig: async () => ({ ...defaultConfig(), last_world: remembered }),
      setLastWorld: async (path: string) => { remembered = path; },
    });
    const first = renderHook(() => useWorlds(client));
    await act(async () => {});
    expect(first.result.current.selected?.dir_name).toBe('Survival 1_17');
    await act(async () => { first.result.current.select('Creative Flat'); });
    first.unmount();
    const restored = renderHook(() => useWorlds(client));
    await act(async () => {});
    expect(restored.result.current.selected?.dir_name).toBe('Creative Flat');
    restored.unmount();
    available = [world()];
    const next = renderHook(() => useWorlds(client));
    await act(async () => {});
    expect(next.result.current.selected?.dir_name).toBe('Survival 1_17');
  });

  it('shows compatibility chips for 1.17 packs on a 1.21.10 world', async () => {
    const stable = worldsClient();
    const { result } = renderHook(() => useWorlds(stable));
    await act(async () => {});
    const format = result.current.selected?.version?.data_format ?? null;
    expect(
      compatibilityWarning(datapack('a.zip'), format, 'Survival 1.17'),
    ).toMatch(/1\.17/);
    expect(checkCompatibility({ min: null, max: null }, format)).toBe('compatible');
    expect(checkCompatibility({ min: { major: 88, minor: 0 }, max: null }, format)).toBe(
      'compatible',
    );
  });

  it('never calls a pack compatible when the world format or the pack range is missing', () => {
    const range = { min: { major: 88, minor: 0 }, max: null };
    expect(checkCompatibility(range, null)).toBe('unknown');
    expect(checkCompatibility(null, { major: 88, minor: 0 })).toBe('unknown');
    expect(checkCompatibility(range, { major: 101, minor: 1 })).toBe('compatible');
    expect(checkCompatibility({ min: { major: 101, minor: 1 }, max: null }, { major: 88, minor: 0 })).toBe(
      'incompatible',
    );
  });

  it('treats a max written as a whole number as covering every minor of that major', () => {
    const anyMinor = 2147483647;
    const range = { min: { major: 88, minor: 0 }, max: { major: 94, minor: anyMinor } };
    expect(checkCompatibility(range, { major: 94, minor: 1 })).toBe('compatible');
    expect(checkCompatibility(range, { major: 95, minor: 0 })).toBe('incompatible');
    expect(checkCompatibility(range, { major: 87, minor: 5 })).toBe('incompatible');
  });

  it('names no game version for a pack pinned to 94.0, which a 94.1 world rejects', () => {
    const world = { major: 94, minor: 1 };
    const pinned = datapack('pinned.zip', null, null, {
      format_range: { min: { major: 94, minor: 0 }, max: { major: 94, minor: 0 } },
      game_label: null,
    });
    const whole = datapack('whole.zip', null, null, {
      format_range: { min: { major: 94, minor: 0 }, max: { major: 94, minor: 2147483647 } },
      game_label: '1.21.11',
    });
    expect(checkCompatibility(pinned.format_range, world)).toBe('incompatible');
    expect(compatibilityWarning(pinned, world, 'World')).toBe(
      'Built for an unknown range, but World needs data format 94.1. It may not load, but you can still enable it.',
    );
    expect(checkCompatibility(whole.format_range, world)).toBe('compatible');
    expect(compatibilityWarning(whole, world, 'World')).toBeNull();
  });

  it('reports a read-only error for a corrupt level.dat', async () => {
    const broken = world({ issue: { code: 'level_dat', message: 'corrupt' }, version: null });
    const failing: WorldsClient = {
      ...worldsClient(),
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
    const lockedClient = { ...worldsClient(), listDatapacks: async () => ({
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
    const roundTrip = worldsClient({
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
    const twoWorlds = worldsClient({
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
    let resolveCreative!: (value: DatapackListing) => void;
    const creativeListing = new Promise<DatapackListing>((resolve) => {
      resolveCreative = resolve;
    });
    const switching = worldsClient({
      listWorlds: async () => [world(), second],
      listDatapacks: async (name: string) => {
        if (name === 'Creative Flat') return creativeListing;
        return {
          packs: { packs: [datapack('a.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
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
        packs: { packs: [datapack('c.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
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
    let resolveSurvival!: (value: DatapackListing) => void;
    const survivalListing = new Promise<DatapackListing>((resolve) => {
      resolveSurvival = resolve;
    });
    const creativeStack = stack({
      enabled: [{ kind: 'foreign', value: 'vanilla' }],
      disabled: [],
    });
    const racing = worldsClient({
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
        packs: { packs: [datapack('a.zip')], count: 1, bytes_read: 128, elapsed_ms: 1 },
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
    const twoWorlds = worldsClient({
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

  it('sends the rendered snapshot on a reorder, not a fresh listing', async () => {
    const listDatapacks = vi
      .fn<WorldsClient['listDatapacks']>()
      .mockResolvedValueOnce(datapackListing(['a.zip', 'b.zip']))
      .mockResolvedValue(datapackListing(['a.zip', 'b.zip', 'c.zip']));
    const reorder = vi.fn(async () => stack());
    const client = worldsClient({ listDatapacks, reorderDatapacks: reorder });
    const { result } = renderHook(() => useWorlds(client));
    await act(async () => {});
    await act(async () => {
      await result.current.reorderDatapacks(['file/b.zip', 'file/a.zip']);
    });
    expect(reorder).toHaveBeenCalledWith('Survival 1_17', ['file/b.zip', 'file/a.zip'], {
      enabled: ['file/a.zip', 'file/b.zip'],
      disabled: [],
    });
  });

  it('makes no listDatapacks call between the move and the write', async () => {
    const listDatapacks = vi.fn(async () => datapackListing(['a.zip', 'b.zip']));
    const listedAtWrite: number[] = [];
    const reorder = vi.fn(async () => {
      listedAtWrite.push(listDatapacks.mock.calls.length);
      return stack();
    });
    const client = worldsClient({ listDatapacks, reorderDatapacks: reorder });
    const { result } = renderHook(() => useWorlds(client));
    await act(async () => {});
    expect(listDatapacks).toHaveBeenCalledTimes(1);
    await act(async () => {
      await result.current.reorderDatapacks(['file/b.zip', 'file/a.zip']);
    });
    expect(listedAtWrite).toEqual([1]);
    expect(listDatapacks).toHaveBeenCalledTimes(1);
  });
});
