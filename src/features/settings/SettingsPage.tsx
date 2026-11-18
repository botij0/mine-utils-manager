import type { Config, GameRunning, Theme } from '../../lib/types';
import type { IpcClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import { useMemo, useRef, useState } from 'react';
import { SettingsWriteTimeoutError, isSettingsWriteTimeout } from '../../lib/types';
import type { ClientError } from '../../lib/types';
import GameDirSettings from '../game-dirs/GameDirSettings';
import BackupsPanel from './BackupsPanel';

const THEMES: { value: Theme; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

const SETTINGS_WRITE_TIMEOUT_MS = 15_000;

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
  const [error, setError] = useState<ClientError | null>(null);
  const failedTheme = useRef<Theme | null>(null);
  const [folderRevision, setFolderRevision] = useState(0);
  const orderedClient = useMemo(() => {
    function enqueue<T>(action: () => Promise<T>, folderWrite = false): Promise<T> {
      const result = (pendingWrites.get(client) ?? Promise.resolve()).then(() =>
        new Promise<T>((resolve, reject) => {
          let timedOut = false;
          const timer = setTimeout(() => {
            timedOut = true;
            reject(new SettingsWriteTimeoutError());
          }, SETTINGS_WRITE_TIMEOUT_MS);
          action().then(resolve, reject).finally(() => {
            clearTimeout(timer);
            if (timedOut && folderWrite) setFolderRevision((revision) => revision + 1);
          });
        }),
      );
      pendingWrites.set(client, result.then(() => {}, () => {}));
      return result;
    }
    return {
      ...client,
      addGameDir: (path: string) => enqueue(() => client.addGameDir(path), true),
      removeGameDir: (path: string) => enqueue(() => client.removeGameDir(path), true),
      setActiveGameDir: (path: string) => enqueue(() => client.setActiveGameDir(path), true),
      setTheme: (theme: Theme) => enqueue(() => client.setTheme(theme)),
    };
  }, [client]);

  async function handleTheme(theme: Theme) {
    if (!failedTheme.current || failedTheme.current === theme) {
      failedTheme.current = null;
      setError(null);
    }
    try {
      onConfigChanged(await orderedClient.setTheme(theme));
    } catch (err) {
      const next = toAppError(err);
      if (isSettingsWriteTimeout(next) && !failedTheme.current) failedTheme.current = theme;
      setError((current) => (current && isSettingsWriteTimeout(current) ? current : next));
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
            <button type="button" onClick={() => { failedTheme.current = null; setError(null); }}>Dismiss</button>
          </p>
        ) : null}
      </section>
      <GameDirSettings
        client={orderedClient}
        revision={folderRevision}
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
