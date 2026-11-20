import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
  HoldingEntry,
  ImportChoice,
  ImportOutcome,
  IpcError,
  Pack,
  ResourceStack,
  StackSnapshot,
} from '../../lib/types';

export interface PackStats {
  bytesRead: number;
  elapsedMs: number;
}

export interface PacksState {
  packs: Pack[];
  holdings: HoldingEntry[];
  restoreHolding: (holdingName: string) => Promise<void>;
  stats: PackStats | null;
  loading: boolean;
  busy: boolean;
  /** The pack-list failure. A stack failure never lands here. */
  error: IpcError | null;
  stack: ResourceStack | null;
  stackLoading: boolean;
  /** The stack-read failure. A pack-list failure never lands here. */
  stackError: IpcError | null;
  /** True when this call applied the pack list. False when a newer read won or the read failed. */
  reload: () => Promise<boolean>;
  /** True when this call applied a stack. False when a newer read won or the read failed. */
  refreshStack: () => Promise<boolean>;
  importPack: (path: string, choice?: ImportChoice) => Promise<ImportOutcome>;
  deletePack: (path: string) => Promise<void>;
  togglePack: (filename: string, enabled: boolean) => Promise<void>;
  reorderStack: (order: string[]) => Promise<void>;
}

/** What the UI rendered, sent along so a write against a changed file set is
 *  refused instead of clobbering content the user has not seen. */
export function snapshotOf(stack: ResourceStack): StackSnapshot {
  return {
    resource: stack.entries.map((entry) => entry.value),
    incompatible: [...stack.incompatible],
  };
}

/**
 * Loads the active game dir's pack list and resource stack. Toggle and reorder
 * render the stack their write returns instead of re-reading it. The backend
 * rechecks the game guard before each write reaches the disk.
 */
export function usePacks(
  client: PackClient,
  active = true,
  reloadSignal = '',
  gameDir: string | null = null,
): PacksState {
  const [holdings, setHoldings] = useState<HoldingEntry[]>([]);
  const [packs, setPacks] = useState<Pack[]>([]);
  const [stats, setStats] = useState<PackStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);
  const [stack, setStack] = useState<ResourceStack | null>(null);
  const [stackLoading, setStackLoading] = useState(true);
  const [stackError, setStackError] = useState<IpcError | null>(null);
  const mounted = useRef(true);
  const packRequest = useRef(0);
  const stackRequest = useRef(0);
  // Set when both reads apply. A failure leaves it unset so the next activation retries.
  const loadedSignal = useRef<string | null>(null);
  // Set while this signal's reads have not settled, so another activation does not start a second pair.
  const inflightSignal = useRef<string | null>(null);
  // Bumped on a game-dir change so a late completion cannot clear the replacement attempt.
  const loadAttempt = useRef(0);

  useLayoutEffect(() => {
    packRequest.current += 1;
    stackRequest.current += 1;
    loadAttempt.current += 1;
    loadedSignal.current = null;
    inflightSignal.current = null;
    setPacks([]);
    setHoldings([]);
    setStats(null);
    setStack(null);
    setError(null);
    setStackError(null);
    setLoading(true);
    setStackLoading(true);
  }, [gameDir]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const reload = useCallback(async (): Promise<boolean> => {
    const request = ++packRequest.current;
    setLoading(true);
    try {
      const listing = await client.listResourcePacks();
      if (!mounted.current || request !== packRequest.current) return false;
      setPacks(listing.packs);
      setHoldings(listing.holding_entries);
      setStats({ bytesRead: listing.bytes_read, elapsedMs: listing.elapsed_ms });
      setError(null);
      return true;
    } catch (err) {
      if (!mounted.current || request !== packRequest.current) return false;
      setError(toAppError(err));
      setPacks([]);
      setHoldings([]);
      return false;
    } finally {
      if (mounted.current && request === packRequest.current) setLoading(false);
    }
  }, [client]);

  const refreshStack = useCallback(async (): Promise<boolean> => {
    const request = ++stackRequest.current;
    setStackLoading(true);
    setStackError(null);
    try {
      const next = await client.getResourceStack();
      if (!mounted.current || request !== stackRequest.current) return false;
      setStack(next);
      setStackError(null);
      return true;
    } catch (err) {
      // Keep the last stack on screen. A failed re-read is not a reason to
      // replace a usable view with an error card.
      if (!mounted.current || request !== stackRequest.current) return false;
      setStackError(toAppError(err));
      return false;
    } finally {
      if (mounted.current && request === stackRequest.current) setStackLoading(false);
    }
  }, [client]);

  useEffect(() => {
    if (!active || loadedSignal.current === reloadSignal) return;
    if (inflightSignal.current === reloadSignal) return;
    const signal = reloadSignal;
    const attempt = ++loadAttempt.current;
    inflightSignal.current = signal;
    void Promise.all([reload(), refreshStack()]).then(([listed, stacked]) => {
      if (loadAttempt.current !== attempt) return;
      inflightSignal.current = null;
      if (listed && stacked) loadedSignal.current = signal;
    });
  }, [reload, refreshStack, active, reloadSignal, gameDir]);

  const importPack = useCallback(
    async (path: string, choice: ImportChoice = 'default') => {
      setBusy(true);
      try {
        const outcome = await client.importResourcePack(path, choice);
        if (outcome.status === 'imported') {
          await reload();
          await refreshStack();
        }
        return outcome;
      } catch (err) {
        await Promise.all([reload(), refreshStack()]);
        throw err;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, reload, refreshStack],
  );

  const deletePack = useCallback(
    async (path: string) => {
      setBusy(true);
      try {
        // The backend handles a missing options.txt without a pre-read, so a
        // delete still works when the stack cannot be read.
        await client.deleteResourcePack(path);
        await reload();
        await refreshStack();
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, reload, refreshStack],
  );

  const restoreHolding = useCallback(async (holdingName: string) => {
    setBusy(true);
    try {
      await client.restoreResourceHolding(holdingName);
    } finally {
      await Promise.all([reload(), refreshStack()]);
      if (mounted.current) setBusy(false);
    }
  }, [client, reload, refreshStack]);

  const togglePack = useCallback(
    async (filename: string, enabled: boolean) => {
      setBusy(true);
      try {
        const next = await client.setResourcePackEnabled(filename, enabled);
        if (mounted.current) {
          setStack(next);
          setStackError(null);
        }
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client],
  );

  const reorderStack = useCallback(
    async (order: string[]) => {
      setBusy(true);
      try {
        // The snapshot is the stack on screen, not a fresh read. If the file
        // set changed since that render, the backend refuses the write.
        const expected = snapshotOf(stack ?? (await client.getResourceStack()));
        const next = await client.reorderResourceStack(order, expected);
        if (mounted.current) {
          setStack(next);
          setStackError(null);
        }
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, stack],
  );

  return {
    holdings,
    restoreHolding,
    packs,
    stats,
    loading,
    busy,
    error,
    stack,
    stackLoading,
    stackError,
    reload,
    refreshStack,
    importPack,
    deletePack,
    togglePack,
    reorderStack,
  };
}
