import { useEffect, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type { BackupInfo, GameRunning, IpcError, ResourceStack } from '../../lib/types';

interface BackupsPanelProps {
  client: PackClient;
  /** Changes here must reload the list; a game-running change must not. */
  activeGameDir?: string | null;
  /** When the parent already tracks the game guard, it decides the block. */
  gameRunning?: GameRunning | null;
  /** A failed app-level guard check: unknown, so restores stay disabled. */
  gameGuardUnknown?: boolean;
  /** Ask the app-level poll to re-check after a `game_running` error. */
  refreshGameRunning?: () => void | Promise<void>;
  onRestored?: (stack: ResourceStack) => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KB`;
}

/** Every `options.txt` write leaves a timestamped backup; this lists them and
 *  restores one. The backend blocks restore while the game is running; this
 *  panel only mirrors that state so the button explains itself. */
export default function BackupsPanel({
  client,
  activeGameDir,
  gameRunning,
  gameGuardUnknown = false,
  refreshGameRunning = () => {},
  onRestored,
}: BackupsPanelProps) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [ownRunning, setOwnRunning] = useState<GameRunning | null>(null);
  const [ownUnknown, setOwnUnknown] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const running = gameRunning !== undefined ? gameRunning : ownRunning;
  const guardUnknown =
    gameGuardUnknown || (gameRunning === undefined && ownUnknown);
  const blocked = running !== null || guardUnknown;

  // A changed game dir replaces the whole table. The five-second running poll
  // object identity is deliberately not a dependency: it would flash the list.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setStatus(null);
    client
      .listOptionsBackups()
      .then((next) => {
        if (!cancelled) setBackups(next);
      })
      .catch((err) => {
        if (cancelled) return;
        setBackups([]);
        setError(toAppError(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client, activeGameDir]);

  // Only used when the parent does not track the guard at all.
  useEffect(() => {
    if (gameRunning !== undefined) return undefined;
    let cancelled = false;
    client
      .getResourceStack()
      .then((stack) => {
        if (cancelled) return;
        setOwnRunning(stack.running);
        setOwnUnknown(false);
      })
      .catch(() => {
        if (cancelled) return;
        // Keep the last known running value; a failed check is unknown.
        setOwnUnknown(true);
      });
    return () => {
      cancelled = true;
    };
  }, [client, activeGameDir, gameRunning]);

  const handleRestore = async (file: string) => {
    setRestoring(file);
    setStatus(null);
    setError(null);
    try {
      // The backend guards the game and can recreate a missing options.txt, so
      // there is no client-side pre-check to block the exact recovery path.
      const refreshed = await client.restoreOptionsBackup(file);
      onRestored?.(refreshed);
      setStatus(`Restored ${file}. The active stack now has ${refreshed.entries.length} entries.`);
      setBackups(await client.listOptionsBackups());
    } catch (err) {
      const appError = toAppError(err);
      setError(appError);
      if (appError.code === 'game_running') void refreshGameRunning();
    } finally {
      setRestoring(null);
    }
  };

  return (
    <section className="panel" aria-label="Options backups">
      <h2>Backups</h2>
      <p className="panel-note">
        Every options.txt write leaves a timestamped copy here. Restoring writes
        the copy back and keeps the current file as a new backup.
      </p>

      {error ? (
        error.code === 'no_active_game_dir' ? (
          <p className="empty-note">Pick a game dir first to see its backups.</p>
        ) : (
          <p className="error" role="alert">
            {error.message}
          </p>
        )
      ) : null}
      {status ? (
        <p className="ok" role="status">
          {status}
        </p>
      ) : null}

      {loading ? (
        <p className="empty-note">Loading backups…</p>
      ) : !error && backups.length === 0 ? (
        <p className="empty-note">No backups yet. They appear after the first write.</p>
      ) : backups.length > 0 ? (
        <table className="backups-table">
          <thead>
            <tr>
              <th scope="col">Date</th>
              <th scope="col">File</th>
              <th scope="col">Size</th>
              <th scope="col">
                <span className="visually-hidden">Restore</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {backups.map((backup) => (
              <tr key={backup.file_name}>
                <td>{backup.display}</td>
                <td className="backups-name">{backup.file_name}</td>
                <td>{formatBytes(backup.bytes)}</td>
                <td>
                  <button
                    type="button"
                    className="button"
                    disabled={blocked || restoring !== null}
                    title={
                      running
                        ? 'Close Minecraft before restoring'
                        : guardUnknown
                          ? 'Mine Utils could not check whether Minecraft is running'
                          : undefined
                    }
                    onClick={() => void handleRestore(backup.file_name)}
                  >
                    {restoring === backup.file_name ? 'Restoring…' : 'Restore'}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  );
}
