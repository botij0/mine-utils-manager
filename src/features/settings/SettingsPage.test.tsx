import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import type { Config } from '../../lib/types';
import { defaultConfig } from '../../lib/types';
import SettingsPage from './SettingsPage';
import { gameDir, makeFakeClient } from '../../test/fake-ipc';
import type { FakeClient } from '../../test/fake-ipc';

function Harness({ client }: { client: FakeClient }) {
  const [config, setConfig] = useState<Config>(defaultConfig());
  return (
    <SettingsPage
      client={client}
      config={config}
      gameRunning={null}
      onConfigChanged={setConfig}
    />
  );
}

describe('SettingsPage', () => {
  it('keeps the latest theme when an earlier directory reply arrives late', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([]);
    const add = client.addGameDir;
    let release!: () => void;
    const delayed = new Promise<void>((resolve) => { release = resolve; });
    client.addGameDir = async (path) => {
      const result = await add(path);
      await delayed;
      return result;
    };
    render(<Harness client={client} />);
    await user.type(screen.getByLabelText('Game dir path'), '/tmp/saves/one');
    await user.click(screen.getByRole('button', { name: 'Add' }));
    await user.click(screen.getByRole('radio', { name: 'Dark' }));
    release();
    await screen.findByText('/tmp/saves/one');
    await waitFor(() => expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked());
  });

  it('recovers from a timed out write and ignores its late reply', async () => {
    vi.useFakeTimers();
    try {
      const client = makeFakeClient([]);
      const add = client.addGameDir;
      let release!: () => void;
      const delayed = new Promise<void>((resolve) => { release = resolve; });
      client.addGameDir = async (path) => {
        const result = await add(path);
        await delayed;
        return result;
      };
      render(<Harness client={client} />);
      await act(async () => {});
      fireEvent.change(screen.getByLabelText('Game dir path'), { target: { value: '/tmp/saves/one' } });
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
      fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
      await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
      expect(screen.getByRole('alert')).toHaveTextContent('Settings write timed out');
      expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked();
      await act(async () => { release(); });
      expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked();
    } finally {
      vi.useRealTimers();
    }
  });

  it('refreshes a late folder write and retains its timeout after a later folder succeeds', async () => {
    vi.useFakeTimers();
    try {
      const client = makeFakeClient([]);
      const add = client.addGameDir;
      let release!: () => void;
      const delayed = new Promise<void>((resolve) => { release = resolve; });
      client.addGameDir = async (path) => {
        if (path.endsWith('/one')) await delayed;
        return add(path);
      };
      render(<Harness client={client} />);
      await act(async () => {});
      fireEvent.change(screen.getByLabelText('Game dir path'), { target: { value: '/tmp/saves/one' } });
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
      fireEvent.change(screen.getByLabelText('Game dir path'), { target: { value: '/tmp/saves/two' } });
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
      await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
      expect(screen.getByRole('alert')).toHaveTextContent('Settings write timed out');
      expect(screen.getByText('/tmp/saves/two')).toBeInTheDocument();
      await act(async () => { release(); });
      expect(screen.getByText('/tmp/saves/one')).toBeInTheDocument();
      expect(screen.getByRole('alert')).toHaveTextContent('Settings write timed out');
    } finally { vi.useRealTimers(); }
  });

  it('retains a theme timeout after a later theme succeeds', async () => {
    vi.useFakeTimers();
    try {
      const client = makeFakeClient([]);
      const setTheme = client.setTheme;
      let release!: () => void;
      const delayed = new Promise<void>((resolve) => { release = resolve; });
      client.setTheme = async (theme) => {
        if (theme === 'light') await delayed;
        return setTheme(theme);
      };
      render(<Harness client={client} />);
      await act(async () => {});
      fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
      fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
      await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
      expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked();
      expect(screen.getByRole('alert')).toHaveTextContent('Settings write timed out');
      await act(async () => { release(); });
    } finally { vi.useRealTimers(); }
  });

  it('keeps an added dir when the theme changes', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([
      gameDir('/home/demo/.minecraft', true, false),
    ]);
    render(<Harness client={client} />);

    await user.type(screen.getByLabelText('Game dir path'), '/tmp/saves/one');
    await user.click(screen.getByRole('button', { name: 'Add' }));
    await screen.findByText('/tmp/saves/one');

    await user.click(screen.getByRole('radio', { name: 'Dark' }));

    expect(client.config.theme).toBe('dark');
    expect(client.config.game_dirs.map((dir) => dir.path)).toContain(
      '/tmp/saves/one',
    );
  });
});
