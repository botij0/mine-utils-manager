import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type {
  BackupInfo,
  GameRunning,
  LevelBackupInfo,
} from '../../lib/types';
import { IpcError } from '../../lib/types';
import { gameDir, makeFakeClient, type FakeClient } from '../../test/fake-ipc';
import { resourceStack } from '../../test/fake-packs';
import BackupsPanel from './BackupsPanel';

const RUNNING: GameRunning = {
  kind: 'process',
  detail: 'pid 4242 runs java',
  lock_path: null,
  pid: 4242,
  command: 'java -jar client.jar --gameDir /home/demo/.minecraft',
};

const STACK = resourceStack(['vanilla', 'file/Alpha.zip']);

const DEFAULT_BACKUPS: BackupInfo[] = [
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
];

const LEVEL_BACKUPS: LevelBackupInfo[] = [
  {
    file_name: '20261025T101500.000000000-Survival 1_17-level.dat',
    world: 'Survival 1_17',
    bytes: 10258,
    display: '2026-10-25 10:15:00 UTC',
  },
];

function makeClient({
  backups = DEFAULT_BACKUPS,
  error = null as IpcError | null,
  running = null as GameRunning | null,
  restored = STACK,
} = {}): FakeClient {
  return makeFakeClient({
    backups,
    backupsError: error,
    running,
    restoredStack: restored,
    levelBackups: LEVEL_BACKUPS,
  });
}

describe('BackupsPanel', () => {
  it('lists date, file, size, and a restore action', async () => {
    render(<BackupsPanel client={makeClient()} />);

    expect(await screen.findByText('2026-10-25 09:00:00 UTC')).toBeInTheDocument();
    expect(
      screen.getByText('20261025T090000.000000000-options.txt'),
    ).toBeInTheDocument();
    expect(screen.getByText('4.0 KB')).toBeInTheDocument();
    const optionsTable = screen.getAllByRole('table')[0];
    expect(within(optionsTable).getAllByRole('button', { name: 'Restore' })).toHaveLength(2);
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

  it('disables restore when the guard check is unknown', async () => {
    render(
      <BackupsPanel client={makeClient()} gameRunning={null} gameGuardUnknown />,
    );

    const buttons = await screen.findAllByRole('button', { name: 'Restore' });
    for (const button of buttons) {
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute(
        'title',
        'Mine Utils could not check whether Minecraft is running',
      );
    }
  });

  it('allows restore without an app guard when options.txt is missing and the game is closed', async () => {
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true)],
      backups: DEFAULT_BACKUPS,
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => {
      throw new IpcError('options_txt', 'options.txt is missing at /home/demo/.minecraft/options.txt');
    };
    render(<BackupsPanel client={client} activeGameDir="/home/demo/.minecraft" />);

    const optionsTable = await screen.findByRole('table');
    await act(async () => {});

    for (const button of within(optionsTable).getAllByRole('button', { name: 'Restore' })) {
      expect(button).toBeEnabled();
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

  it('lists world level.dat backups grouped by world and restores one', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    render(<BackupsPanel client={client} />);

    expect(await screen.findByText('World level.dat')).toBeInTheDocument();
    expect(await screen.findByText('Survival 1_17')).toBeInTheDocument();
    expect(
      screen.getByText('20261025T101500.000000000-Survival 1_17-level.dat'),
    ).toBeInTheDocument();

    const region = screen.getByRole('region', { name: 'Survival 1_17 backups' });
    const { getByRole } = within(region);
    await user.click(getByRole('button', { name: 'Restore' }));

    await waitFor(() =>
      expect(client.levelRestores).toEqual([
        'Survival 1_17/20261025T101500.000000000-Survival 1_17-level.dat',
      ]),
    );
  });

  it('shows backups for corrupt and missing worlds without the world list', async () => {
    const client = makeClient();
    const original = client.listAllLevelBackups;
    client.listAllLevelBackups = async () => [
      ...(await original()),
      {
        file_name: '20261025T101500.000000000-Broken World-level.dat',
        world: 'Broken World',
        bytes: 512,
        display: '2026-10-25 10:15:00 UTC',
      },
      {
        file_name: '20261025T101500.000000000-Deleted World-level.dat',
        world: 'Deleted World',
        bytes: 512,
        display: '2026-10-25 10:15:00 UTC',
      },
    ];
    render(<BackupsPanel client={client} />);

    expect(await screen.findByText('Broken World')).toBeInTheDocument();
    expect(await screen.findByText('Deleted World')).toBeInTheDocument();
    expect(
      screen.getByText('20261025T101500.000000000-Broken World-level.dat'),
    ).toBeInTheDocument();
  });

  it('surfaces the backend game_running error and asks the app guard to re-check', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    let rechecks = 0;
    client.restoreOptionsBackup = async () => {
      throw new IpcError(
        'game_running',
        'Minecraft is running (pid 4242 runs java); close it before Mine Utils writes game files',
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
