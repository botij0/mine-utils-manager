import { useEffect, useId, useState } from 'react';
import type { BackupsClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type {
  BackupInfo,
  GameRunning,
  IpcError,
  LevelBackupInfo,
  ResourceStack,
} from '../../lib/types';
import type { GuardPause } from '../packs/GameRunningBanner';

interface BackupsPanelProps {
  active?: boolean;
  client: BackupsClient;
  /** Changes here must reload the list; a game-running change must not. */
  activeGameDir?: string | null;
  gameRunning: GameRunning | null;
  guardPause?: GuardPause;
  /** Ask the app-level poll to re-check after a `game_running` error. */
  refreshGameRunning?: () => void | Promise<void>;
  onRestored?: (stack: ResourceStack) => void;
  onLevelRestored?: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KB`;
}

function restoreName(busy: boolean, display: string, world?: string): string {
  const verb = busy ? 'Restoring' : 'Restore';
  return world ? `${verb} ${display} for ${world}` : `${verb} ${display}`;
}

const RESTORE_IN_PROGRESS = 'A restore is in progress';

/** Every `options.txt` write leaves a timestamped backup; this lists them and
 *  restores one. The backend blocks restore while the game is running; this
 *  panel only mirrors that state so the button explains itself. */
export default function BackupsPanel({
  client,
  active = true,
  activeGameDir,
  gameRunning,
  guardPause = null,
  refreshGameRunning = () => {},
  onRestored,
  onLevelRestored,
}: BackupsPanelProps) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [levelGroups, setLevelGroups] = useState<
    { dirName: string; label: string; backups: LevelBackupInfo[] }[]
  >([]);
  const [levelLoading, setLevelLoading] = useState(true);
  const [levelError, setLevelError] = useState<IpcError | null>(null);
  const [levelRestoring, setLevelRestoring] = useState<string | null>(null);

  const blocked = gameRunning !== null || guardPause !== null;
  const gateReason = gameRunning
    ? 'Close Minecraft before restoring'
    : guardPause === 'unknown'
      ? 'Mine Utils could not check whether Minecraft is running'
      : guardPause === 'checking'
        ? 'Checking whether Minecraft is running'
        : undefined;
  const restoreInProgress = restoring !== null || levelRestoring !== null;
  const optionsReason = gateReason ?? (restoreInProgress ? RESTORE_IN_PROGRESS : undefined);
  const levelReason = gateReason ?? (restoreInProgress ? RESTORE_IN_PROGRESS : undefined);
  const reasonId = useId();
  const optionsReasonId = `${reasonId}-options`;
  const levelReasonId = `${reasonId}-level`;

  // A changed game dir replaces the whole table. The five-second running poll
  // object identity is deliberately not a dependency: it would flash the list.
  useEffect(() => {
    if (!active) return undefined;
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
  }, [client, activeGameDir, active]);

  // The `level.dat` backups of every world in one batch call, grouped by
  // world. Groups come from the backups themselves, so a corrupt `level.dat`
  // (the main restore case) or a deleted one still shows its backups, and a
  // world without `level.dat` is listed by its backups.
  useEffect(() => {
    if (!active) return undefined;
    let cancelled = false;
    setLevelLoading(true);
    setLevelError(null);
    client
      .listAllLevelBackups()
      .then((all) => {
        if (cancelled) return;
        const byWorld = new Map<string, LevelBackupInfo[]>();
        for (const backup of all) {
          const group = byWorld.get(backup.world);
          if (group) group.push(backup);
          else byWorld.set(backup.world, [backup]);
        }
        setLevelGroups(
          [...byWorld.entries()].map(([dirName, backups]) => ({
            dirName,
            label: dirName,
            backups,
          })),
        );
      })
      .catch((err) => {
        if (cancelled) return;
        setLevelGroups([]);
        setLevelError(toAppError(err));
      })
      .finally(() => {
        if (!cancelled) setLevelLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client, activeGameDir, active]);

  /** A refused restore has already written its pre-restore copy, so the list is re-read. */
  const reloadOptionsBackups = async () => {
    try {
      setBackups(await client.listOptionsBackups());
    } catch (err) {
      setError(toAppError(err));
    }
  };

  const reloadLevelBackups = async (dirName: string) => {
    try {
      const fresh = await client.listLevelBackups(dirName);
      setLevelGroups((groups) =>
        groups.map((group) => (group.dirName === dirName ? { ...group, backups: fresh } : group)),
      );
    } catch (err) {
      setLevelError(toAppError(err));
    }
  };

  const handleLevelRestore = async (dirName: string, label: string, file: string) => {
    const key = `${dirName}/${file}`;
    setLevelRestoring(key);
    setStatus(null);
    setLevelError(null);
    try {
      // The backend guards the game and recreates a missing level.dat, so
      // there is no client-side pre-check to block the recovery path.
      await client.restoreLevelBackup(dirName, file);
      onLevelRestored?.();
      setStatus(`Restored ${file} for ${label}.`);
      const groups = [...levelGroups];
      const group = groups.find((entry) => entry.dirName === dirName);
      if (group) group.backups = await client.listLevelBackups(dirName);
      setLevelGroups(groups);
    } catch (err) {
      const appError = toAppError(err);
      setLevelError(appError);
      if (appError.code === 'game_running') void refreshGameRunning();
      if (appError.code === 'stale_stack') await reloadLevelBackups(dirName);
    } finally {
      setLevelRestoring(null);
    }
  };

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
      if (appError.code === 'stale_stack') await reloadOptionsBackups();
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

      {optionsReason && !loading && backups.length > 0 ? (
        <p id={optionsReasonId} className="panel-note">
          {optionsReason}
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
                    disabled={blocked || restoreInProgress}
                    title={optionsReason}
                    aria-describedby={optionsReason ? optionsReasonId : undefined}
                    aria-label={restoreName(restoring === backup.file_name, backup.display)}
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

      <h2>World level.dat</h2>
      <p className="panel-note">
        Every level.dat write leaves a timestamped copy here, grouped by world.
        Restoring writes the copy back and keeps the current file as a new backup.
      </p>

      {levelError ? (
        <p className="error" role="alert">
          {levelError.code === 'world_mismatch'
            ? 'This backup belongs to a different world. Its seed does not match the current world.'
            : levelError.message}
        </p>
      ) : null}

      {levelReason && !levelLoading && levelGroups.length > 0 ? (
        <p id={levelReasonId} className="panel-note">
          {levelReason}
        </p>
      ) : null}
      {levelLoading ? (
        <p className="empty-note">Loading world backups…</p>
      ) : levelGroups.length === 0 ? (
        <p className="empty-note">No worlds with level.dat backups yet.</p>
      ) : (
        levelGroups.map((group) => (
          <section key={group.dirName} className="level-backup-group" aria-label={`${group.label} backups`}>
            <h3>{group.label}</h3>
            {group.backups.length === 0 ? (
              <p className="empty-note">No backups yet. They appear after the first write.</p>
            ) : (
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
                  {group.backups.map((backup) => {
                    const key = `${group.dirName}/${backup.file_name}`;
                    return (
                      <tr key={backup.file_name}>
                        <td>{backup.display}</td>
                        <td className="backups-name">{backup.file_name}</td>
                        <td>{formatBytes(backup.bytes)}</td>
                        <td>
                          <button
                            type="button"
                            className="button"
                            disabled={blocked || restoreInProgress}
                            title={levelReason}
                            aria-describedby={levelReason ? levelReasonId : undefined}
                            aria-label={restoreName(levelRestoring === key, backup.display, group.label)}
                            onClick={() => void handleLevelRestore(group.dirName, group.label, backup.file_name)}
                          >
                            {levelRestoring === key ? 'Restoring…' : 'Restore'}
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </section>
        ))
      )}
    </section>
  );
}
