import { useCallback, useEffect, useRef, useState } from 'react';
import { formatVersionLabel } from '../../lib/format';
import type { WorldsClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
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
  reload: () => Promise<void>;
  refreshListing: () => Promise<void>;
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
export function useWorlds(client: WorldsClient, active = true, reloadSignal = ''): WorldsState {
  const [worlds, setWorlds] = useState<World[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [selectedName, setSelectedName] = useState<string | null>(null);
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
  selectedRef.current = selectedName;

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
    setLoading(true);
    try {
      const next = await client.listWorlds();
      if (!mounted.current) return;
      setWorlds(next);
      setError(null);
      if (next.length > 0) {
        const first = next[0].dir_name;
        setSelectedName((prev) => {
          if (prev !== null && next.some((world) => world.dir_name === prev)) return prev;
          return first;
        });
      }
    } catch (err) {
      if (!mounted.current) return;
      setError(toAppError(err));
      setWorlds([]);
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, [client]);

  useEffect(() => {
    if (active) void reload();
  }, [reload, active, reloadSignal]);

  const selected = worlds.find((world) => world.dir_name === selectedName) ?? null;

  const refreshListing = useCallback(async () => {
    if (!selectedName) {
      setPacks([]);
      setStack(null);
      setListingWorld(null);
      return;
    }
    // Capture the selection and the latest request: a late listing for a
    // previous world, or one overtaken by a mutation, never repopulates the
    // new selection.
    const world = selectedName;
    const request = requestId.current;
    setPacksLoading(true);
    setStackLoading(true);
    setPacksError(null);
    setStackError(null);
    try {
      const listing = await client.listDatapacks(world);
      if (!isCurrentRequest(world, request)) return;
      setPacks(listing.packs.packs);
      setStack(listing.stack);
      setListingWorld(world);
    } catch (err) {
      if (!isCurrentRequest(world, request)) return;
      const appError = toAppError(err);
      setPacks([]);
      setStack(null);
      setListingWorld(world);
      setPacksError(appError);
      setStackError(appError);
    } finally {
      if (isCurrentRequest(world, request)) {
        setPacksLoading(false);
        setStackLoading(false);
      }
    }
  }, [client, selectedName, isCurrentRequest]);

  useEffect(() => {
    if (active) void refreshListing();
  }, [refreshListing, active, reloadSignal]);

  const select = useCallback((dirName: string) => {
    // Invalidate every in-flight request, drop the old world's rows so the
    // grid shows the new world's loading state (never stale actionable
    // cards), and release `busy`: the guarded `finally` blocks below skip a
    // stale request, so without this a switch mid-mutation would strand the
    // new world with every write control permanently disabled.
    requestId.current += 1;
    setActionError(null);
    setPacks([]);
    setStack(null);
    setListingWorld(null);
    setPacksError(null);
    setStackError(null);
    setBusy(false);
    setPacksLoading(true);
    setStackLoading(true);
    setSelectedName(dirName);
  }, []);

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

  const clearActionError = useCallback(() => setActionError(null), []);

  return {
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
