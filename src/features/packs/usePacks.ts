import { useCallback, useEffect, useRef, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
  BackupInfo,
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
  stats: PackStats | null;
  loading: boolean;
  busy: boolean;
  error: IpcError | null;
  stack: ResourceStack | null;
  stackLoading: boolean;
  backups: BackupInfo[];
  reload: () => Promise<void>;
  refreshStack: () => Promise<void>;
  refreshBackups: () => Promise<void>;
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
 * Loads the active game dir's pack list and resource stack, and keeps both
 * fresh after every mutation. Each write rechecks the stack first, so a game
 * that started in the meantime blocks the write before it reaches the disk.
 */
export function usePacks(client: PackClient): PacksState {
  const [packs, setPacks] = useState<Pack[]>([]);
  const [stats, setStats] = useState<PackStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);
  const [stack, setStack] = useState<ResourceStack | null>(null);
  const [stackLoading, setStackLoading] = useState(true);
  const [backups, setBackups] = useState<BackupInfo[]>([]);
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
      const listing = await client.listResourcePacks();
      if (!mounted.current) return;
      setPacks(listing.packs);
      setStats({ bytesRead: listing.bytes_read, elapsedMs: listing.elapsed_ms });
      setError(null);
    } catch (err) {
      if (!mounted.current) return;
      setError(toAppError(err));
      setPacks([]);
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, [client]);

  const refreshStack = useCallback(async () => {
    setStackLoading(true);
    try {
      const next = await client.getResourceStack();
      if (mounted.current) setStack(next);
    } catch (err) {
      if (!mounted.current) return;
      setError(toAppError(err));
      setStack(null);
    } finally {
      if (mounted.current) setStackLoading(false);
    }
  }, [client]);

  const refreshBackups = useCallback(async () => {
    try {
      setBackups(await client.listOptionsBackups());
    } catch {
      if (mounted.current) setBackups([]);
    }
  }, [client]);

  useEffect(() => {
    void reload();
    void refreshStack();
    void refreshBackups();
  }, [reload, refreshStack, refreshBackups]);

  const importPack = useCallback(
    async (path: string, choice: ImportChoice = 'default') => {
      setBusy(true);
      try {
        const outcome = await client.importResourcePack(path, choice);
        if (outcome.status === 'imported') await reload();
        return outcome;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, reload],
  );

  const deletePack = useCallback(
    async (path: string) => {
      setBusy(true);
      try {
        await refreshStack();
        await client.deleteResourcePack(path);
        await reload();
        await refreshStack();
        await refreshBackups();
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, reload, refreshStack, refreshBackups],
  );

  const togglePack = useCallback(
    async (filename: string, enabled: boolean) => {
      setBusy(true);
      try {
        await refreshStack();
        await client.setResourcePackEnabled(filename, enabled);
        await refreshStack();
        await refreshBackups();
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, refreshStack, refreshBackups],
  );

  const reorderStack = useCallback(
    async (order: string[]) => {
      setBusy(true);
      try {
        // The snapshot the user acted on, so the backend can refuse a write
        // against a file set that changed since the screen was drawn.
        const expected = snapshotOf(stack ?? (await client.getResourceStack()));
        await refreshStack();
        await client.reorderResourceStack(order, expected);
        await refreshStack();
        await refreshBackups();
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, stack, refreshStack, refreshBackups],
  );

  return {
    packs,
    stats,
    loading,
    busy,
    error,
    stack,
    stackLoading,
    backups,
    reload,
    refreshStack,
    refreshBackups,
    importPack,
    deletePack,
    togglePack,
    reorderStack,
  };
}
