import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { defaultConfig } from '../../lib/types';
import type { Config, World } from '../../lib/types';
import { worldsClient } from '../../test/fake-ipc';
import { world } from '../../test/fake-packs';
import { useWorlds } from './useWorlds';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

const survival = world({ path: '/old/saves/Survival 1_17' });
const creative = world({ dir_name: 'Creative Flat', path: '/old/saves/Creative Flat' });
const newFirst = world({ dir_name: 'New First', path: '/new/saves/New First' });
const newCreative = world({ dir_name: 'Creative Flat', path: '/new/saves/Creative Flat' });

function selectionClient() {
  let config: Config = { ...defaultConfig(), active_game_dir: '/old' };
  let available = [survival, creative];
  const setLastWorld = vi.fn(async (path: string) => { config = { ...config, last_world: path }; });
  const client = {
    ...worldsClient(),
    listWorlds: async () => available,
    getConfig: async () => config,
    setLastWorld,
  };
  return {
    client,
    get config() { return config; },
    switchDir() {
      config = { ...config, active_game_dir: '/new', last_world: null };
      available = [newFirst, newCreative];
    },
  };
}

describe('remembered world selection', () => {
  it('persists one explicit selection across reloads and restart without writing fallbacks', async () => {
    const state = selectionClient();
    const hook = renderHook(() => useWorlds(state.client, true, '', '/old'));
    await act(async () => {});
    expect(hook.result.current.selected?.path).toBe('/old/saves/Survival 1_17');
    expect(state.client.setLastWorld).not.toHaveBeenCalled();
    await act(async () => { hook.result.current.select('Creative Flat'); });
    expect(state.config.last_world).toBe('/old/saves/Creative Flat');
    await act(async () => { await hook.result.current.reload(); });
    hook.rerender();
    expect(hook.result.current.selected?.path).toBe('/old/saves/Creative Flat');
    expect(state.client.setLastWorld.mock.calls).toEqual([['/old/saves/Creative Flat']]);
    hook.unmount();
    const restarted = renderHook(() => useWorlds(state.client, true, '', '/old'));
    await act(async () => {});
    expect(restarted.result.current.selected?.path).toBe('/old/saves/Creative Flat');
    expect(state.client.setLastWorld.mock.calls).toEqual([['/old/saves/Creative Flat']]);
  });

  it('drops the old selection while hidden and selects the new directory first world on return', async () => {
    const state = selectionClient();
    const hook = renderHook(({ active, dir }) => useWorlds(state.client, active, '', dir), {
      initialProps: { active: true, dir: '/old' },
    });
    await act(async () => {});
    await act(async () => { hook.result.current.select('Creative Flat'); });
    hook.rerender({ active: false, dir: '/old' });
    state.switchDir();
    hook.rerender({ active: false, dir: '/new' });
    expect(hook.result.current.selected).toBeNull();
    expect(hook.result.current.worlds).toEqual([]);
    hook.rerender({ active: true, dir: '/new' });
    await act(async () => {});
    expect(hook.result.current.selected?.path).toBe('/new/saves/New First');
    expect(state.config.last_world).toBeNull();
    expect(state.client.setLastWorld.mock.calls).toEqual([['/old/saves/Creative Flat']]);
  });

  it('ignores a held old-directory world list and its remembered selection after a directory switch', async () => {
    const state = selectionClient();
    const held = deferred<World[]>();
    const listWorlds = vi.fn().mockReturnValueOnce(held.promise)
      .mockImplementation(() => state.client.listWorlds());
    const client = { ...state.client, listWorlds };
    const hook = renderHook(({ dir }) => useWorlds(client, true, '', dir), {
      initialProps: { dir: '/old' },
    });
    state.switchDir();
    hook.rerender({ dir: '/new' });
    await act(async () => {});
    expect(hook.result.current.selected?.path).toBe('/new/saves/New First');
    await act(async () => { held.resolve([survival, creative]); });
    expect(hook.result.current.selected?.path).toBe('/new/saves/New First');
    expect(hook.result.current.worlds.map((item) => item.path)).toEqual([
      '/new/saves/New First', '/new/saves/Creative Flat',
    ]);
    expect(state.config.last_world).toBeNull();
    expect(state.client.setLastWorld).not.toHaveBeenCalled();
  });

  it('drops queued old-directory persistence after a directory switch', async () => {
    const state = selectionClient();
    const held = deferred<void>();
    const setLastWorld = vi.fn().mockReturnValueOnce(held.promise)
      .mockImplementation(state.client.setLastWorld);
    const client = { ...state.client, setLastWorld };
    const hook = renderHook(({ dir }) => useWorlds(client, true, '', dir), {
      initialProps: { dir: '/old' },
    });
    await act(async () => {});
    await act(async () => { hook.result.current.select('Creative Flat'); });
    act(() => { hook.result.current.select('Survival 1_17'); });
    state.switchDir();
    hook.rerender({ dir: '/new' });
    await act(async () => { held.resolve(); });
    expect(hook.result.current.selected?.path).toBe('/new/saves/New First');
    expect(state.config.last_world).toBeNull();
    expect(setLastWorld.mock.calls).toEqual([['/old/saves/Creative Flat']]);
  });
});
