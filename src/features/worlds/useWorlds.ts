import { useCallback, useEffect, useRef, useState } from 'react';
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

/** True when the pack's declared range accepts the world's data format. A
 *  pack with no range is treated as unknown, never as a warning. */
export function isCompatible(range: FormatRange | null, format: FormatVersion | null): boolean {
  if (!range || !format) return true;
  const below = (a: FormatVersion, b: FormatVersion) =>
    a.major < b.major || (a.major === b.major && a.minor <= b.minor);
  if (range.min && !below(range.min, format)) return false;
  if (range.max && !below(format, range.max)) return false;
  return true;
}

/** The warning text for an incompatible pack, or null when it fits. */
export function compatibilityWarning(
  pack: Pack,
  format: FormatVersion | null,
  worldName: string | null,
): string | null {
  if (!pack.format_range || !format || !worldName) return null;
  if (isCompatible(pack.format_range, format)) return null;
  const packLabel = pack.game_label ?? 'an unknown range';
  return `Built for ${packLabel}, but ${worldName} needs data format ${format.major}.${format.minor}. It may not load, but you can still enable it.`;
}

/**
 * Loads the world's list and the selected world's datapacks, and keeps both
 * fresh after every mutation.
 */
export function useWorlds(client: WorldsClient): WorldsState {
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
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
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
    void reload();
  }, [reload]);

  const selected = worlds.find((world) => world.dir_name === selectedName) ?? null;

  const refreshListing = useCallback(async () => {
    if (!selectedName) {
      setPacks([]);
      setStack(null);
      return;
    }
    setPacksLoading(true);
    setStackLoading(true);
    setPacksError(null);
    setStackError(null);
    try {
      const listing = await client.listDatapacks(selectedName);
      if (!mounted.current) return;
      setPacks(listing.packs.packs);
      setStack(listing.stack);
    } catch (err) {
      if (!mounted.current) return;
      const appError = toAppError(err);
      setPacks([]);
      setStack(null);
      setPacksError(appError);
      setStackError(appError);
    } finally {
      if (mounted.current) {
        setPacksLoading(false);
        setStackLoading(false);
      }
    }
  }, [client, selectedName]);

  useEffect(() => {
    void refreshListing();
  }, [refreshListing]);

  const select = useCallback((dirName: string) => {
    setActionError(null);
    setSelectedName(dirName);
  }, []);

  const toggleDatapack = useCallback(
    async (filename: string, enabled: boolean) => {
      if (!selectedName) return;
      setBusy(true);
      setActionError(null);
      try {
        await refreshListing();
        const next = await client.setDatapackEnabled(selectedName, filename, enabled);
        if (mounted.current) setStack(next);
      } catch (err) {
        if (mounted.current) setActionError(toAppError(err));
        throw err;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, selectedName, refreshListing],
  );

  const reorderDatapacks = useCallback(
    async (order: string[]) => {
      if (!selectedName) return;
      setBusy(true);
      setActionError(null);
      try {
        const current = stack ?? (await client.listDatapacks(selectedName)).stack;
        const expected = dataSnapshotOf(current);
        await refreshListing();
        const next = await client.reorderDatapacks(selectedName, order, expected);
        if (mounted.current) setStack(next);
      } catch (err) {
        if (mounted.current) setActionError(toAppError(err));
        throw err;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, selectedName, stack, refreshListing],
  );

  const importDatapack = useCallback(
    async (world: string, path: string, choice: ImportChoice = 'default') => {
      setBusy(true);
      try {
        const outcome = await client.importDatapack(world, path, choice);
        if (outcome.status === 'imported' || outcome.status === 'locked') {
          await refreshListing();
        }
        return outcome;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, refreshListing],
  );

  const deleteDatapack = useCallback(
    async (path: string): Promise<DatapackDeleteOutcomeSummary> => {
      if (!selectedName) return { removedReferences: false };
      setBusy(true);
      setActionError(null);
      try {
        const outcome = await client.deleteDatapack(selectedName, path);
        await refreshListing();
        return { removedReferences: outcome.removed_references };
      } catch (err) {
        if (mounted.current) setActionError(toAppError(err));
        throw err;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, selectedName, refreshListing],
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
    busy,
    actionError,
    reload,
    refreshListing,
    toggleDatapack,
    reorderDatapacks,
    importDatapack,
    deleteDatapack,
    clearActionError,
  };
}
