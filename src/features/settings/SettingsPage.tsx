import type { Config, GameRunning, Theme } from '../../lib/types';
import type { IpcClient } from '../../lib/ipc';
import { toAppError } from '../../lib/ipc';
import { useState } from 'react';
import type { AppError } from '../../lib/types';
import GameDirSettings from '../game-dirs/GameDirSettings';
import BackupsPanel from './BackupsPanel';

const THEMES: { value: Theme; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

interface SettingsPageProps {
  client: IpcClient;
  config: Config;
  onConfigChanged: (config: Config) => void;
  /** From the app-level game guard poll, if there is one. */
  gameRunning?: GameRunning | null;
}

export default function SettingsPage({
  client,
  config,
  onConfigChanged,
  gameRunning = null,
}: SettingsPageProps) {
  const [error, setError] = useState<AppError | null>(null);

  async function handleTheme(theme: Theme) {
    try {
      onConfigChanged(await client.setTheme(theme));
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
        client={client}
        activePath={config.active_game_dir}
        onConfigChanged={onConfigChanged}
      />
      <BackupsPanel client={client} gameRunning={gameRunning} />
    </div>
  );
}
