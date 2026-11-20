import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { formatVersionLabel } from '../../lib/format';
import type { WorldsClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
  HoldingEntry,
  DataSnapshot,
  DataStack,
  DatapackImportOutcome,
  FormatRange,
  FormatVersion,
  IpcError,
  ImportChoice,
  Pack,
  World,
} from '../../lib/types';

export interface WorldsState {
  worlds: World[];
  loading: boolean;
  error: IpcError | null;
  selected: World | null;
  select: (dirName: string) => void;
  packs: Pack[];
  holdings: HoldingEntry[];
  restoreHolding: (holdingName: string) => Promise<void>;
  packsLoading: boolean;
  packsError: IpcError | null;
  stack: DataStack | null;
  stackLoading: boolean;
  stackError: IpcError | null;
  /** The world the visible `packs`/`stack` were listed for, or null while a
   *  fresh selection has no listing yet. While it differs from the selection,
   *  every write control stays disabled. */
  listingWorld: string | null;
  busy: boolean;
  actionError: IpcError | null;
  clearStaleActionError: () => void;
  reload: () => Promise<void>;
  /** True when this call applied a listing. False when a newer read won or the read failed. */
  refreshListing: () => Promise<boolean>;
  toggleDatapack: (filename: string, enabled: boolean) => Promise<void>;
  reorderDatapacks: (order: string[]) => Promise<void>;
  importDatapack: (
    world: string,
    path: string,
    choice?: ImportChoice,
  ) => Promise<DatapackImportOutcome>;
  deleteDatapack: (path: string) => Promise<DatapackDeleteOutcomeSummary>;
  removeDatapackReference: (filename: string) => Promise<void>;
  clearActionError: () => void;
}

export interface DatapackDeleteOutcomeSummary {
  removedReferences: boolean;
}

/** What the UI rendered, so a write against a changed file set is refused. */
export function dataSnapshotOf(stack: DataStack): DataSnapshot {
  return {
    enabled: stack.enabled.map((entry) => entry.value),
    disabled: stack.disabled.map((entry) => entry.value),
  };
}

export type Compatibility = 'compatible' | 'incompatible' | 'unknown';

/** Whether the pack's declared range accepts the world's data format. A missing
 *  range or a missing world format is `unknown`, never `compatible`. */
export function checkCompatibility(
  range: FormatRange | null,
  format: FormatVersion | null,
): Compatibility {
  if (!range || !format) return 'unknown';
  const below = (a: FormatVersion, b: FormatVersion) =>
    a.major < b.major || (a.major === b.major && a.minor <= b.minor);
  if (range.min && !below(range.min, format)) return 'incompatible';
  if (range.max && !below(format, range.max)) return 'incompatible';
  return 'compatible';
}

/** The warning text for an incompatible pack, or null when it fits. */
export function compatibilityWarning(
  pack: Pack,
  format: FormatVersion | null,
  worldName: string | null,
): string | null {
  if (!pack.format_range || !format || !worldName) return null;
  if (checkCompatibility(pack.format_range, format) !== 'incompatible') return null;
  const packLabel = pack.game_label ?? 'an unknown range';
  return `Built for ${packLabel}, but ${worldName} needs data format ${formatVersionLabel(format)}. It may not load, but you can still enable it.`;
}

/**
 * Loads the world's list and the selected world's datapacks, and keeps both
 * fresh after every mutation.
 */
