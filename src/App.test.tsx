import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { IpcClient } from './lib/ipc';
import type { GameRunning } from './lib/types';
import { IpcError } from './lib/types';
import { datapack, makeFakeClient, gameDir, stack, world } from './test/fake-ipc';
import type { FakeClient } from './test/fake-ipc';

afterEach(() => {
  vi.useRealTimers();
  Reflect.deleteProperty(document, 'hidden');
  window.location.hash = '';
});

/** Counts guard requests and records the folder each one was sent for. */
function countGuardChecks(client: FakeClient): (string | null)[] {
  const checkedDirs: (string | null)[] = [];
  const guard = client.getGameGuard;
  client.getGameGuard = () => {
    checkedDirs.push(client.config.active_game_dir);
    return guard();
  };
  return checkedDirs;
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
    detection_supported: true,
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
      return { running, stale_locks: [], detection_supported: true };
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
    const checkedDirs = countGuardChecks(client);
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
    const checkedDirs = countGuardChecks(client);
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await waitFor(() => expect(checkedDirs).toEqual(['/home/demo/.minecraft']));
    await user.click(await screen.findByRole('button', { name: 'Use' }));

    await waitFor(() =>
      expect(checkedDirs).toEqual(['/home/demo/.minecraft', '/home/demo/prism']),
    );
  });

  it('sends no guard request while the window is hidden and checks once it is shown', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const client = await shellClient();
    const checkedDirs = countGuardChecks(client);
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
});
