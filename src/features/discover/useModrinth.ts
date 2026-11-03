import { useCallback, useEffect, useRef, useState } from 'react';
import type { DiscoverClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
  IpcError,
  ModrinthInstallOutcome,
  ModrinthKind,
  ModrinthProject,
  ModrinthSearchHit,
  ModrinthSort,
  ModrinthTargets,
  ModrinthVersion,
} from '../../lib/types';

/** The search box waits this long after the last keystroke before firing. */
export const SEARCH_DEBOUNCE_MS = 350;

/** First version compatible with the target, else the newest. The picker
 *  keeps the full list; this only preselects. */
export function preferModrinthVersion(
  versions: ModrinthVersion[],
  target: string | null,
): ModrinthVersion | null {
  if (versions.length === 0) return null;
  if (target) {
    const hit = versions.find((version) => version.game_versions.includes(target));
    if (hit) return hit;
  }
  return versions[0];
}

/** A datapack version installs as a `.zip` (the backend `data/` root check
 *  and the `.zip` guard both require it). A version can also carry mod
 *  `.jar` files, which can never install as a datapack. */
export function isDatapackVersion(version: ModrinthVersion): boolean {
  const file = version.files.find((item) => item.primary) ?? version.files[0];
  return !!file && file.filename.toLowerCase().endsWith('.zip');
}

/** Versions visible for a tab: datapacks keep only `.zip` versions (no jar
 *  fallback — an empty list renders the empty-versions state); resource
 *  packs keep every version. */
export function versionsForKind(
  versions: ModrinthVersion[],
  kind: ModrinthKind,
): ModrinthVersion[] {
  if (kind !== 'datapack') return versions;
  return versions.filter(isDatapackVersion);
}

/** Network-level failures show the offline state; every other failure shows
 *  the inline error with a retry. */
export function isOfflineError(err: IpcError): boolean {
  return err.code === 'modrinth_offline' || err.code === 'download_offline';
}

/** A response belongs to the current search only while the page is mounted
 *  and no newer search has started. */
export function isCurrentSearchResponse(
  mounted: boolean,
  responseRequestId: number,
  currentRequestId: number,
): boolean {
  return mounted && responseRequestId === currentRequestId;
}

interface PaginationState {
  nextOffset: number | null;
  loading: boolean;
  error: IpcError | null;
}

const EMPTY_PAGINATION: PaginationState = {
  nextOffset: null,
  loading: false,
  error: null,
};

export interface ModrinthState {
  query: string;
  setQuery: (query: string) => void;
  kind: ModrinthKind;
  setKind: (kind: ModrinthKind) => void;
  gameVersion: string;
  setGameVersion: (version: string) => void;
  sort: ModrinthSort;
  setSort: (sort: ModrinthSort) => void;
  hits: ModrinthSearchHit[];
  searching: boolean;
  searched: boolean;
  error: IpcError | null;
  offline: boolean;
  retry: () => void;
  hasMore: boolean;
  loadingMore: boolean;
  loadMoreError: IpcError | null;
  loadMore: () => Promise<void>;
  targets: ModrinthTargets | null;
  /** The game version the picker prefers: the launcher version for resource
   *  packs, the selected world's version for datapacks. */
  targetVersion: string | null;
  world: string | null;
  setWorld: (world: string | null) => void;
  selected: ModrinthSearchHit | null;
  select: (hit: ModrinthSearchHit | null) => Promise<void>;
  project: ModrinthProject | null;
  versions: ModrinthVersion[];
  detailLoading: boolean;
  detailError: IpcError | null;
  selectedVersionId: string | null;
  setSelectedVersionId: (id: string) => void;
  installBusy: boolean;
  installResult: ModrinthInstallOutcome | null;
  installError: IpcError | null;
  install: () => Promise<ModrinthInstallOutcome | null>;
  clearInstall: () => void;
  closeDetail: () => void;
}

/**
 * Discover state. Nothing fires before the page mounts, and a search fires
 * only for a non-empty query: opening the tab never hits the network.
 */
