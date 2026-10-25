import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { PackClient } from '../../lib/ipc';
import type {
  BackupInfo,
  GameRunning,
  ResourceStack,
} from '../../lib/types';
import { IpcError } from '../../lib/types';
import BackupsPanel from './BackupsPanel';

const RUNNING: GameRunning = {
  kind: 'process',
  detail: 'pid 4242 runs java',
  lock_path: null,
  pid: 4242,
  command: 'java -jar client.jar --gameDir /home/demo/.minecraft',
};

const STACK: ResourceStack = {
  entries: [
    { kind: 'foreign', value: 'vanilla' },
    { kind: 'file', value: 'file/Alpha.zip', filename: 'Alpha.zip' },
  ],
  incompatible: [],
  running: null,
  stale_locks: [],
  detection_supported: true,
};

function makeClient({
  backups = [
    {
      file_name: '20261025T090000.000000000-options.txt',
      bytes: 4066,
      display: '2026-10-25 09:00:00 UTC',
    },
    {
      file_name: '20261025T080000.000000000-options.txt',
      bytes: 1024,
      display: '2026-10-25 08:00:00 UTC',
    },
  ] as BackupInfo[],
  error = null as IpcError | null,
  running = null as GameRunning | null,
  restored = STACK as ResourceStack,
} = {}) {
  const restores: string[] = [];
  const client: PackClient & { restores: string[] } = {
    restores,
    async listResourcePacks() {
      return { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 };
    },
    async importResourcePack(): Promise<never> {
      throw new Error('not used');
    },
    async deleteResourcePack() {},
    async getResourceStack() {
      return { ...STACK, running };
    },
    async setResourcePackEnabled(): Promise<ResourceStack> {
      throw new Error('not used');
    },
    async reorderResourceStack(): Promise<ResourceStack> {
      throw new Error('not used');
    },
    async listOptionsBackups() {
      if (error) throw error;
      return backups;
    },
    async restoreOptionsBackup(backup: string): Promise<ResourceStack> {
      restores.push(backup);
      return restored;
    },
  };
  return client;
}

describe('BackupsPanel', () => {
  it('lists date, file, size, and a restore action', async () => {
    render(<BackupsPanel client={makeClient()} />);

    expect(await screen.findByText('2026-10-25 09:00:00 UTC')).toBeInTheDocument();
    expect(
      screen.getByText('20261025T090000.000000000-options.txt'),
    ).toBeInTheDocument();
    expect(screen.getByText('4.0 KB')).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Restore' })).toHaveLength(2);
  });

  it('shows the empty state when there are no backups', async () => {
    render(<BackupsPanel client={makeClient({ backups: [] })} />);

    expect(
      await screen.findByText(/No backups yet/),
    ).toBeInTheDocument();
  });

  it('asks for a game dir when there is none', async () => {
    render(
      <BackupsPanel
        client={makeClient({ error: new IpcError('no_active_game_dir', 'no active game dir') })}
      />,
    );

    expect(
      await screen.findByText(/Pick a game dir first/),
    ).toBeInTheDocument();
  });

  it('shows other errors in an alert', async () => {
    render(
      <BackupsPanel
        client={makeClient({ error: new IpcError('backup', 'backup failed: disk on fire') })}
      />,
    );

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('disk on fire');
  });

  it('disables restore while the game is running', async () => {
    render(<BackupsPanel client={makeClient()} gameRunning={RUNNING} />);

    const buttons = await screen.findAllByRole('button', { name: 'Restore' });
    for (const button of buttons) {
      expect(button).toBeDisabled();
    }
  });

  it('restores a backup and reports the refreshed stack', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    render(<BackupsPanel client={client} />);

    const buttons = await screen.findAllByRole('button', { name: 'Restore' });
    await user.click(buttons[0]);

    await waitFor(() =>
      expect(client.restores).toEqual(['20261025T090000.000000000-options.txt']),
    );
    const status = await screen.findByRole('status');
    expect(status).toHaveTextContent(/The active stack now has 2 entries/);
  });

  it('surfaces the backend game_running error and asks the app guard to re-check', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    let rechecks = 0;
    client.restoreOptionsBackup = async () => {
      throw new IpcError(
        'game_running',
        'Minecraft is running (pid 4242 runs java); close it before Mine Utils writes options.txt',
      );
    };
    render(
      <BackupsPanel
        client={client}
        gameRunning={null}
        refreshGameRunning={() => {
          rechecks += 1;
        }}
      />,
    );

    const buttons = await screen.findAllByRole('button', { name: 'Restore' });
    await user.click(buttons[0]);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/Minecraft is running/);
    expect(client.restores).toEqual([]);
    expect(rechecks).toBe(1);
  });

  it('reloads the table when the active game dir changes, not when the guard object does', async () => {
    const client = makeClient();
    let calls = 0;
    const original = client.listOptionsBackups;
    client.listOptionsBackups = async () => {
      calls += 1;
      return original();
    };
    const { rerender } = render(
      <BackupsPanel client={client} activeGameDir="/games/one" gameRunning={null} />,
    );
    await screen.findByText('2026-10-25 09:00:00 UTC');
    expect(calls).toBe(1);

    // A new 5-second poll object must not reload or flash the table.
    rerender(
      <BackupsPanel
        client={client}
        activeGameDir="/games/one"
        gameRunning={{ ...RUNNING }}
      />,
    );
    await waitFor(() => expect(calls).toBe(1));

    rerender(
      <BackupsPanel client={client} activeGameDir="/games/two" gameRunning={null} />,
    );
    await waitFor(() => expect(calls).toBe(2));
  });
});
