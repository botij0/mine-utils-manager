import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => ({ hits: [], has_more: false })),
  convertFileSrc: (path: string) => `asset://${path}`,
}));

import { invoke } from '@tauri-apps/api/core';
import { ipc } from './ipc';

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

const mockedInvoke = vi.mocked(invoke);

afterEach(() => {
  delete (window as TauriWindow).__TAURI_INTERNALS__;
  mockedInvoke.mockClear();
});

describe('tauri transport', () => {
  it('forwards the search offset in the invoke payload', async () => {
    (window as TauriWindow).__TAURI_INTERNALS__ = {};

    await ipc.searchModrinth('fresh', 'resourcepack', null, 'relevance', 40);

    expect(mockedInvoke).toHaveBeenCalledWith('search_modrinth', {
      query: 'fresh',
      kind: 'resourcepack',
      gameVersion: null,
      sort: 'relevance',
      offset: 40,
    });
  });
});
