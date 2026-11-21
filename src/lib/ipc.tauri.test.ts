import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', async (importOriginal) => ({
  ...await importOriginal<typeof import('@tauri-apps/api/core')>(),
  invoke: vi.fn(async () => ({ hits: [], has_more: false })),
  convertFileSrc: (path: string) => `asset://${path}`,
}));

import { Channel, invoke } from '@tauri-apps/api/core';
import { ipc } from './ipc';
import type { ModrinthInstallOutcome, ModrinthInstallPhase } from './generated/wire';
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

it('forwards install progress and converts the result icon path to an asset URL', async () => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', {
    configurable: true, value: { transformCallback: () => 123 },
  });
  const installed = {
    kind: 'resourcepack',
    outcome: {
      status: 'imported',
      pack: { ...pack('Fresh'), icon: { sha256: 'digest', path: '/cache/fresh.png' } },
    },
    provenance_warning: 'Could not save provenance',
  } satisfies ModrinthInstallOutcome;
  mockedInvoke.mockResolvedValueOnce(undefined).mockImplementationOnce(async (_command, args) => {
    if (!args || typeof args !== 'object' || !('progress' in args) || !(args.progress instanceof Channel)) {
      throw new Error('install progress channel missing');
    }
    expect(args.progress.toJSON()).toBe('__CHANNEL__:123');
    args.progress.onmessage('waiting_for_slot');
    args.progress.onmessage('downloading');
    args.progress.onmessage('waiting_for_folder_lock');
    args.progress.onmessage('committing');
    return installed;
  });

  const phases: ModrinthInstallPhase[] = [];
  const result = await ipc.installModrinthPack({
    installId: 'wire-icon', projectId: 'fresh', versionId: 'v1',
    filename: 'fresh.zip', kind: 'resourcepack',
  }, (phase) => phases.push(phase));

  expect(phases).toEqual(['waiting_for_slot', 'downloading', 'waiting_for_folder_lock', 'committing']);
  expect(result).toEqual({
    ...installed,
    outcome: { ...installed.outcome, pack: { ...installed.outcome.pack, icon: { sha256: 'digest', url: 'asset:///cache/fresh.png' } } },
  });
});
