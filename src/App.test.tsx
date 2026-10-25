import { act, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { IpcClient } from './lib/ipc';
import type { GameRunning } from './lib/types';
import { IpcError } from './lib/types';
import { makeFakeClient } from './test/fake-ipc';

afterEach(() => {
  vi.useRealTimers();
});

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
    const client = makeFakeClient([]);
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
    const client = makeFakeClient([]);
    const running: GameRunning = {
      kind: 'process',
      detail: 'pid 4242 runs java',
      lock_path: null,
      pid: 4242,
      command: 'java -jar client.jar',
    };
    let fail = false;
    client.getResourceStack = async () => {
      if (fail) throw new IpcError('io', 'guard read failed');
      return stackResponse(running);
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
    const client = makeFakeClient([]);
    client.getResourceStack = async () => {
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
});
