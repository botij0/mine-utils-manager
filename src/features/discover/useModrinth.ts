import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { DiscoverClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
  IpcError,
  ModrinthInstallOutcome,
  ModrinthInstallPhase,
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

export type TargetsStatus = 'loading' | 'failed' | 'ready';
export type InstallPhase = 'idle' | 'preparing' | ModrinthInstallPhase;

type TargetsLoad =
  | { status: 'loading' }
  | { status: 'failed' }
  | { status: 'ready'; targets: ModrinthTargets };

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
  targetsStatus: TargetsStatus;
  reloadTargets: () => void;
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
  cancelInstall: () => Promise<void>;
  installPhase: InstallPhase;
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
export function useModrinth(
  client: DiscoverClient,
  active = true,
  reloadSignal = '',
  gameDir: string | null = null,
  onGameRunning?: () => void,
  onInstalled?: (kind: ModrinthKind) => void,
): ModrinthState {
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
  const [targetsLoad, setTargetsLoad] = useState<TargetsLoad>({ status: 'loading' });
  const [targetsAttempt, setTargetsAttempt] = useState(0);
  const targets = targetsLoad.status === 'ready' ? targetsLoad.targets : null;
  const [world, setWorld] = useState<string | null>(null);
  const [selected, setSelected] = useState<ModrinthSearchHit | null>(null);
  const [project, setProject] = useState<ModrinthProject | null>(null);
  const [versions, setVersions] = useState<ModrinthVersion[]>([]);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<IpcError | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [installPhase, setInstallPhase] = useState<InstallPhase>('idle');
  const [installResult, setInstallResult] = useState<ModrinthInstallOutcome | null>(null);
  const [installError, setInstallError] = useState<IpcError | null>(null);
  const installId = useRef<string | null>(null);
  const installCallbacks = useRef({ onGameRunning, onInstalled });
  useLayoutEffect(() => {
    installCallbacks.current = { onGameRunning, onInstalled };
  }, [onGameRunning, onInstalled]);
  const requestId = useRef(0);
  const selectToken = useRef(0);
  const mounted = useRef(true);
  const loadingMore = useRef(false);
  const [retryNonce, setRetryNonce] = useState(0);
  const loadedTargets = useRef<string | null>(null);
  const versionChoice = useRef<{ manual: boolean; target: string | null }>({ manual: false, target: null });

  useLayoutEffect(() => {
    loadedTargets.current = null;
    setWorld(null);
    setTargetsLoad({ status: 'loading' });
    setInstallResult(null);
    setInstallError(null);
  }, [gameDir]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      selectToken.current += 1;
      const id = installId.current;
      if (!id) return;
      void client
        .cancelModrinthInstall(id)
        .then((cancelled) => {
          if (!cancelled && mounted.current && installId.current === id) setInstallPhase('committing');
        })
        .catch(() => {});
    };
  }, [client, gameDir]);

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
    const key = JSON.stringify([reloadSignal, world]);
    if (!active || loadedTargets.current === key) return;
    let cancelled = false;
    setTargetsLoad({ status: 'loading' });
    client
      .modrinthTargets(world)
      .then((loaded) => {
        if (!cancelled && mounted.current) {
          loadedTargets.current = key;
          setTargetsLoad({ status: 'ready', targets: loaded });
        }
      })
      .catch(() => {
        if (!cancelled && mounted.current) setTargetsLoad({ status: 'failed' });
      });
    return () => {
      cancelled = true;
    };
  }, [client, world, active, reloadSignal, gameDir, targetsAttempt]);

  const targetVersion =
    kind === 'datapack'
      ? (targets?.world_version ?? null)
      : (targets?.preferred_launcher ?? null);

  useEffect(() => {
    if (!targetVersion) return;
    const targetChanged = versionChoice.current.target !== targetVersion;
    versionChoice.current.target = targetVersion;
    setSelectedVersionId((current) => {
      const picked = versions.find((version) => version.id === current);
      if (versionChoice.current.manual && picked &&
          (!targetChanged || picked.game_versions.includes(targetVersion))) return current;
      versionChoice.current.manual = false;
      return preferModrinthVersion(versions, targetVersion)?.id ?? null;
    });
  }, [versions, targetVersion]);

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
      versionChoice.current = { manual: false, target: targetVersion };
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
    [client, kind, targets, targetVersion],
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

  const chooseVersion = useCallback((id: string) => {
    versionChoice.current.manual = true;
    setSelectedVersionId(id);
  }, []);

  const retry = useCallback(() => {
    setOffline(false);
    setError(null);
    setRetryNonce((nonce) => nonce + 1);
  }, []);

  const reloadTargets = useCallback(() => {
    loadedTargets.current = null;
    setTargetsAttempt((attempt) => attempt + 1);
  }, []);

  const install = useCallback(async (): Promise<ModrinthInstallOutcome | null> => {
    if (!selected || installId.current) return null;
    const version = versions.find((item) => item.id === selectedVersionId) ?? null;
    const file = version?.files.find((item) => item.primary) ?? version?.files[0] ?? null;
    if (!version || !file) {
      setInstallError(toAppError(new Error('pick a version with a downloadable file first')));
      return null;
    }
    const token = selectToken.current;
    const id = crypto.randomUUID();
    installId.current = id;
    setInstallPhase('preparing');
    setInstallError(null);
    try {
      const outcome = await client.installModrinthPack({
        installId: id,
        projectId: selected.project_id,
        versionId: version.id,
        filename: file.filename,
        kind,
        world: kind === 'datapack' ? (world ?? undefined) : undefined,
      }, (phase) => {
        if (mounted.current && installId.current === id) {
          setInstallPhase((current) => current === 'committing' ? current : phase);
        }
      });
      if (mounted.current && selectToken.current === token) setInstallResult(outcome);
      if (outcome.outcome.status === 'imported' || outcome.outcome.status === 'locked') {
        installCallbacks.current.onInstalled?.(outcome.kind);
      }
      return outcome;
    } catch (err) {
      const appError = toAppError(err);
      if (mounted.current && selectToken.current === token) setInstallError(appError);
      if (appError.code === 'game_running') installCallbacks.current.onGameRunning?.();
      if (appError.installed_path) installCallbacks.current.onInstalled?.(kind);
      return null;
    } finally {
      installId.current = null;
      if (mounted.current) setInstallPhase('idle');
    }
  }, [client, kind, selected, selectedVersionId, versions, world]);

  const cancelInstall = useCallback(async () => {
    const id = installId.current;
    if (!id) return;
    const token = selectToken.current;
    try {
      const cancelled = await client.cancelModrinthInstall(id);
      if (!cancelled && mounted.current && installId.current === id) {
        setInstallPhase('committing');
      }
    } catch (err) {
      if (mounted.current && installId.current === id && selectToken.current === token) {
        setInstallError(toAppError(err));
      }
    }
  }, [client]);

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
    targetsStatus: targetsLoad.status,
    reloadTargets,
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
    setSelectedVersionId: chooseVersion,
    cancelInstall,
    installPhase,
    installResult,
    installError,
    install,
    clearInstall,
    closeDetail,
  };
}
