import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { IpcClient } from './lib/ipc';
import type { GameRunning } from './lib/types';
import { IpcError } from './lib/types';
import { datapack, makeFakeClient, gameDir, stack, world } from './test/fake-ipc';
import type { FakeClient } from './test/fake-ipc';
import { freshHit, pack as worldPack } from './test/fake-packs';

afterEach(() => {
  vi.useRealTimers();
  Reflect.deleteProperty(document, 'hidden');
  window.location.hash = '';
});

const RUNNING: GameRunning = {
  kind: 'process',
  detail: 'pid 4242 runs java',
  lock_path: null,
  pid: 4242,
  command: 'java -jar client.jar',
};

const OPTIONS_BACKUP = {
  file_name: '20261025T090000.000000000-options.txt',
  bytes: 4096,
  display: '2026-10-25 09:00:00 UTC',
};

function recordGuardDirs(client: FakeClient): (string | null)[] {
  const checkedDirs: (string | null)[] = [];
  const guard = client.getGameGuard;
  client.getGameGuard = () => {
    checkedDirs.push(client.config.active_game_dir);
    return guard();
  };
  return checkedDirs;
}

function holdGuardCalls(client: FakeClient) {
  const calls: { dir: string | null; answer: (running: GameRunning | null) => void }[] = [];
  client.getGameGuard = () => {
    const dir = client.config.active_game_dir;
    return new Promise((resolve) =>
      calls.push({
        dir,
        answer: (running) => resolve({ running, stale_locks: [], detection_supported: true, process_probe_available: true }),
      }),
    );
  };
  return calls;
}

function setDocumentHidden(hidden: boolean) {
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  document.dispatchEvent(new Event('visibilitychange'));
}

/** A client past onboarding: a stored dir set active, like a returning user. */
async function shellClient() {
  const client = makeFakeClient([gameDir('/home/demo/.minecraft', true, true)]);
  await client.setActiveGameDir('/home/demo/.minecraft');
  return client;
}

function stackResponse(running: GameRunning | null) {
  return {
    entries: [{ kind: 'file' as const, value: 'file/Alpha.zip', filename: 'Alpha.zip' }],
    incompatible: [],
    running,
    stale_locks: [],
    detection_supported: true, process_probe_available: true,
  };
}

