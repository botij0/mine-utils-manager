import { useCallback, useEffect, useState } from 'react';
import type { IpcClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import type { GameDirEntry, IpcError } from '../../lib/types';
import GameDirPicker from '../game-dirs/GameDirPicker';

interface OnboardingProps {
  client: Pick<IpcClient, 'listGameDirs' | 'addGameDir' | 'setActiveGameDir'>;
  /** Fires after an active dir is set, or when the operator skips. */
  onDone: () => void;
}

/**
 * First-run onboarding: shown when no active game dir is set. The operator
 * picks one or more detected dirs (or adds one), sets the active dir, and
 * learns the safety rules. Skipping leaves the normal empty state; Settings
 * stays open for later.
 */
export default function Onboarding({ client, onDone }: OnboardingProps) {
  const [entries, setEntries] = useState<GameDirEntry[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);
  const [newPath, setNewPath] = useState('');

  useEffect(() => {
    let cancelled = false;
    client
      .listGameDirs()
      .then((loaded) => {
        if (!cancelled) setEntries(loaded);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(toAppError(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  const choose = useCallback(
    async (path: string) => {
      setBusy(true);
      setError(null);
      try {
        const state = await client.setActiveGameDir(path);
        setEntries(state.entries);
        setActivePath(state.config.active_game_dir);
        onDone();
      } catch (err) {
        setError(toAppError(err));
      } finally {
        setBusy(false);
      }
    },
    [client, onDone],
  );

  const add = useCallback(async () => {
    const path = newPath.trim();
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      const state = await client.addGameDir(path);
      setEntries(state.entries);
      setNewPath('');
    } catch (err) {
      setError(toAppError(err));
    } finally {
      setBusy(false);
    }
  }, [client, newPath]);

  return (
    <section className="panel" aria-label="Welcome">
      <h2>Welcome to Mine Utils</h2>
      <p className="empty-note">
        Pick the Minecraft game folder to manage. Writes are blocked while the
        game runs, and every write keeps a backup.
      </p>
      {loading ? (
        <p className="empty-note" role="status">
          Looking for game folders…
        </p>
      ) : null}
      {!loading && entries.length === 0 ? (
        <p className="empty" role="status">
          No game folders found. Add one below, or skip and set it later in
          Settings.
        </p>
      ) : null}
      {!loading && entries.length > 0 ? (
        <GameDirPicker entries={entries} activePath={activePath} onSelect={(path) => void choose(path)} />
      ) : null}
      {error ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}
      <div className="sheet-form">
        <div className="field">
          <label htmlFor="onboarding-path">Add a game folder</label>
          <input
            id="onboarding-path"
            type="text"
            value={newPath}
            onChange={(event) => setNewPath(event.target.value)}
            placeholder="/home/you/.minecraft"
          />
        </div>
        <button type="button" className="button" disabled={busy || !newPath.trim()} onClick={() => void add()}>
          {busy ? 'Working…' : 'Add'}
        </button>
      </div>
      <div className="pack-confirm-actions">
        <button type="button" className="button" onClick={onDone}>
          Skip for now
        </button>
      </div>
    </section>
  );
}