export function useModrinth(client: DiscoverClient): ModrinthState {
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState<ModrinthKind>('resourcepack');
  const [gameVersion, setGameVersion] = useState('');
  const [sort, setSort] = useState<ModrinthSort>('relevance');
  const [debouncedQuery, setDebouncedQuery] = useState('');
  const [hits, setHits] = useState<ModrinthSearchHit[]>([]);
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);
  const [offline, setOffline] = useState(false);
  const [pagination, setPagination] = useState<PaginationState>(EMPTY_PAGINATION);
  const [targets, setTargets] = useState<ModrinthTargets | null>(null);
  const [world, setWorld] = useState<string | null>(null);
  const [selected, setSelected] = useState<ModrinthSearchHit | null>(null);
  const [project, setProject] = useState<ModrinthProject | null>(null);
  const [versions, setVersions] = useState<ModrinthVersion[]>([]);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<IpcError | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [installBusy, setInstallBusy] = useState(false);
  const [installResult, setInstallResult] = useState<ModrinthInstallOutcome | null>(null);
  const [installError, setInstallError] = useState<IpcError | null>(null);
  const requestId = useRef(0);
  const selectToken = useRef(0);
  const mounted = useRef(true);
  const loadingMore = useRef(false);
  const [retryNonce, setRetryNonce] = useState(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const resetSearchResults = useCallback(() => {
    loadingMore.current = false;
    setHits([]);
    setError(null);
    setOffline(false);
    setPagination(EMPTY_PAGINATION);
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(query.trim()), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    let cancelled = false;
    client
      .modrinthTargets(world)
      .then((loaded) => {
        if (!cancelled && mounted.current) setTargets(loaded);
      })
      .catch(() => {
        if (!cancelled && mounted.current) setTargets(null);
      });
    return () => {
      cancelled = true;
    };
  }, [client, world]);

  const targetVersion =
    kind === 'datapack'
      ? (targets?.world_version ?? null)
      : (targets?.preferred_launcher ?? null);

  useEffect(() => {
    if (!debouncedQuery) {
      // Invalidate any in-flight search so a late response cannot repopulate
      // hits under an empty box.
      requestId.current += 1;
      resetSearchResults();
      setSearching(false);
      setSearched(false);
      return;
    }
    const id = requestId.current + 1;
    requestId.current = id;
    resetSearchResults();
    setSearching(true);
    client
      .searchModrinth(debouncedQuery, kind, gameVersion || null, sort, 0)
      .then((found) => {
        if (!isCurrentSearchResponse(mounted.current, id, requestId.current)) return;
        setHits(found.hits);
        setPagination({
          nextOffset: found.has_more ? found.hits.length : null,
          loading: false,
          error: null,
        });
        setError(null);
        setOffline(false);
        setSearched(true);
      })
      .catch((err: unknown) => {
        if (!isCurrentSearchResponse(mounted.current, id, requestId.current)) return;
        const failure = toAppError(err);
        resetSearchResults();
        setError(failure);
        setOffline(isOfflineError(failure));
        setSearched(true);
      })
      .finally(() => {
        if (isCurrentSearchResponse(mounted.current, id, requestId.current)) {
          setSearching(false);
        }
      });
    // retryNonce re-fires the same query after an error without new typing.
  }, [client, debouncedQuery, kind, gameVersion, sort, retryNonce, resetSearchResults]);

  const loadMore = useCallback(async () => {
    const offset = pagination.nextOffset;
    if (
      offset === null ||
      loadingMore.current ||
      searching ||
      query.trim() !== debouncedQuery
    ) {
      return;
    }
    const id = requestId.current;
    loadingMore.current = true;
    setPagination((current) => ({
      nextOffset: current.nextOffset,
      loading: true,
      error: null,
    }));
    try {
      const page = await client.searchModrinth(
        debouncedQuery,
        kind,
        gameVersion || null,
        sort,
        offset,
      );
      if (!isCurrentSearchResponse(mounted.current, id, requestId.current)) return;
      setHits((current) => {
        const seen = new Set(current.map((hit) => hit.project_id));
        const appended: ModrinthSearchHit[] = [];
        for (const hit of page.hits) {
          if (seen.has(hit.project_id)) continue;
          seen.add(hit.project_id);
          appended.push(hit);
        }
        return [...current, ...appended];
      });
      setPagination({
        nextOffset: page.has_more ? offset + page.hits.length : null,
        loading: false,
        error: null,
      });
    } catch (err) {
      if (!isCurrentSearchResponse(mounted.current, id, requestId.current)) return;
      setPagination((current) => ({
        nextOffset: current.nextOffset,
        loading: false,
        error: toAppError(err),
      }));
    } finally {
      if (isCurrentSearchResponse(mounted.current, id, requestId.current)) {
        loadingMore.current = false;
      }
    }
  }, [
    client,
    debouncedQuery,
    kind,
    gameVersion,
    sort,
    pagination.nextOffset,
    query,
    searching,
  ]);

  const select = useCallback(
    async (hit: ModrinthSearchHit | null) => {
      const token = selectToken.current + 1;
      selectToken.current = token;
      setSelected(hit);
      setProject(null);
      setVersions([]);
      setDetailError(null);
      setSelectedVersionId(null);
      setInstallResult(null);
      setInstallError(null);
      if (!hit) return;
      setDetailLoading(true);
      try {
        const [loadedProject, loadedVersions] = await Promise.all([
          client.getModrinthProject(hit.project_id),
          client.listModrinthVersions(hit.project_id),
        ]);
        if (!mounted.current || selectToken.current !== token) return;
        setProject(loadedProject);
        const visible = versionsForKind(loadedVersions, kind);
        setVersions(visible);
        const preferred = preferModrinthVersion(
          visible,
          kind === 'datapack' ? targets?.world_version ?? null : (targets?.preferred_launcher ?? null),
        );
        setSelectedVersionId(preferred?.id ?? visible[0]?.id ?? null);
      } catch (err) {
        if (!mounted.current || selectToken.current !== token) return;
        setDetailError(toAppError(err));
      } finally {
        if (mounted.current && selectToken.current === token) setDetailLoading(false);
      }
    },
    [client, kind, targets],
  );

  const setKindAndClear = useCallback((next: ModrinthKind) => {
    // A new tab is a new search: drop the old hits so a stale datapack list
    // never renders under the resource-pack tab (and vice versa).
    requestId.current += 1;
    // Invalidate any in-flight project/version fetch so it cannot repopulate
    // hidden detail state after the tab switch.
    selectToken.current += 1;
    setKind(next);
    resetSearchResults();
    setSearched(false);
    setSearching(false);
    setSelected(null);
    setProject(null);
    setVersions([]);
    setInstallResult(null);
    setInstallError(null);
  }, [resetSearchResults]);

  // When the datapack target world changes, re-run the version preference:
  // a version picked for the old world's game version may not support the
  // new one. A compatible manual pick is left alone; an incompatible one is
  // reset to the compatible-first (or newest) version.
  const worldTarget = targets?.world_version ?? null;
  useEffect(() => {
    if (kind !== 'datapack' || versions.length === 0) return;
    setSelectedVersionId((current) => {
      const picked = versions.find((version) => version.id === current) ?? null;
      if (picked && (!worldTarget || picked.game_versions.includes(worldTarget))) {
        return current;
      }
      return preferModrinthVersion(versions, worldTarget)?.id ?? current;
    });
    // Runs only on target-world changes, not on every versions load.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, worldTarget]);

  const retry = useCallback(() => {
    setOffline(false);
    setError(null);
    setRetryNonce((nonce) => nonce + 1);
  }, []);

  const install = useCallback(async (): Promise<ModrinthInstallOutcome | null> => {
    if (!selected) return null;
    const version = versions.find((item) => item.id === selectedVersionId) ?? null;
    const file = version?.files.find((item) => item.primary) ?? version?.files[0] ?? null;
    if (!version || !file) {
      setInstallError(toAppError(new Error('pick a version with a downloadable file first')));
      return null;
    }
    setInstallBusy(true);
    setInstallError(null);
    try {
      const outcome = await client.installModrinthPack({
        projectId: selected.project_id,
        versionId: version.id,
        filename: file.filename,
        kind,
        world: kind === 'datapack' ? (world ?? undefined) : undefined,
      });
      if (mounted.current) setInstallResult(outcome);
      return outcome;
    } catch (err) {
      if (mounted.current) setInstallError(toAppError(err));
      return null;
    } finally {
      if (mounted.current) setInstallBusy(false);
    }
  }, [client, kind, selected, selectedVersionId, versions, world]);

  const clearInstall = useCallback(() => {
    setInstallResult(null);
    setInstallError(null);
  }, []);

  const closeDetail = useCallback(() => {
    // Invalidate any in-flight fetch so it cannot repopulate hidden state
    // after the dialog closes.
    selectToken.current += 1;
    setSelected(null);
    setProject(null);
    setVersions([]);
    setDetailError(null);
    setSelectedVersionId(null);
    setInstallResult(null);
    setInstallError(null);
  }, []);

  const querySettled = query.trim() === debouncedQuery;

  return {
    query,
    setQuery,
    kind,
    setKind: setKindAndClear,
    gameVersion,
    setGameVersion,
    sort,
    setSort,
    hits,
    searching,
    searched,
    error,
    offline,
    retry,
    hasMore: pagination.nextOffset !== null && querySettled,
    loadingMore: pagination.loading,
    loadMoreError: pagination.error,
    loadMore,
    targets,
    targetVersion,
    world,
    setWorld,
    selected,
    select,
    project,
    versions,
    detailLoading,
    detailError,
    selectedVersionId,
    setSelectedVersionId,
    installBusy,
    installResult,
    installError,
    install,
    clearInstall,
    closeDetail,
  };
}
