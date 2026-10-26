import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { IpcClient } from '../../lib/ipc';
import type { ModrinthInstallOutcome, ModrinthSearchHit } from '../../lib/types';
import { gameDir, makeFakeClient } from '../../test/fake-ipc';
import {
  SEARCH_DEBOUNCE_MS,
  isOfflineError,
  preferModrinthVersion,
  useModrinth,
} from './useModrinth';

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

function freshHit(): ModrinthSearchHit {
  return {
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
  };
}

function mismatchHit(): ModrinthSearchHit {
  return {
    ...freshHit(),
    project_id: 'mm1',
    slug: 'mismatch-pack',
    title: 'Mismatch Pack',
  };
}

async function flushSearch() {
  // One pass fires the debounce timer; the next passes settle the search
  // promise chain (then, state, finally) without any real waiting.
  await act(async () => {
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS + 50);
  });
  await act(async () => {});
  await act(async () => {});
  await act(async () => {});
}

async function searchFor(client: IpcClient, text: string) {
  const { result } = renderHook(() => useModrinth(client));
  await act(async () => {
    result.current.setQuery(text);
  });
  await flushSearch();
  return result;
}

describe('useModrinth', () => {
  it('debounces the query: one request per burst', async () => {
    const client = makeFakeClient([]);
    const seen: string[] = [];
    const orig = client.searchModrinth.bind(client);
    client.searchModrinth = async (...args) => {
      seen.push(args[0]);
      return orig(...args);
    };
    const { result } = renderHook(() => useModrinth(client));

    await act(async () => {
      result.current.setQuery('f');
      result.current.setQuery('fr');
      result.current.setQuery('fresh');
    });
    expect(seen).toEqual([]);

    await flushSearch();
    expect(seen).toEqual(['fresh']);
  });

  it('switches facets with the type tabs: datapacks list only datapacks', async () => {
    const client = makeFakeClient([]);
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      result.current.setKind('datapack');
      result.current.setQuery('pure');
    });
    await flushSearch();
    expect(result.current.searched).toBe(true);
    expect(result.current.hits.map((hit) => hit.slug)).toEqual(['pure-datapack']);
    expect(result.current.targetVersion).toBe('1.21.1');
  });

  it('preselects the compatible version but keeps the full list', async () => {
    const client = makeFakeClient([]);
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      await result.current.select(freshHit());
    });
    expect(result.current.versions.map((version) => version.id)).toEqual(['v2', 'v1']);
    expect(result.current.selectedVersionId).toBe('v2');
    expect(preferModrinthVersion(result.current.versions, '1.20.1')?.id).toBe('v1');
    expect(preferModrinthVersion(result.current.versions, '1.19')?.id).toBe('v2');
    expect(preferModrinthVersion([], '1.21.1')).toBeNull();
  });

  it('installs the selected version and reports the outcome', async () => {
    const client = makeFakeClient([gameDir('/home/demo/.minecraft', true, true)]);
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      await result.current.select(freshHit());
    });
    let outcome: ModrinthInstallOutcome | null | undefined;
    await act(async () => {
      outcome = await result.current.install();
    });
    expect(outcome && outcome.kind).toBe('resourcepack');
    expect(result.current.installError).toBeNull();
    expect(result.current.installResult?.kind).toBe('resourcepack');
  });

  it('reports a hash mismatch with the expected and actual hashes', async () => {
    const client = makeFakeClient([]);
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      await result.current.select(mismatchHit());
    });
    await act(async () => {
      await result.current.install();
    });
    expect(result.current.installResult).toBeNull();
    expect(result.current.installError?.code).toBe('download_hash_mismatch');
    expect(result.current.installError?.message).toContain('expected 00aa');
    expect(result.current.installError?.message).toContain('got ffbb');
  });

  it('shows the offline state on a network error and retries cleanly', async () => {
    const client = makeFakeClient([]);
    const result = await searchFor(client, '__offline__');
    expect(result.current.offline).toBe(true);
    expect(result.current.error?.code).toBe('modrinth_offline');
    expect(isOfflineError(result.current.error!)).toBe(true);

    await act(async () => {
      result.current.setQuery('fresh');
    });
    await flushSearch();
    expect(result.current.offline).toBe(false);
    expect(result.current.hits.length).toBeGreaterThan(0);
  });

  it('surfaces a server error as an inline error, not offline', async () => {
    const client = makeFakeClient([]);
    const result = await searchFor(client, '__boom__');
    expect(result.current.offline).toBe(false);
    expect(result.current.error?.code).toBe('modrinth_status');
  });

  it('makes no request before the query is typed', async () => {
    const client = makeFakeClient([]);
    let calls = 0;
    const orig = client.searchModrinth.bind(client);
    client.searchModrinth = (...args) => {
      calls += 1;
      return orig(...args);
    };
    renderHook(() => useModrinth(client));
    await act(async () => {
      vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS + 500);
    });
    expect(calls).toBe(0);
  });
});
