import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { toAppError } from '../../lib/ipc';
import type { IpcClient } from '../../lib/ipc';
import type { ClientError, Config, GameDirEntry } from '../../lib/types';
import GameDirPicker from './GameDirPicker';

interface GameDirSettingsProps {
  client: IpcClient;
  activePath: string | null;
  onConfigChanged: (config: Config) => void;
}

export default function GameDirSettings({
  client,
  activePath,
  onConfigChanged,
}: GameDirSettingsProps) {
  const [entries, setEntries] = useState<GameDirEntry[]>([]);
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<ClientError | null>(null);

  useEffect(() => {
    client
      .listGameDirs()
      .then(setEntries)
      .catch((err: unknown) => setError(toAppError(err)));
  }, [client]);

  async function handleAdd(event: FormEvent) {
    event.preventDefault();
    const path = draft.trim();
    if (!path) return;
    try {
      const state = await client.addGameDir(path);
      setEntries(state.entries);
      onConfigChanged(state.config);
      setDraft('');
      setError(null);
    } catch (err) {
      setError(toAppError(err));
    }
  }

  async function handleRemove(path: string) {
    try {
      const state = await client.removeGameDir(path);
      setEntries(state.entries);
      onConfigChanged(state.config);
      setError(null);
    } catch (err) {
      setError(toAppError(err));
    }
  }

  async function handleSelect(path: string) {
    try {
      const state = await client.setActiveGameDir(path);
      setEntries(state.entries);
      onConfigChanged(state.config);
      setError(null);
    } catch (err) {
      setError(toAppError(err));
    }
  }

  return (
    <section className="panel">
      <h2>Game dirs</h2>
      <p className="hint">
        Detected Minecraft dirs and any dir you add. The active dir drives every
        page.
      </p>
      <GameDirPicker
        entries={entries}
        activePath={activePath}
        onSelect={handleSelect}
        onRemove={handleRemove}
      />
      <form className="add-form" onSubmit={handleAdd}>
        <input
          type="text"
          value={draft}
          aria-label="Game dir path"
          placeholder="/path/to/.minecraft"
          onChange={(event) => setDraft(event.target.value)}
        />
        <button type="submit" className="button primary">
          Add
        </button>
      </form>
      {error ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}
