import { useCallback, useEffect, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type { BackupInfo, GameRunning, IpcError, ResourceStack } from '../../lib/types';

interface BackupsPanelProps {
  client: PackClient;
  /** When the parent already tracks the game guard, it decides the block. */
  gameRunning?: GameRunning | null;
  onRestored?: (stack: ResourceStack) => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KB`;
}

/** Every `options.txt` write leaves a timestamped backup; this lists them and
 *  restores one. Restore is blocked while the game is running. */
export default function BackupsPanel({ client, gameRunning, onRestored }: BackupsPanelProps) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [ownRunning, setOwnRunning] = useState<GameRunning | null>(null);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const running = gameRunning !== undefined ? gameRunning : ownRunning;

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      if (gameRunning === undefined) {
        const stack = await client.getResourceStack();
        setOwnRunning(stack.running);
      }
      setBackups(await client.listOptionsBackups());
    } catch (err) {
      setBackups([]);
      setError(toAppError(err));
    } finally {
      setLoading(false);
    }
  }, [client, gameRunning]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleRestore = async (file: string) => {
    setRestoring(file);
    setStatus(null);
    setError(null);
    try {
      const before = await client.getResourceStack();
      if (before.running) {
        setOwnRunning(before.running);
        throw new Error('Minecraft is running; close it before restoring a backup');
      }
      const refreshed = await client.restoreOptionsBackup(file);
      onRestored?.(refreshed);
      setStatus(`Restored ${file}. The active stack now has ${refreshed.entries.length} entries.`);
      setBackups(await client.listOptionsBackups());
    } catch (err) {
      setError(toAppError(err));
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
                    disabled={running !== null || restoring !== null}
                    title={running ? 'Close Minecraft before restoring' : undefined}
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