export function useWorlds(
  client: WorldsClient,
  active = true,
  reloadSignal = '',
  gameDir: string | null = null,
): WorldsState {
  const [worlds, setWorlds] = useState<World[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [holdings, setHoldings] = useState<HoldingEntry[]>([]);
  const [packs, setPacks] = useState<Pack[]>([]);
  const [packsLoading, setPacksLoading] = useState(false);
  const [packsError, setPacksError] = useState<IpcError | null>(null);
  const [stack, setStack] = useState<DataStack | null>(null);
  const [stackLoading, setStackLoading] = useState(false);
  const [stackError, setStackError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<IpcError | null>(null);
  // The world the visible `packs`/`stack` were listed for. While it differs
  // from the selection (a switch is in flight), every write control stays
  // disabled so no action can address a stale filename.
  const [listingWorld, setListingWorld] = useState<string | null>(null);
  const mounted = useRef(true);
  // Monotonic id of the latest selection or mutation. A response from an
  // older request, or for a world that is no longer selected, is ignored.
  // Listings share it but never bump it: a refresh must not cancel an
  // in-flight write, it only has to lose to a newer selection or mutation.
  const requestId = useRef(0);
  const selectedRef = useRef<string | null>(null);
  const lastWorldWrite = useRef<Promise<void>>(Promise.resolve());
  const directoryGeneration = useRef(0);
  selectedRef.current = selectedName;
  const worldsRequest = useRef(0);
  const listingRequest = useRef(0);
  // Set when the world list applies. A failure leaves it unset so the next activation retries.
  const loadedSignal = useRef<string | null>(null);
  // Set while this signal's world list has not settled, so another activation does not start a second scan.
  const inflightSignal = useRef<string | null>(null);
  const loadedListing = useRef<string | null>(null);

  useLayoutEffect(() => {
    directoryGeneration.current += 1;
    worldsRequest.current += 1;
    listingRequest.current += 1;
    requestId.current += 1;
    loadedSignal.current = null;
    inflightSignal.current = null;
    loadedListing.current = null;
    selectedRef.current = null;
    setSelectedName(null);
    setWorlds([]);
    setPacks([]);
    setHoldings([]);
    setStack(null);
    setListingWorld(null);
    setError(null);
    setPacksError(null);
    setStackError(null);
    setActionError(null);
    setBusy(false);
    setLoading(true);
    setPacksLoading(false);
    setStackLoading(false);
  }, [gameDir]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const isCurrentRequest = useCallback(
    (world: string, request: number) =>
      mounted.current && selectedRef.current === world && request === requestId.current,
    [],
  );

  const beginAction = useCallback(() => {
    const request = (requestId.current += 1);
    setBusy(true);
    setActionError(null);
    return request;
  }, []);

  const reload = useCallback(async () => {
    const request = ++worldsRequest.current;
    const signal = reloadSignal;
    inflightSignal.current = signal;
    setLoading(true);
    try {
      const [next, config] = await Promise.all([client.listWorlds(), client.getConfig()]);
      if (!mounted.current || request !== worldsRequest.current) return;
      if (gameDir !== null && config.active_game_dir !== gameDir) {
        loadedSignal.current = null;
        return;
      }
      loadedSignal.current = signal;
      setWorlds(next);
      setError(null);
      setSelectedName((prev) => {
        if (prev !== null && next.some((world) => world.dir_name === prev)) return prev;
        return next.find((world) => world.path === config.last_world)?.dir_name
          ?? next[0]?.dir_name ?? null;
      });
    } catch (err) {
      if (!mounted.current || request !== worldsRequest.current) return;
      loadedSignal.current = null;
      setError(toAppError(err));
      setWorlds([]);
    } finally {
      if (request === worldsRequest.current) {
        inflightSignal.current = null;
        if (mounted.current) setLoading(false);
      }
    }
  }, [client, gameDir, reloadSignal]);

  useEffect(() => {
    if (!active || loadedSignal.current === reloadSignal) return;
    if (inflightSignal.current === reloadSignal) return;
    void reload();
  }, [reload, active, reloadSignal, gameDir]);

  const selected = worlds.find((world) => world.dir_name === selectedName) ?? null;

  const refreshListing = useCallback(async (): Promise<boolean> => {
    if (!selectedName || selectedRef.current !== selectedName) {
      setPacks([]);
      setHoldings([]);
      setStack(null);
      setListingWorld(null);
      return false;
    }
    // Capture the selection and the latest request: a late listing for a
    // previous world, or one overtaken by a mutation, never repopulates the
    // new selection.
    const world = selectedName;
    const listingId = ++listingRequest.current;
    loadedListing.current = null;
    const request = requestId.current;
    setPacksLoading(true);
    setStackLoading(true);
    setPacksError(null);
    setStackError(null);
    try {
      const listing = await client.listDatapacks(world);
      if (!isCurrentRequest(world, request) || listingId !== listingRequest.current) return false;
      loadedListing.current = JSON.stringify([reloadSignal, world]);
      setPacks(listing.packs.packs);
      setHoldings(listing.packs.holding_entries);
      setStack(listing.stack);
      setListingWorld(world);
      return true;
    } catch (err) {
      if (!isCurrentRequest(world, request) || listingId !== listingRequest.current) return false;
      const appError = toAppError(err);
      setPacks([]);
      setHoldings([]);
      setStack(null);
      setListingWorld(world);
      setPacksError(appError);
      setStackError(appError);
      return false;
    } finally {
      if (isCurrentRequest(world, request) && listingId === listingRequest.current) {
        setPacksLoading(false);
        setStackLoading(false);
      }
    }
  }, [client, selectedName, isCurrentRequest, reloadSignal]);

  useEffect(() => {
    if (!active || !selectedName || selectedRef.current !== selectedName) return;
    const key = JSON.stringify([reloadSignal, selectedName]);
    if (loadedListing.current === key) return;
    void refreshListing();
  }, [refreshListing, active, reloadSignal, selectedName, gameDir]);

  const select = useCallback((dirName: string) => {
    // Invalidate every in-flight request, drop the old world's rows so the
    // grid shows the new world's loading state (never stale actionable
    // cards), and release `busy`: the guarded `finally` blocks below skip a
    // stale request, so without this a switch mid-mutation would strand the
    // new world with every write control permanently disabled.
    requestId.current += 1;
    setActionError(null);
    setPacks([]);
    setHoldings([]);
    setStack(null);
    setListingWorld(null);
    setPacksError(null);
    setStackError(null);
    setBusy(false);
    setPacksLoading(true);
    setStackLoading(true);
    setSelectedName(dirName);
    const world = worlds.find((item) => item.dir_name === dirName);
    if (world) {
      const generation = directoryGeneration.current;
      lastWorldWrite.current = lastWorldWrite.current
        .then(() => {
          if (generation === directoryGeneration.current) return client.setLastWorld(world.path);
        })
        .catch((err: unknown) => {
          if (mounted.current && generation === directoryGeneration.current) {
            setActionError(toAppError(err));
          }
        });
    }
  }, [client, worlds]);

  const toggleDatapack = useCallback(
    async (filename: string, enabled: boolean) => {
      if (!selectedName) return;
      const world = selectedName;
      const request = beginAction();
      try {
        // No pre-toggle fetch: the backend re-reads `level.dat` anyway, and
        // the extra round-trip only flashes the loading state.
        const next = await client.setDatapackEnabled(world, filename, enabled);
        if (!isCurrentRequest(world, request)) return;
        setStack(next);
      } catch (err) {
        if (!isCurrentRequest(world, request)) return;
        setActionError(toAppError(err));
        throw err;
      } finally {
        if (isCurrentRequest(world, request)) setBusy(false);
      }
    },
    [client, selectedName, beginAction, isCurrentRequest],
  );

  const reorderDatapacks = useCallback(
    async (order: string[]) => {
      if (!selectedName || !stack) return;
      const world = selectedName;
      const request = beginAction();
      try {
        // `order` came from this rendered stack, so the guard checks the file set
        // the user saw; a datapack added since then yields stale_stack, not a raw order error.
        const next = await client.reorderDatapacks(world, order, dataSnapshotOf(stack));
        if (!isCurrentRequest(world, request)) return;
        setStack(next);
      } catch (err) {
        if (!isCurrentRequest(world, request)) return;
        setActionError(toAppError(err));
        throw err;
      } finally {
        if (isCurrentRequest(world, request)) setBusy(false);
      }
    },
    [client, selectedName, stack, beginAction, isCurrentRequest],
  );

  const importDatapack = useCallback(
    async (world: string, path: string, choice: ImportChoice = 'default') => {
      const request = beginAction();
      try {
        const outcome = await client.importDatapack(world, path, choice);
        if (!isCurrentRequest(world, request)) return outcome;
        if (outcome.status === 'imported' || outcome.status === 'locked') {
          await refreshListing();
        }
        return outcome;
      } catch (err) {
        // The new pack can already be in datapacks when this rejects.
        if (isCurrentRequest(world, request)) await refreshListing();
        throw err;
      } finally {
        if (mounted.current && request === requestId.current) {
          setBusy(false);
        }
      }
    },
    [client, refreshListing, beginAction, isCurrentRequest],
  );

  const deleteDatapack = useCallback(
    async (path: string): Promise<DatapackDeleteOutcomeSummary> => {
      if (!selectedName) return { removedReferences: false };
      const world = selectedName;
      const request = beginAction();
      try {
        const outcome = await client.deleteDatapack(world, path);
        if (!isCurrentRequest(world, request)) {
          return { removedReferences: outcome.removed_references };
        }
        await refreshListing();
        return { removedReferences: outcome.removed_references };
      } catch (err) {
        if (!isCurrentRequest(world, request)) throw err;
        setActionError(toAppError(err));
        // Disable can already be in level.dat when trash or a later check rejects.
        await refreshListing();
        throw err;
      } finally {
        if (isCurrentRequest(world, request)) setBusy(false);
      }
    },
    [client, selectedName, refreshListing, beginAction, isCurrentRequest],
  );

  const removeDatapackReference = useCallback(
    async (filename: string) => {
      if (!selectedName) return;
      const world = selectedName;
      const request = beginAction();
      try {
        const next = await client.removeDatapackReference(world, filename);
        if (!isCurrentRequest(world, request)) return;
        setStack(next);
        await refreshListing();
      } catch (err) {
        if (!isCurrentRequest(world, request)) return;
        setActionError(toAppError(err));
        throw err;
      } finally {
        if (isCurrentRequest(world, request)) setBusy(false);
      }
    },
    [client, selectedName, refreshListing, beginAction, isCurrentRequest],
  );

  const restoreHolding = useCallback(async (holdingName: string) => {
    if (!selectedName) return;
    const world = selectedName;
    const request = beginAction();
    try {
      await client.restoreDatapackHolding(world, holdingName);
    } catch (err) {
      if (isCurrentRequest(world, request)) setActionError(toAppError(err));
      throw err;
    } finally {
      if (isCurrentRequest(world, request)) await refreshListing();
      if (isCurrentRequest(world, request)) setBusy(false);
    }
  }, [client, selectedName, beginAction, isCurrentRequest, refreshListing]);

  const clearActionError = useCallback(() => setActionError(null), []);

  const clearStaleActionError = useCallback(() => {
    setActionError((current) => (current?.code === 'stale_stack' ? null : current));
  }, []);

  return {
    holdings,
    restoreHolding,
    worlds,
    loading,
    error,
    selected,
    select,
    packs,
    packsLoading,
    packsError,
    stack,
    stackLoading,
    stackError,
    listingWorld,
    busy,
    actionError,
    clearStaleActionError,
    reload,
    refreshListing,
    toggleDatapack,
    reorderDatapacks,
    importDatapack,
    deleteDatapack,
    removeDatapackReference,
    clearActionError,
  };
}
