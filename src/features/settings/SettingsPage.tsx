import type { Config, GameRunning, Theme } from '../../lib/types';
import type { IpcClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import { useMemo, useState } from 'react';
import type { AppError } from '../../lib/types';
import GameDirSettings from '../game-dirs/GameDirSettings';
import BackupsPanel from './BackupsPanel';

const THEMES: { value: Theme; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

const pendingWrites = new WeakMap<IpcClient, Promise<void>>();

interface SettingsPageProps {
  active?: boolean;
  client: IpcClient;
  config: Config;
  onConfigChanged: (config: Config) => void;
  onBackupRestored?: (file: 'options' | 'level') => void;
  /** From the app-level game guard poll. Undefined until the first check. */
  gameRunning: GameRunning | null | undefined;
  /** No current guard answer: unknown, so writes stay paused. */
  gameGuardUnknown?: boolean;
  /** The unknown state is a new folder's first check, still pending. */
  gameGuardChecking?: boolean;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
}

export default function SettingsPage({
  client,
  active = true,
  config,
  onConfigChanged,
  onBackupRestored,
  gameRunning,
  gameGuardUnknown = false,
  gameGuardChecking = false,
  refreshGameRunning = () => {},
}: SettingsPageProps) {
  const [error, setError] = useState<AppError | null>(null);
  const orderedClient = useMemo(() => {
    function enqueue<T>(action: () => Promise<T>): Promise<T> {
      const result = (pendingWrites.get(client) ?? Promise.resolve()).then(action);
      pendingWrites.set(client, result.then(() => {}, () => {}));
      return result;
    }
    return {
      ...client,
      addGameDir: (path: string) => enqueue(() => client.addGameDir(path)),
      removeGameDir: (path: string) => enqueue(() => client.removeGameDir(path)),
      setActiveGameDir: (path: string) => enqueue(() => client.setActiveGameDir(path)),
      setTheme: (theme: Theme) => enqueue(() => client.setTheme(theme)),
    };
  }, [client]);

  async function handleTheme(theme: Theme) {
    try {
      onConfigChanged(await orderedClient.setTheme(theme));
      setError(null);
    } catch (err) {
      setError(toAppError(err));
    }
  }

  return (
    <div>
      <section className="panel">
        <h2>Theme</h2>
        <div className="theme-options">
          {THEMES.map((theme) => (
            <label key={theme.value}>
              <input
                type="radio"
                name="theme"
                value={theme.value}
                checked={config.theme === theme.value}
                onChange={() => handleTheme(theme.value)}
              />
              {theme.label}
            </label>
          ))}
        </div>
        {error ? (
          <p className="error" role="alert">
            {error.message}
          </p>
        ) : null}
      </section>
      <GameDirSettings
        client={orderedClient}
        activePath={config.active_game_dir}
        onConfigChanged={onConfigChanged}
      />
      <BackupsPanel
        active={active}
        client={client}
        activeGameDir={config.active_game_dir}
        gameRunning={gameRunning}
        gameGuardUnknown={gameGuardUnknown}
        gameGuardChecking={gameGuardChecking}
        refreshGameRunning={refreshGameRunning}
        onRestored={() => onBackupRestored?.('options')}
        onLevelRestored={() => onBackupRestored?.('level')}
      />
    </div>
  );
}
