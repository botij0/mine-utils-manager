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
import { resourceStack, stack } from '../../test/fake-packs';
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

/** The copy a restore writes before it finds the stack changed on disk. */
const PRE_RESTORE_OPTIONS: BackupInfo = {
  file_name: '20261026T093000.000000000-options.txt',
  bytes: 4066,
  display: '2026-10-26 09:30:00 UTC',
};

const PRE_RESTORE_LEVEL: LevelBackupInfo = {
  file_name: '20261026T093000.000000000-Survival 1_17-level.dat',
  world: 'Survival 1_17',
  bytes: 10258,
  display: '2026-10-26 09:30:00 UTC',
};

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

/** Every button is disabled and described by the in-progress reason its own section shows. */
function expectRestoreInProgress(buttons: HTMLElement[]) {
  for (const button of buttons) expect(button).toBeDisabled();
  const reasons = screen.getAllByText('A restore is in progress');
  expect(reasons).toHaveLength(2);
  for (const reason of reasons) expect(reason).toBeVisible();
  for (const button of buttons) {
    expect(reasons.map((node) => node.id)).toContain(button.getAttribute('aria-describedby'));
    expect(button).toHaveAccessibleDescription('A restore is in progress');
  }
}

describe('BackupsPanel', () => {
  it('lists date, file, size, and a restore action', async () => {
    render(<BackupsPanel client={makeClient()} gameRunning={null} />);

    expect(await screen.findByText('2026-10-25 09:00:00 UTC')).toBeInTheDocument();
    expect(
      screen.getByText('20261025T090000.000000000-options.txt'),
    ).toBeInTheDocument();
    expect(screen.getByText('4.0 KB')).toBeInTheDocument();
    const optionsTable = screen.getAllByRole('table')[0];
    expect(within(optionsTable).getAllByRole('button', { name: /^Restore / })).toHaveLength(2);
  });

  it('names each Restore button after its backup', async () => {
    render(<BackupsPanel client={makeClient()} gameRunning={null} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    expect(
      within(optionsTable).getByRole('button', { name: 'Restore 2026-10-25 09:00:00 UTC' }),
    ).toHaveAccessibleName('Restore 2026-10-25 09:00:00 UTC');
    expect(
      within(optionsTable).getByRole('button', { name: 'Restore 2026-10-25 08:00:00 UTC' }),
    ).toHaveAccessibleName('Restore 2026-10-25 08:00:00 UTC');
    const level = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    expect(
      within(level).getByRole('button', {
        name: 'Restore 2026-10-25 10:15:00 UTC for Survival 1_17',
      }),
    ).toHaveAccessibleName('Restore 2026-10-25 10:15:00 UTC for Survival 1_17');
  });

  it('shows the empty state when there are no backups', async () => {
    render(<BackupsPanel client={makeClient({ backups: [] })} gameRunning={null} />);

    expect(
      await screen.findByText(/No backups yet/),
    ).toBeInTheDocument();
  });

  it('asks for a game dir when there is none', async () => {
    render(
      <BackupsPanel
        client={makeClient({ error: new IpcError('no_active_game_dir', 'no active game dir') })}
        gameRunning={null}
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
        gameRunning={null}
      />,
    );

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('disk on fire');
  });

  it('disables restore while the game is running', async () => {
    render(<BackupsPanel client={makeClient()} gameRunning={RUNNING} />);

    const buttons = await screen.findAllByRole('button', { name: /^Restore / });
    for (const button of buttons) {
      expect(button).toBeDisabled();
    }
  });

  it('disables restore when the guard check is unknown', async () => {
    render(
      <BackupsPanel client={makeClient()} gameRunning={null} guardPause="unknown" />,
    );

    const buttons = await screen.findAllByRole('button', { name: /^Restore / });
    for (const button of buttons) {
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute(
        'title',
        'Mine Utils could not check whether Minecraft is running',
      );
    }
  });

  it.each([
    [
      'Close Minecraft before restoring',
      { gameRunning: RUNNING },
    ],
    [
      'Mine Utils could not check whether Minecraft is running',
      { gameRunning: null, guardPause: 'unknown' as const },
    ],
    [
      'Checking whether Minecraft is running',
      { gameRunning: null, guardPause: 'checking' as const },
    ],
  ])('shows "%s" as visible text that every disabled Restore describes', async (reason, props) => {
    render(<BackupsPanel client={makeClient()} {...props} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    const level = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    const buttons = [
      ...within(optionsTable).getAllByRole('button', { name: /^Restore / }),
      ...within(level).getAllByRole('button', { name: /^Restore / }),
    ];
    expect(buttons).toHaveLength(3);

    const reasons = screen.getAllByText(reason);
    expect(reasons).toHaveLength(2);
    for (const reasonNode of reasons) expect(reasonNode).toBeVisible();
    for (const button of buttons) {
      expect(button).toBeDisabled();
      expect(reasons.map((node) => node.id)).toContain(button.getAttribute('aria-describedby'));
      expect(button).toHaveAccessibleDescription(reason);
    }
  });

  it('describes the other options Restore buttons while a restore is in progress', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    client.restoreOptionsBackup = async () => {
      await gate;
      return STACK;
    };
    render(<BackupsPanel client={client} gameRunning={null} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    const [running, other] = within(optionsTable).getAllByRole('button', { name: /^Restore / });
    await user.click(running);

    expect(await screen.findByText('A restore is in progress')).toBeVisible();
    expect(other).toBeDisabled();
    expect(other).toHaveAccessibleDescription('A restore is in progress');

    await act(async () => {
      release();
    });
    await waitFor(() => expect(other).toBeEnabled());
    expect(other).not.toHaveAttribute('aria-describedby');
    expect(screen.queryByText('A restore is in progress')).not.toBeInTheDocument();
  });

  it('describes the other level Restore buttons while a level restore is in progress', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    const twoBackups: LevelBackupInfo[] = [
      ...LEVEL_BACKUPS,
      {
        file_name: '20261025T091500.000000000-Survival 1_17-level.dat',
        world: 'Survival 1_17',
        bytes: 10258,
        display: '2026-10-25 09:15:00 UTC',
      },
    ];
    client.listAllLevelBackups = async () => [...twoBackups];
    client.listLevelBackups = async () => [...twoBackups];
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    client.restoreLevelBackup = async () => {
      await gate;
      return stack();
    };
    render(<BackupsPanel client={client} gameRunning={null} />);

    const region = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    const [running, other] = within(region).getAllByRole('button', { name: /^Restore / });
    await user.click(running);

    expect(await screen.findByText('A restore is in progress')).toBeVisible();
    expect(other).toBeDisabled();
    expect(other).toHaveAccessibleDescription('A restore is in progress');

    await act(async () => {
      release();
    });
    await waitFor(() => expect(other).toBeEnabled());
  });

  it('disables and describes every Restore button while an options restore is in progress', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    client.restoreOptionsBackup = async () => {
      await gate;
      return STACK;
    };
    render(<BackupsPanel client={client} gameRunning={null} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    const optionsButtons = within(optionsTable).getAllByRole('button', { name: /^Restore / });
    const region = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    const levelButton = within(region).getByRole('button', { name: /^Restore / });
    await user.click(optionsButtons[0]);

    expectRestoreInProgress([...optionsButtons, levelButton]);

    await act(async () => {
      release();
    });
    await waitFor(() => expect(levelButton).toBeEnabled());
    expect(levelButton).not.toHaveAttribute('aria-describedby');
  });

  it('disables and describes every Restore button while a level restore is in progress', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    client.restoreLevelBackup = async () => {
      await gate;
      return stack();
    };
    render(<BackupsPanel client={client} gameRunning={null} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    const optionsButtons = within(optionsTable).getAllByRole('button', { name: /^Restore / });
    const region = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    const levelButton = within(region).getByRole('button', { name: /^Restore / });
    await user.click(levelButton);

    expectRestoreInProgress([...optionsButtons, levelButton]);

    await act(async () => {
      release();
    });
    await waitFor(() => expect(optionsButtons[0]).toBeEnabled());
    expect(optionsButtons[0]).not.toHaveAttribute('aria-describedby');
  });

  it('keeps the game-running reason when the game starts during a restore', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    client.restoreOptionsBackup = async () => {
      await gate;
      return STACK;
    };
    const { rerender } = render(<BackupsPanel client={client} gameRunning={null} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    const [running, other] = within(optionsTable).getAllByRole('button', { name: /^Restore / });
    await user.click(running);
    await waitFor(() => expect(running).toHaveTextContent('Restoring…'));

    rerender(<BackupsPanel client={client} gameRunning={RUNNING} />);

    expect(other).toHaveAccessibleDescription('Close Minecraft before restoring');
    expect(screen.queryByText('A restore is in progress')).not.toBeInTheDocument();

    await act(async () => {
      release();
    });
  });

  it('allows restore when options.txt is missing and the guard says the game is closed', async () => {
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true)],
      backups: DEFAULT_BACKUPS,
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => {
      throw new IpcError('options_txt', 'options.txt is missing at /home/demo/.minecraft/options.txt');
    };
    render(<BackupsPanel client={client} activeGameDir="/home/demo/.minecraft" gameRunning={null} />);

    const optionsTable = await screen.findByRole('table');
    await act(async () => {});

    for (const button of within(optionsTable).getAllByRole('button', { name: /^Restore / })) {
      expect(button).toBeEnabled();
      expect(button).not.toHaveAttribute('aria-describedby');
    }
    expect(screen.queryByText(/before restoring|could not check|Checking whether/)).not.toBeInTheDocument();
  });

  it('restores a backup and reports the refreshed stack', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    render(<BackupsPanel client={client} gameRunning={null} />);

    const buttons = await screen.findAllByRole('button', { name: /^Restore / });
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
    render(<BackupsPanel client={client} gameRunning={null} />);

    expect(await screen.findByText('World level.dat')).toBeInTheDocument();
    expect(await screen.findByText('Survival 1_17')).toBeInTheDocument();
    expect(
      screen.getByText('20261025T101500.000000000-Survival 1_17-level.dat'),
    ).toBeInTheDocument();

    const region = screen.getByRole('region', { name: 'Survival 1_17 backups' });
    const { getByRole } = within(region);
    await user.click(getByRole('button', { name: /^Restore / }));

    await waitFor(() =>
      expect(client.levelRestores).toEqual([
        'Survival 1_17/20261025T101500.000000000-Survival 1_17-level.dat',
      ]),
    );
  });

  it('explains a world seed mismatch', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    client.restoreLevelBackup = async () => { throw new IpcError('world_mismatch', 'backend error'); };
    render(<BackupsPanel client={client} gameRunning={null} />);
    const region = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    await user.click(within(region).getByRole('button', { name: /^Restore / }));
    expect(await screen.findByRole('alert')).toHaveTextContent('This backup belongs to a different world');
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
    render(<BackupsPanel client={client} gameRunning={null} />);

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

    const buttons = await screen.findAllByRole('button', { name: /^Restore / });
    await user.click(buttons[0]);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/Minecraft is running/);
    expect(client.restores).toEqual([]);
    expect(rechecks).toBe(1);
  });

  it('reloads the options backups after a stale_stack restore so the pre-restore copy shows', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    const listed = [...DEFAULT_BACKUPS];
    client.listOptionsBackups = async () => [...listed];
    client.restoreOptionsBackup = async () => {
      listed.unshift(PRE_RESTORE_OPTIONS);
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    render(<BackupsPanel client={client} gameRunning={null} />);

    const optionsTable = (await screen.findAllByRole('table'))[0];
    await user.click(within(optionsTable).getAllByRole('button', { name: /^Restore / })[0]);

    expect(await screen.findByRole('alert')).toHaveTextContent('the stack changed on disk');
    expect(await within(optionsTable).findByText('2026-10-26 09:30:00 UTC')).toBeInTheDocument();
    expect(
      within(optionsTable).getByText('20261026T093000.000000000-options.txt'),
    ).toBeInTheDocument();
    expect(within(optionsTable).getAllByRole('button', { name: /^Restore / })).toHaveLength(3);
  });

  it('reloads the world level.dat backups after a stale_stack restore', async () => {
    const user = userEvent.setup();
    const client = makeClient();
    const listed = [...LEVEL_BACKUPS];
    client.listAllLevelBackups = async () => [...listed];
    client.listLevelBackups = async () => [...listed];
    client.restoreLevelBackup = async () => {
      listed.unshift(PRE_RESTORE_LEVEL);
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    render(<BackupsPanel client={client} gameRunning={null} />);

    const region = await screen.findByRole('region', { name: 'Survival 1_17 backups' });
    await user.click(within(region).getByRole('button', { name: /^Restore / }));

    expect(await screen.findByRole('alert')).toHaveTextContent('the stack changed on disk');
    expect(await within(region).findByText('2026-10-26 09:30:00 UTC')).toBeInTheDocument();
    expect(
      within(region).getByText('20261026T093000.000000000-Survival 1_17-level.dat'),
    ).toBeInTheDocument();
    expect(within(region).getAllByRole('button', { name: /^Restore / })).toHaveLength(2);
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
