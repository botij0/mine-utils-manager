import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { IpcClient } from '../../lib/ipc';
import type { Config, GameDirEntry } from '../../lib/types';
import { defaultConfig, IpcError } from '../../lib/types';
import GameDirSettings from './GameDirSettings';

function fakeClient(entries: GameDirEntry[]): IpcClient {
  let stored = [...entries];
  const config: Config = defaultConfig();
  return {
    async listGameDirs() {
      return stored;
    },
    async addGameDir(path: string) {
      if (!path.startsWith('/')) {
        throw new IpcError(
          'invalid_game_dir',
          `invalid game dir ${path}: path must be absolute`,
        );
      }
      stored = [...stored, { path, source: 'manual', present: true }];
      return stored;
    },
    async removeGameDir(path: string) {
      stored = stored.filter((entry) => entry.path !== path);
      return stored;
    },
    async setActiveGameDir(path: string) {
      return { ...config, active_game_dir: path };
    },
    async getConfig() {
      return config;
    },
    async saveConfig(next: Config) {
      return next;
    },
  };
}

const DETECTED: GameDirEntry = {
  path: '/home/demo/.minecraft',
  source: 'std',
  present: true,
};

describe('GameDirSettings', () => {
  it('adds a dir to the list', async () => {
    const user = userEvent.setup();
    render(
      <GameDirSettings
        client={fakeClient([DETECTED])}
        activePath={null}
        onConfigChanged={() => {}}
      />,
    );

    await user.type(screen.getByLabelText('Game dir path'), '/tmp/fixture-a');
    await user.click(screen.getByRole('button', { name: 'Add' }));

    expect(await screen.findByText('/tmp/fixture-a')).toBeInTheDocument();
  });

  it('removes an added dir from the list', async () => {
    const user = userEvent.setup();
    render(
      <GameDirSettings
        client={fakeClient([
          DETECTED,
          { path: '/tmp/fixture-b', source: 'manual', present: true },
        ])}
        activePath={null}
        onConfigChanged={() => {}}
      />,
    );

    await screen.findByText('/tmp/fixture-b');
    await user.click(screen.getByRole('button', { name: 'Remove' }));

    await waitFor(() =>
      expect(screen.queryByText('/tmp/fixture-b')).not.toBeInTheDocument(),
    );
    expect(screen.getByText('/home/demo/.minecraft')).toBeInTheDocument();
  });

  it('shows an inline error and keeps the list when the path is invalid', async () => {
    const user = userEvent.setup();
    render(
      <GameDirSettings
        client={fakeClient([DETECTED])}
        activePath={null}
        onConfigChanged={() => {}}
      />,
    );

    await user.type(screen.getByLabelText('Game dir path'), 'relative/path');
    await user.click(screen.getByRole('button', { name: 'Add' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'invalid game dir relative/path: path must be absolute',
    );
    expect(screen.getByText('/home/demo/.minecraft')).toBeInTheDocument();
    expect(screen.queryByText('relative/path')).not.toBeInTheDocument();
  });
});
