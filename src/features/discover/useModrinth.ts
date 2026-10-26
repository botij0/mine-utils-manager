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

/** Network-level failures show the offline state; every other failure shows
 *  the inline error with a retry. */
export function isOfflineError(err: IpcError): boolean {
  return err.code === 'modrinth_offline' || err.code === 'download_offline';
}

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
  const mounted = useRef(true);
  const [retryNonce, setRetryNonce] = useState(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
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
      setHits([]);
      setSearching(false);
      setError(null);
      setOffline(false);
      setSearched(false);
      return;
    }
    const id = requestId.current + 1;
    requestId.current = id;
    setSearching(true);
    client
      .searchModrinth(debouncedQuery, kind, gameVersion || null, sort, 20, 0)
      .then((found) => {
        if (!mounted.current || requestId.current !== id) return;
        setHits(found);
        setError(null);
        setOffline(false);
        setSearched(true);
      })
      .catch((err: unknown) => {
        if (!mounted.current || requestId.current !== id) return;
        const failure = toAppError(err);
        setHits([]);
        setError(failure);
        setOffline(isOfflineError(failure));
        setSearched(true);
      })
      .finally(() => {
        if (mounted.current && requestId.current === id) setSearching(false);
      });
    // retryNonce re-fires the same query after an error without new typing.
  }, [client, debouncedQuery, kind, gameVersion, sort, retryNonce]);

  const select = useCallback(
    async (hit: ModrinthSearchHit | null) => {
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
        if (!mounted.current) return;
        setProject(loadedProject);
        setVersions(loadedVersions);
        const preferred = preferModrinthVersion(
          loadedVersions,
          kind === 'datapack' ? targets?.world_version ?? null : (targets?.preferred_launcher ?? null),
        );
        setSelectedVersionId(preferred?.id ?? loadedVersions[0]?.id ?? null);
      } catch (err) {
        if (!mounted.current) return;
        setDetailError(toAppError(err));
      } finally {
        if (mounted.current) setDetailLoading(false);
      }
    },
    [client, kind, targets],
  );

  const setKindAndClear = useCallback((next: ModrinthKind) => {
    setKind(next);
    setSelected(null);
    setProject(null);
    setVersions([]);
    setInstallResult(null);
    setInstallError(null);
  }, []);

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
    setSelected(null);
    setProject(null);
    setVersions([]);
    setDetailError(null);
    setSelectedVersionId(null);
    setInstallResult(null);
    setInstallError(null);
  }, []);

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
