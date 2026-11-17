import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => ({ hits: [], has_more: false })),
  convertFileSrc: (path: string) => `asset://${path}`,
}));

import { invoke } from '@tauri-apps/api/core';
import { ipc } from './ipc';
import type { ModrinthInstallOutcome } from './generated/wire';
import { pack } from '../test/fake-packs';

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

it('converts the generated install result icon path to an asset URL', async () => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
  const installed = {
    kind: 'resourcepack',
    outcome: {
      status: 'imported',
      pack: { ...pack('Fresh'), icon: { sha256: 'digest', path: '/cache/fresh.png' } },
    },
    provenance_warning: 'Could not save provenance',
  } satisfies ModrinthInstallOutcome;
  mockedInvoke.mockResolvedValueOnce(undefined).mockResolvedValueOnce(installed);

  const result = await ipc.installModrinthPack({
    installId: 'wire-icon', projectId: 'fresh', versionId: 'v1',
    filename: 'fresh.zip', kind: 'resourcepack',
  });

  expect(result).toEqual({
    ...installed,
    outcome: { ...installed.outcome, pack: { ...installed.outcome.pack, icon: { sha256: 'digest', url: 'asset:///cache/fresh.png' } } },
  });
});
