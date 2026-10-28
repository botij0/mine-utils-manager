import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { IpcClient } from '../../lib/ipc';
import type { ModrinthInstallOutcome, ModrinthSearchHit } from '../../lib/types';
import { makeFakeClient } from '../../test/fake-ipc';
import { freshHit, gameDir, mismatchHit } from '../../test/fake-packs';
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
    if (outcome && outcome.kind === 'resourcepack' && outcome.outcome.status === 'imported') {
      expect(outcome.outcome.pack.path).toBe(
        `/home/demo/.minecraft/resourcepacks/${outcome.outcome.pack.name}.zip`,
      );
      expect(outcome.outcome.pack.path).not.toContain('.zip.zip');
    } else {
      expect.unreachable('expected an imported resourcepack outcome');
    }
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

  it('clears stale hits when the kind tab changes, then searches the new tab', async () => {
    const client = makeFakeClient([]);
    const result = await searchFor(client, 'fresh');
    expect(result.current.hits.length).toBeGreaterThan(0);
    expect(
      result.current.hits.every((hit) => hit.project_type === 'resourcepack'),
    ).toBe(true);
    await act(async () => {
      result.current.setKind('datapack');
    });
    // The old tab's hits are dropped synchronously, before the new tab's
    // search for the same query lands.
    expect(
      result.current.hits.every((hit) => hit.project_type !== 'resourcepack'),
    ).toBe(true);
    await flushSearch();
    expect(result.current.hits.length).toBeGreaterThan(0);
    expect(
      result.current.hits.every((hit) => hit.project_type === 'datapack'),
    ).toBe(true);
  });

  it('ignores stale project responses when the selection changes fast', async () => {
    const client = makeFakeClient([]);
    const slowHit: ModrinthSearchHit = { ...freshHit(), project_id: 'slow', title: 'Slow' };
    const fastHit: ModrinthSearchHit = { ...freshHit(), project_id: 'fast', title: 'Fast' };
    let resolveSlow!: () => void;
    const slowGate = new Promise<void>((resolve) => {
      resolveSlow = resolve;
    });
    const origProject = client.getModrinthProject.bind(client);
    const origVersions = client.listModrinthVersions.bind(client);
    client.getModrinthProject = async (id) => {
      if (id === 'slow') await slowGate;
      return origProject(id);
    };
    client.listModrinthVersions = async (id) => {
      if (id === 'slow') await slowGate;
      return origVersions(id);
    };
    const { result } = renderHook(() => useModrinth(client));

    let first!: Promise<void>;
    await act(async () => {
      first = result.current.select(slowHit);
    });
    await act(async () => {
      await result.current.select(fastHit);
    });
    await act(async () => {
      resolveSlow();
      await first;
    });
    await act(async () => {});
    expect(result.current.selected?.project_id).toBe('fast');
    // The slow response landed last but must not overwrite the fast one.
    expect(result.current.project?.title).toBe('fast');
    expect(result.current.detailLoading).toBe(false);
  });

  it('re-runs the version preference when the datapack world changes', async () => {    const client = makeFakeClient([]);
    const origTargets = client.modrinthTargets.bind(client);
    client.modrinthTargets = async (world) => ({
      ...(await origTargets(world)),
      world_version: world === 'nether' ? '1.20.1' : '1.21.1',
    });
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      result.current.setKind('datapack');
    });
    await act(async () => {
      result.current.setWorld('overworld');
    });
    await act(async () => {
      await result.current.select(freshHit());
    });
    expect(result.current.selectedVersionId).toBe('v2');
    await act(async () => {
      result.current.setWorld('nether');
    });
    await act(async () => {});
    await act(async () => {});
    // v2 supports only 1.21.1, so the preference resets to the 1.20.1 build.
    expect(result.current.selectedVersionId).toBe('v1');
  });

  it('drops a late search response after the query is cleared', async () => {
    const client = makeFakeClient([]);
    let release!: (hits: ModrinthSearchHit[]) => void;
    const gate = new Promise<ModrinthSearchHit[]>((resolve) => {
      release = resolve;
    });
    client.searchModrinth = () => gate;
    const { result } = renderHook(() => useModrinth(client));

    await act(async () => {
      result.current.setQuery('fresh');
    });
    await act(async () => {
      vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS + 50);
    });
    // Clear the box while the search is still in flight.
    await act(async () => {
      result.current.setQuery('');
    });
    await act(async () => {
      vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS + 50);
    });
    await act(async () => {
      release([freshHit()]);
    });
    await act(async () => {});
    await act(async () => {});
    expect(result.current.hits).toEqual([]);
    expect(result.current.searched).toBe(false);
  });

  it('closeDetail invalidates an in-flight project fetch', async () => {
    const client = makeFakeClient([]);
    let resolveProject!: (value: unknown) => void;
    const gate = new Promise((resolve) => {
      resolveProject = resolve;
    });
    const origProject = client.getModrinthProject.bind(client);
    const origVersions = client.listModrinthVersions.bind(client);
    client.getModrinthProject = async (id) => {
      const project = await origProject(id);
      await gate;
      return project;
    };
    client.listModrinthVersions = async (id) => {
      const versions = await origVersions(id);
      await gate;
      return versions;
    };
    const { result } = renderHook(() => useModrinth(client));

    let selected!: Promise<void>;
    await act(async () => {
      selected = result.current.select(freshHit());
    });
    await act(async () => {
      result.current.closeDetail();
    });
    await act(async () => {
      resolveProject(undefined);
      await selected;
    });
    await act(async () => {});
    expect(result.current.selected).toBeNull();
    expect(result.current.project).toBeNull();
    expect(result.current.versions).toEqual([]);
  });

  it('datapack tab keeps only .zip versions and installs the zip file', async () => {
    const client = makeFakeClient([]);
    const zip = {
      id: 'zip-v',
      project_id: 'mixed-dp',
      name: 'Mixed 2.0',
      version_number: '2.0',
      game_versions: ['1.21.1'],
      loaders: ['datapack'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/mixed/mixed-2.0.zip',
          filename: 'mixed-2.0.zip',
          size: 256,
          primary: true,
          hashes: { sha512: 'zz', sha1: 'z' },
        },
      ],
    };
    const jar = {
      id: 'jar-v',
      project_id: 'mixed-dp',
      name: 'Mixed 1.0 fabric',
      version_number: '1.0',
      game_versions: ['1.21.1'],
      loaders: ['fabric'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/mixed/mixed-1.0.jar',
          filename: 'mixed-1.0.jar',
          size: 512,
          primary: true,
          hashes: { sha512: 'jj', sha1: 'j' },
        },
      ],
    };
    client.listModrinthVersions = async () => [zip, jar];
    let sentFilename: string | null = null;
    const origInstall = client.installModrinthPack.bind(client);
    client.installModrinthPack = async (args) => {
      sentFilename = args.filename;
      return origInstall(args);
    };
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      result.current.setKind('datapack');
    });
    await act(async () => {
      await result.current.select({ ...freshHit(), project_id: 'mixed-dp' });
    });
    expect(result.current.versions.map((version) => version.id)).toEqual(['zip-v']);
    expect(result.current.selectedVersionId).toBe('zip-v');
    await act(async () => {
      await result.current.install();
    });
    expect(sentFilename).toBe('mixed-2.0.zip');
  });

  it('datapack tab with only jar versions selects nothing and blocks install', async () => {
    const client = makeFakeClient([]);
    client.listModrinthVersions = async () => [
      {
        id: 'jar-only',
        project_id: 'jar-only',
        name: 'Jar Only 1.0',
        version_number: '1.0',
        game_versions: ['1.21.1'],
        loaders: ['fabric'],
        files: [
          {
            url: 'https://cdn.modrinth.com/data/jar/jar-1.0.jar',
            filename: 'jar-1.0.jar',
            size: 512,
            primary: true,
            hashes: { sha512: 'jj', sha1: 'j' },
          },
        ],
      },
    ];
    const { result } = renderHook(() => useModrinth(client));
    await act(async () => {
      result.current.setKind('datapack');
    });
    await act(async () => {
      await result.current.select({ ...freshHit(), project_id: 'jar-only' });
    });
    expect(result.current.versions).toEqual([]);
    expect(result.current.selectedVersionId).toBeNull();
    await act(async () => {
      await result.current.install();
    });
    expect(result.current.installResult).toBeNull();
    expect(result.current.installError).not.toBeNull();
  });
});