describe('App', () => {
  it('shows the startup warning and the shell', async () => {
    const client = await shellClient();
    const withWarning: IpcClient = {
      ...client,
      getStartupWarning: async () => 'The config file could not be read.',
    };

    render(<App client={withWarning} />);

    expect(await screen.findByRole('status')).toHaveTextContent(
      'The config file could not be read.',
    );
    expect(screen.getByText('Mine Utils')).toBeInTheDocument();
  });

  it('keeps the running banner and disabled writes when a later guard check fails', async () => {
    // Only the five-second poll uses timers; the rest of the page keeps real
    // timers so Testing Library can await normal UI updates.
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const client = await shellClient();
    const running: GameRunning = {
      kind: 'process',
      detail: 'pid 4242 runs java',
      lock_path: null,
      pid: 4242,
      command: 'java -jar client.jar',
    };
    let fail = false;
    client.getResourceStack = async () => stackResponse(null);
    client.getGameGuard = async () => {
      if (fail) throw new IpcError('io', 'guard read failed');
      return { running, stale_locks: [], detection_supported: true, process_probe_available: true };
    };

    render(<App client={client} />);

    const list = await screen.findByRole('list', { name: 'Stack order' });
    expect(await screen.findByText('Minecraft is running.')).toBeInTheDocument();
    expect(within(list).getByRole('button', { name: 'Remove' })).toBeDisabled();

    fail = true;
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });

    expect(screen.getByText('Minecraft is running.')).toBeInTheDocument();
    expect(screen.getByText(/pid 4242 runs java/)).toBeInTheDocument();
    expect(within(list).getByRole('button', { name: 'Remove' })).toBeDisabled();
  });

  it('shows the unavailable banner when the first guard check fails', async () => {
    const client = await shellClient();
    client.getGameGuard = async () => {
      throw new IpcError('io', 'guard read failed');
    };

    render(<App client={client} />);

    expect(
      await screen.findByText(
        /could not check\s+whether Minecraft is running, so writes are paused/i,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText('Minecraft is running.')).not.toBeInTheDocument();
  });

  it('shows onboarding on first run and skips to the shell', async () => {
    const user = (await import('@testing-library/user-event')).default.setup();
    const client = makeFakeClient([]);
    render(<App client={client} />);

    expect(await screen.findByText('Welcome to Mine Utils')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Skip for now' }));
    expect(await screen.findByRole('button', { name: 'Resource Packs' })).toBeInTheDocument();
  });

  it('allows datapack toggles and restores when options.txt is unreadable but the game is closed', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true)],
      backups: [
        {
          file_name: '20261025T090000.000000000-options.txt',
          bytes: 4096,
          display: '2026-10-25 09:00:00 UTC',
        },
      ],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => {
      throw new IpcError(
        'options_txt',
        'options.txt is missing at /home/demo/.minecraft/options.txt; launch Minecraft once so it writes the file, then try again',
      );
    };
    client.listWorlds = async () => [world()];
    client.listDatapacks = async () => ({
      packs: {
        packs: [datapack('a.zip'), datapack('b.zip')],
        count: 2,
        bytes_read: 128,
        elapsed_ms: 1,
      },
      stack: stack(),
    });

    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Worlds' }));
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByRole('button', { name: 'Disable' })).toBeEnabled();
    expect(within(grid).getByRole('button', { name: 'Enable' })).toBeEnabled();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const restore = await screen.findByRole('button', { name: 'Restore' });
    expect(restore).toBeEnabled();
    expect(screen.queryByText('Game guard unavailable.')).not.toBeInTheDocument();
  });

  it('shows no guard alert after skipping onboarding', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([]);
    client.getResourceStack = async () => {
      throw new IpcError('no_active_game_dir', 'no active game dir; pick one in Settings first');
    };
    const checkedDirs = recordGuardDirs(client);
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Skip for now' }));

    expect(await screen.findByText('Pick a game dir in Settings first.')).toBeInTheDocument();
    expect(screen.queryByText('Game guard unavailable.')).not.toBeInTheDocument();
    await waitFor(() => expect(checkedDirs).toEqual([null, null]));
    expect(screen.queryByText('Game guard unavailable.')).not.toBeInTheDocument();
  });

  it('checks the guard as soon as the active folder changes', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const user = userEvent.setup();
    const client = makeFakeClient([
      gameDir('/home/demo/.minecraft', true, true),
      gameDir('/home/demo/prism', true, true),
    ]);
    await client.setActiveGameDir('/home/demo/.minecraft');
    const checkedDirs = recordGuardDirs(client);
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await waitFor(() => expect(checkedDirs).toEqual(['/home/demo/.minecraft']));
    await user.click(await screen.findByRole('button', { name: 'Use' }));

    await waitFor(() =>
      expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/prism']),
    );
  });

  it('drops a guard answer sent for the previous folder', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true), gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    const held: { dir: string | null; answer: () => void }[] = [];
    client.getGameGuard = () => {
      const dir = client.config.active_game_dir;
      return new Promise((resolve) =>
        held.push({
          dir,
          answer: () => resolve({ running: null, stale_locks: [], detection_supported: true, process_probe_available: true }),
        }),
      );
    };
    const answerFor = (dir: string) =>
      act(async () => {
        for (const call of held) if (call.dir === dir) call.answer();
      });
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/prism/ }),
    ).toBeInTheDocument();
    await answerFor('/home/demo/.minecraft');

    const restore = await screen.findByRole('button', { name: 'Restore' });
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Checking whether Minecraft is running');

    await answerFor('/home/demo/prism');
    expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled();
  });

  it('shows a neutral checking line while the check a folder switch sent is pending', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([
      gameDir('/home/demo/.minecraft', true, true),
      gameDir('/home/demo/prism', true, true),
    ]);
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => stackResponse(null);
    const guard = client.getGameGuard;
    const held: Array<() => void> = [];
    client.getGameGuard = () =>
      client.config.active_game_dir === '/home/demo/prism'
        ? new Promise((resolve) => held.push(() => resolve(guard())))
        : guard();
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    const list = await screen.findByRole('list', { name: 'Stack order' });

    expect(await screen.findByText(/^Checking whether Minecraft is running/)).toHaveAttribute(
      'role',
      'status',
    );
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(within(list).getByRole('button', { name: 'Remove' })).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'Discover' }));
    expect(screen.getByRole('status')).toHaveTextContent(/^Checking whether Minecraft is running/);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    await act(async () => {
      for (const answer of held) answer();
    });
    expect(screen.queryByText(/^Checking whether Minecraft is running/)).not.toBeInTheDocument();
    expect(
      within(await screen.findByRole('list', { name: 'Stack order' })).getByRole('button', {
        name: 'Remove',
      }),
    ).toBeEnabled();
  });

  it('shows the unavailable alert when the check a folder switch sent fails', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([
      gameDir('/home/demo/.minecraft', true, true),
      gameDir('/home/demo/prism', true, true),
    ]);
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => stackResponse(null);
    const guard = client.getGameGuard;
    client.getGameGuard = async () => {
      if (client.config.active_game_dir === '/home/demo/prism') {
        throw new IpcError('io', 'guard read failed');
      }
      return guard();
    };
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    const list = await screen.findByRole('list', { name: 'Stack order' });

    expect(await screen.findByRole('alert')).toHaveTextContent('Game guard unavailable.');
    expect(screen.queryByText(/^Checking whether Minecraft is running/)).not.toBeInTheDocument();
    expect(within(list).getByRole('button', { name: 'Remove' })).toBeDisabled();
  });

  it('pauses writes when the folder changes while the window is hidden', async () => {
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    const noFolderGuard = client.getGameGuard;
    const checkedDirs: (string | null)[] = [];
    client.getGameGuard = async () => {
      const dir = client.config.active_game_dir;
      checkedDirs.push(dir);
      if (dir === null) return noFolderGuard();
      return { running: RUNNING, stale_locks: [], detection_supported: true, process_probe_available: true };
    };
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Skip for now' }));
    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    expect(await screen.findByRole('button', { name: 'Restore' })).toBeEnabled();
    await waitFor(() => expect(checkedDirs).toEqual([null, null]));

    await act(async () => {
      setDocumentHidden(true);
    });
    await user.click(screen.getByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/prism/ }),
    ).toBeInTheDocument();
    await act(async () => {
      vi.advanceTimersByTime(15000);
    });

    const restore = screen.getByRole('button', { name: 'Restore' });
    expect(checkedDirs).toEqual([null, null]);
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Checking whether Minecraft is running');

    await act(async () => {
      setDocumentHidden(false);
    });
    expect(checkedDirs).toEqual([null, null, '/home/demo/prism']);
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Close Minecraft before restoring');
  });

  it('keeps one guard check outstanding after it times out and checks again once it settles', async () => {
    // The guard timeout is a setTimeout. The clock still follows real time so
    // Testing Library and user-event can await their own short timers.
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    const client = await shellClient();
    client.getResourceStack = async () => stackResponse(null);
    const guard = client.getGameGuard;
    let checks = 0;
    let settleHung = () => {};
    client.getGameGuard = () => {
      checks += 1;
      if (checks !== 2) return guard();
      return new Promise((resolve) => {
        settleHung = () => resolve(guard());
      });
    };
    render(<App client={client} />);
    const list = await screen.findByRole('list', { name: 'Stack order' });
    const remove = within(list).getByRole('button', { name: 'Remove' });
    await waitFor(() => expect(remove).toBeEnabled());

    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(checks).toBe(2);
    expect(remove).toBeEnabled();

    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Game guard unavailable.');
    expect(remove).toBeDisabled();

    for (let tick = 0; tick < 12; tick += 1) {
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
    }
    await act(async () => {
      setDocumentHidden(true);
    });
    await act(async () => {
      setDocumentHidden(false);
    });
    expect(checks).toBe(2);
    expect(remove).toBeDisabled();

    await act(async () => {
      settleHung();
    });
    expect(checks).toBe(3);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(remove).toBeEnabled();

    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(checks).toBe(4);
  });

  it('keeps writes paused when a timed-out check answers late until a fresh check succeeds', async () => {
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    const client = await shellClient();
    client.getResourceStack = async () => stackResponse(null);
    const guard = client.getGameGuard;
    const held: Array<() => void> = [];
    let checks = 0;
    client.getGameGuard = () => {
      checks += 1;
      if (checks === 1) return guard();
      return new Promise((resolve) => held.push(() => resolve(guard())));
    };
    render(<App client={client} />);
    const list = await screen.findByRole('list', { name: 'Stack order' });
    const remove = within(list).getByRole('button', { name: 'Remove' });
    await waitFor(() => expect(remove).toBeEnabled());

    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await act(async () => {
      vi.advanceTimersByTime(15000);
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Game guard unavailable.');
    expect(checks).toBe(2);

    await act(async () => {
      held[0]();
    });
    expect(checks).toBe(3);
    expect(screen.getByRole('alert')).toHaveTextContent('Game guard unavailable.');
    expect(remove).toBeDisabled();

    await act(async () => {
      held[1]();
    });
    expect(checks).toBe(3);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(remove).toBeEnabled();
  });

  it('times out the check a folder switch sent and checks that folder again once it settles', async () => {
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true), gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    const guard = client.getGameGuard;
    const checkedDirs: (string | null)[] = [];
    let settleHung = () => {};
    client.getGameGuard = () => {
      const dir = client.config.active_game_dir;
      checkedDirs.push(dir);
      const prismChecks = checkedDirs.filter((checked) => checked === '/home/demo/prism').length;
      if (dir !== '/home/demo/prism' || prismChecks !== 1) return guard();
      return new Promise((resolve) => {
        settleHung = () => resolve(guard());
      });
    };
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled());
    await user.click(screen.getByRole('button', { name: 'Use' }));
    await waitFor(() =>
      expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/prism']),
    );

    await act(async () => {
      vi.advanceTimersByTime(15000);
    });
    const restore = screen.getByRole('button', { name: 'Restore' });
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Mine Utils could not check whether Minecraft is running');

    for (let tick = 0; tick < 4; tick += 1) {
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
    }
    expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/prism']);
    expect(restore).toBeDisabled();

    await act(async () => {
      settleHung();
    });
    expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/prism', '/home/demo/prism']);
    expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled();
  });

  it('shows unknown one window after a folder switch finds a timed-out check still hung', async () => {
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true), gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    const guard = client.getGameGuard;
    const checkedDirs: (string | null)[] = [];
    let settleHung = () => {};
    client.getGameGuard = () => {
      checkedDirs.push(client.config.active_game_dir);
      if (checkedDirs.length !== 2) return guard();
      return new Promise((resolve) => {
        settleHung = () => resolve({ running: null, stale_locks: [], detection_supported: true, process_probe_available: true });
      });
    };
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled());
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await act(async () => {
      vi.advanceTimersByTime(15000);
    });
    expect(screen.getByRole('button', { name: 'Restore' })).toHaveAttribute(
      'title',
      'Mine Utils could not check whether Minecraft is running',
    );
    expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/.minecraft']);

    await user.click(screen.getByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/prism/ }),
    ).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Restore' })).toHaveAttribute(
      'title',
      'Checking whether Minecraft is running',
    );

    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(screen.getByRole('button', { name: 'Restore' })).toHaveAttribute(
      'title',
      'Checking whether Minecraft is running',
    );
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    const restore = screen.getByRole('button', { name: 'Restore' });
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Mine Utils could not check whether Minecraft is running');
    expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/.minecraft']);

    await act(async () => {
      settleHung();
    });
    expect(checkedDirs).toEqual([
      '/home/demo/.minecraft',
      '/home/demo/.minecraft',
      '/home/demo/prism',
    ]);
    expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled();
  });

  it('sends no guard request until a window that starts hidden is shown', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    setDocumentHidden(true);
    const client = await shellClient();
    const checkedDirs = recordGuardDirs(client);
    render(<App client={client} />);

    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/\.minecraft/ }),
    ).toBeInTheDocument();
    await act(async () => {
      vi.advanceTimersByTime(15000);
    });
    expect(checkedDirs).toEqual([]);

    await act(async () => {
      setDocumentHidden(false);
    });
    expect(checkedDirs).toEqual(['/home/demo/.minecraft']);
  });

  it('sends no guard request while the window is hidden and checks once it is shown', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const client = await shellClient();
    const checkedDirs = recordGuardDirs(client);
    render(<App client={client} />);
    await waitFor(() => expect(checkedDirs).toHaveLength(1));

    await act(async () => {
      setDocumentHidden(true);
      vi.advanceTimersByTime(15000);
    });
    expect(checkedDirs).toHaveLength(1);

    await act(async () => {
      setDocumentHidden(false);
    });
    expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/.minecraft']);
  });

  it('skips a poll tick while a guard check is still in flight', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const client = await shellClient();
    const guard = client.getGameGuard;
    const pending: Array<() => void> = [];
    let checks = 0;
    client.getGameGuard = () => {
      checks += 1;
      return new Promise((resolve) => pending.push(() => resolve(guard())));
    };
    render(<App client={client} />);
    await waitFor(() => expect(checks).toBe(1));

    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(checks).toBe(1);

    await act(async () => {
      pending.shift()?.();
    });
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(checks).toBe(2);
  });
  it('keeps one guard call outstanding when Settings opens first and the user moves around', async () => {
    window.location.hash = 'settings';
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true), gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => stackResponse(null);
    const calls = holdGuardCalls(client);
    render(<App client={client} />);

    expect(await screen.findByRole('button', { name: 'Restore' })).toBeInTheDocument();
    for (let trip = 0; trip < 4; trip += 1) {
      await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
      await screen.findByRole('list', { name: 'Stack order' });
      await user.click(screen.getByRole('button', { name: 'Settings' }));
      await screen.findByRole('button', { name: 'Restore' });
    }
    await user.click(screen.getByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/prism/ }),
    ).toBeInTheDocument();
    await act(async () => {});

    expect(calls.map((call) => call.dir)).toEqual(['/home/demo/.minecraft']);
  });

  it('blocks Restore with the checking reason until the first guard answer lands', async () => {
    window.location.hash = 'settings';
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    const calls = holdGuardCalls(client);
    render(<App client={client} />);

    const restore = await screen.findByRole('button', { name: 'Restore' });
    await act(async () => {});
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Checking whether Minecraft is running');

    await act(async () => {
      for (const call of calls) call.answer(null);
    });
    expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled();
  });
  it('shows unknown one window after a hidden folder switch finds a timed-out check still hung', async () => {
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    window.location.hash = 'settings';
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true), gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    client.getResourceStack = async () => stackResponse(null);
    const calls = holdGuardCalls(client);
    render(<App client={client} />);

    await screen.findByRole('button', { name: 'Restore' });
    await waitFor(() => expect(calls).toHaveLength(1));
    await act(async () => {
      vi.advanceTimersByTime(15000);
    });
    expect(screen.getByRole('button', { name: 'Restore' })).toHaveAttribute(
      'title',
      'Mine Utils could not check whether Minecraft is running',
    );

    await act(async () => {
      setDocumentHidden(true);
    });
    await user.click(screen.getByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/prism/ }),
    ).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Restore' })).toHaveAttribute(
      'title',
      'Checking whether Minecraft is running',
    );

    await act(async () => {
      vi.advanceTimersByTime(15000);
    });
    expect(screen.getByRole('button', { name: 'Restore' })).toHaveAttribute(
      'title',
      'Mine Utils could not check whether Minecraft is running',
    );
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    expect(await screen.findByText('Game guard unavailable.')).toBeInTheDocument();
    expect(calls.map((call) => call.dir)).toEqual(['/home/demo/.minecraft']);
  });

  it('drops a hung answer after switching away and back, and lets a fresh check answer', async () => {
    window.location.hash = 'settings';
    const user = userEvent.setup();
    const client = makeFakeClient({
      gameDirs: [gameDir('/home/demo/.minecraft', true, true), gameDir('/home/demo/prism', true, true)],
      backups: [OPTIONS_BACKUP],
    });
    await client.setActiveGameDir('/home/demo/.minecraft');
    const calls = holdGuardCalls(client);
    render(<App client={client} />);

    await screen.findByRole('button', { name: 'Restore' });
    await user.click(screen.getByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/prism/ }),
    ).toBeInTheDocument();
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    expect(
      await screen.findByRole('button', { name: /Active game dir\s*\/home\/demo\/\.minecraft/ }),
    ).toBeInTheDocument();
    expect(calls.map((call) => call.dir)).toEqual(['/home/demo/.minecraft']);

    await act(async () => {
      calls[0].answer(RUNNING);
    });
    const restore = await screen.findByRole('button', { name: 'Restore' });
    expect(restore).toBeDisabled();
    expect(restore).toHaveAttribute('title', 'Checking whether Minecraft is running');
    expect(calls.map((call) => call.dir)).toEqual(['/home/demo/.minecraft', '/home/demo/.minecraft']);

    await act(async () => {
      calls[1].answer(null);
    });
    expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled();
  });

  it('sends nothing and leaves no timer after unmount', async () => {
    vi.useFakeTimers({
      toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout'],
      shouldAdvanceTime: true,
    });
    const client = await shellClient();
    const calls = holdGuardCalls(client);
    const { unmount } = render(<App client={client} />);
    await waitFor(() => expect(calls).toHaveLength(1));
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });

    unmount();
    expect(vi.getTimerCount()).toBe(0);
    await act(async () => {
      calls[0].answer(null);
    });
    expect(calls).toHaveLength(1);
  });

  it('shows the page the hash names after Back and Forward', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    render(<App client={client} />);

    await screen.findByRole('heading', { level: 1, name: 'Resource Packs' });
    const entries = window.history.length;
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
    expect(await screen.findByRole('heading', { level: 1, name: 'Worlds' })).toBeInTheDocument();
    expect(window.history.length).toBe(entries + 1);

    window.history.back();
    expect(
      await screen.findByRole('heading', { level: 1, name: 'Resource Packs' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Resource Packs' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(window.history.length).toBe(entries + 1);

    window.history.forward();
    expect(await screen.findByRole('heading', { level: 1, name: 'Worlds' })).toBeInTheDocument();
  });

  it('falls back to Packs when the hash names no page', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Worlds' }));
    expect(await screen.findByRole('heading', { level: 1, name: 'Worlds' })).toBeInTheDocument();
    const entries = window.history.length;

    window.location.hash = 'nonsense';
    expect(
      await screen.findByRole('heading', { level: 1, name: 'Resource Packs' }),
    ).toBeInTheDocument();
    expect(window.location.hash).toBe('#packs');
    expect(window.history.length).toBe(entries + 1);
  });
  it('retains the selected world across navigation', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    client.listWorlds = async () => [world(), world({ dir_name: 'second', level_name: 'Second World' })];
    render(<App client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Worlds' }));
    await user.click(await screen.findByRole('button', { name: /Second World/ }));
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
    expect(await screen.findByRole('button', { name: /Second World/ })).toHaveAttribute('aria-current', 'true');
  });

  it('retains Discover search and loaded results across navigation', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    client.searchModrinth = async (_query, _kind, _version, _sort, offset = 0) => ({
      hits: [{ ...freshHit(), project_id: offset === 0 ? 'first' : 'second', title: offset === 0 ? 'First result' : 'Second result' }],
      has_more: offset === 0,
    });
    render(<App client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Discover' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'fresh' } });
    await user.click(await screen.findByRole('button', { name: 'Load more' }));
    expect(await screen.findByText('Second result')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Discover' }));
    expect(screen.getByLabelText('Search Modrinth')).toHaveValue('fresh');
    expect(screen.getByText('First result')).toBeInTheDocument();
    expect(screen.getByText('Second result')).toBeInTheDocument();
  });

  it('refreshes Packs after a pending install without losing its filter', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    const install = client.installModrinthPack;
    const list = client.listResourcePacks;
    let installed = false;
    client.listResourcePacks = async () => {
      const listing = await list();
      return installed ? { ...listing, packs: [...listing.packs, { ...worldPack('fresh.zip'), name: 'Fresh Animations' }] } : listing;
    };
    let release = () => {};
    client.installModrinthPack = (args) => new Promise((resolve, reject) => {
      release = () => { installed = true; void install(args).then(resolve, reject); };
    });
    render(<App client={client} />);
    await user.type(await screen.findByLabelText('Search packs'), 'fresh');
    await user.click(screen.getByRole('button', { name: 'Discover' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'fresh' } });
    await user.click(await screen.findByRole('button', { name: /^Fresh Animations,/ }));
    await user.click(await screen.findByRole('button', { name: 'Install…' }));
    await user.click(await screen.findByRole('button', { name: 'Install' }));
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    await act(async () => release());
    expect(await screen.findByLabelText('Search packs')).toHaveValue('fresh');
    expect(within(await screen.findByRole('list', { name: 'Installed packs' })).getByText('Fresh Animations')).toBeInTheDocument();
  });

  it('does not reload packs or worlds when returning without changes', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    const listPacks = vi.spyOn(client, 'listResourcePacks');
    const listWorlds = vi.spyOn(client, 'listWorlds');
    client.listWorlds = listWorlds.mockResolvedValue([world()]);
    render(<App client={client} />);

    await screen.findByLabelText('Search packs');
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
    expect(await screen.findByRole('button', { name: /Survival 1.17/ })).toBeInTheDocument();
    const packCalls = listPacks.mock.calls.length;
    const worldCalls = listWorlds.mock.calls.length;
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
    expect(screen.getByRole('button', { name: /Survival 1.17/ })).toBeInTheDocument();
    expect(screen.queryByText('Loading worlds…')).not.toBeInTheDocument();
    expect(listPacks).toHaveBeenCalledTimes(packCalls);
    expect(listWorlds).toHaveBeenCalledTimes(worldCalls);
  });

  it('refreshes Discover worlds and targets after switching game directories', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    await client.addGameDir('/dir-b/.minecraft');
    const worlds = vi.spyOn(client, 'listWorlds').mockImplementation(async () => [world({
      dir_name: client.config.active_game_dir === '/dir-b/.minecraft' ? 'b' : 'a',
      level_name: client.config.active_game_dir === '/dir-b/.minecraft' ? 'B world' : 'A world',
    })]);
    const targets = vi.spyOn(client, 'modrinthTargets').mockImplementation(async () => ({
      installed_versions: [client.config.active_game_dir === '/dir-b/.minecraft' ? '1.20' : '1.21.1'],
      preferred_launcher: client.config.active_game_dir === '/dir-b/.minecraft' ? '1.20' : '1.21.1',
      world_version: null,
    }));
    render(<App client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Discover' }));
    await user.click(screen.getByRole('tab', { name: 'Datapacks' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'demo' } });
    await user.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
    await user.click(await screen.findByRole('button', { name: 'Install…' }));
    await user.selectOptions(await screen.findByLabelText('Target world'), 'a');
    const oldWorldCalls = worlds.mock.calls.length;
    const oldTargetCalls = targets.mock.calls.length;
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Discover' }));
    expect(await screen.findByRole('option', { name: 'B world (b)' })).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'A world (a)' })).not.toBeInTheDocument();
    expect(screen.getByLabelText('Target world')).toHaveValue('');
    expect(worlds).toHaveBeenCalledTimes(oldWorldCalls + 1);
    expect(targets).toHaveBeenCalledTimes(oldTargetCalls + 1);
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByLabelText('Game version')).toHaveTextContent('1.20');
  });

  it('clears old packs and write controls before a new directory load resolves', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    await client.addGameDir('/dir-b/.minecraft');
    client.listResourcePacks = async () => ({ packs: [worldPack('Alpha')], count: 1, bytes_read: 0, elapsed_ms: 0 });
    const list = client.listResourcePacks;
    client.listResourcePacks = () => client.config.active_game_dir === '/dir-b/.minecraft'
      ? new Promise(() => {}) : list();
    const getStack = client.getResourceStack;
    client.getResourceStack = () => client.config.active_game_dir === '/dir-b/.minecraft'
      ? new Promise(() => {}) : getStack();
    render(<App client={client} />);
    const grid = await screen.findByRole('list', { name: 'Installed packs' });
    await user.click(within(grid).getByRole('button', { name: /Alpha/ }));
    expect(screen.getByRole('button', { name: 'Delete pack' })).toBeEnabled();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    expect(screen.queryAllByText('Alpha')).toHaveLength(0);
    expect(screen.queryByRole('button', { name: 'Delete pack' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Remove' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Import pack' })).toBeDisabled();
  });

  it('ignores old pack and stack reloads that settle after a directory switch', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    await client.addGameDir('/dir-b/.minecraft');
    const listing = { packs: [worldPack('Old pack')], count: 1, bytes_read: 0, elapsed_ms: 0 };
    let releasePacks = () => {};
    let releaseStack = () => {};
    client.listResourcePacks = () => client.config.active_game_dir === '/dir-b/.minecraft'
      ? Promise.resolve({ ...listing, packs: [worldPack('B pack')] })
      : new Promise((resolve) => { releasePacks = () => resolve(listing); });
    client.getResourceStack = () => client.config.active_game_dir === '/dir-b/.minecraft'
      ? Promise.resolve({ ...stackResponse(null), entries: [] })
      : new Promise((resolve) => { releaseStack = () => resolve(stackResponse(null)); });
    render(<App client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
    expect(await screen.findByText('B pack')).toBeInTheDocument();
    await act(async () => { releasePacks(); releaseStack(); });
    expect(screen.getByText('B pack')).toBeInTheDocument();
    expect(screen.queryByText('Old pack')).not.toBeInTheDocument();
    expect(screen.queryByText('Alpha.zip')).not.toBeInTheDocument();
  });

  it('ignores an old world reload and never lists the previous selection in a new directory', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    await client.addGameDir('/dir-b/.minecraft');
    let release = () => {};
    client.listWorlds = () => {
      if (client.config.active_game_dir === '/dir-b/.minecraft') return Promise.resolve([]);
      return new Promise((resolve) => { release = () => resolve([world({ level_name: 'Late A world' })]); });
    };
    const datapacks = vi.spyOn(client, 'listDatapacks');
    render(<App client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Worlds' }));
    expect(within(await screen.findByRole('navigation', { name: 'Worlds' })).getByText('Loading worlds…')).toBeInTheDocument();
    const oldCalls = datapacks.mock.calls.length;
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
    expect(await screen.findByText('No worlds yet. Launch Minecraft once to create one.')).toBeInTheDocument();
    await act(async () => release());
    expect(screen.getByText('No worlds yet. Launch Minecraft once to create one.')).toBeInTheDocument();
    expect(datapacks).toHaveBeenCalledTimes(oldCalls);
  });

  it('drops the previous world selection before loading a different directory', async () => {
    const user = userEvent.setup();
    const client = await shellClient();
    await client.addGameDir('/dir-b/.minecraft');
    client.listWorlds = async () => client.config.active_game_dir === '/dir-b/.minecraft'
      ? [world({ dir_name: 'first-b', level_name: 'B first' }), world()]
      : [world()];
    const datapacks = vi.spyOn(client, 'listDatapacks');
    render(<App client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Worlds' }));
    expect(await screen.findByRole('button', { name: /Survival 1.17/ })).toHaveAttribute('aria-current', 'true');
    await screen.findByRole('list', { name: 'Datapack stack' });
    const oldCalls = datapacks.mock.calls.length;
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Use' }));
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
    expect(await screen.findByRole('button', { name: /B first/ })).toHaveAttribute('aria-current', 'true');
    await waitFor(() => expect(datapacks.mock.calls.slice(oldCalls)).toEqual([['first-b']]));
  });

});
