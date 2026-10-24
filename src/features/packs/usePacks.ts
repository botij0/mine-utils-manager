import { useCallback, useEffect, useRef, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type { ImportChoice, ImportOutcome, IpcError, Pack } from '../../lib/types';

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
  reload: () => Promise<void>;
  importPack: (path: string, choice?: ImportChoice) => Promise<ImportOutcome>;
  deletePack: (path: string) => Promise<void>;
}

/**
 * Loads the active game dir's pack list and keeps it fresh after imports and
 * deletes. The page owns filters and selection, since those are view state.
 */
export function usePacks(client: PackClient): PacksState {
  const [packs, setPacks] = useState<Pack[]>([]);
  const [stats, setStats] = useState<PackStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);
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

  useEffect(() => {
    void reload();
  }, [reload]);

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
        await client.deleteResourcePack(path);
        await reload();
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [client, reload],
  );

  return { packs, stats, loading, busy, error, reload, importPack, deletePack };
}
