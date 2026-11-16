import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Config } from '../../lib/types';
import GameDirSettings from './GameDirSettings';
import { gameDir, makeFakeClient } from '../../test/fake-ipc';

const DETECTED = gameDir('/home/demo/.minecraft', true, false);

describe('GameDirSettings', () => {
  it('adds a dir to the list and reports the config', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([DETECTED]);
    const onConfigChanged = vi.fn();
    render(
      <GameDirSettings
        client={client}
        activePath={null}
        onConfigChanged={onConfigChanged}
      />,
    );

    await user.type(screen.getByLabelText('Game dir path'), '/tmp/saves/one');
    await user.click(screen.getByRole('button', { name: 'Add' }));

    expect(await screen.findByText('/tmp/saves/one')).toBeInTheDocument();
    expect(onConfigChanged).toHaveBeenCalled();
    const last = onConfigChanged.mock.calls.at(-1)?.[0] as Config;
    expect(last.game_dirs.map((dir) => dir.path)).toContain('/tmp/saves/one');
  });

  it('removes an added dir from the list', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([
      DETECTED,
      gameDir('/tmp/saves/two', true, true),
    ]);
    const onConfigChanged = vi.fn();
    render(
      <GameDirSettings
        client={client}
        activePath={null}
        onConfigChanged={onConfigChanged}
      />,
    );

    await screen.findByText('/tmp/saves/two');
    await user.click(screen.getByRole('button', { name: 'Remove' }));

    await waitFor(() =>
      expect(screen.queryByText('/tmp/saves/two')).not.toBeInTheDocument(),
    );
    expect(screen.getByText('/home/demo/.minecraft')).toBeInTheDocument();
    const last = onConfigChanged.mock.calls.at(-1)?.[0] as Config;
    expect(last.game_dirs.map((dir) => dir.path)).not.toContain(
      '/tmp/saves/two',
    );
  });

  it('removes a stored dir that is missing on disk', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([
      DETECTED,
      gameDir('/tmp/saves/missing', false, true),
    ]);
    render(
      <GameDirSettings
        client={client}
        activePath={null}
        onConfigChanged={() => {}}
      />,
    );

    const missing = await screen.findByText('/tmp/saves/missing');
    const row = missing.closest('li');
    expect(row).not.toBeNull();
    expect(within(row!).queryByRole('button', { name: 'Use' })).toBeNull();
    await user.click(within(row!).getByRole('button', { name: 'Remove' }));

    await waitFor(() =>
      expect(screen.queryByText('/tmp/saves/missing')).not.toBeInTheDocument(),
    );
  });

  it('names each folder row Use and Remove after its path', async () => {
    const client = makeFakeClient([
      DETECTED,
      gameDir('/tmp/saves/two', true, true),
    ]);
    render(
      <GameDirSettings
        client={client}
        activePath={null}
        onConfigChanged={() => {}}
      />,
    );

    const added = (await screen.findByText('/tmp/saves/two')).closest('li');
    expect(added).not.toBeNull();
    expect(within(added!).getByRole('button', { name: 'Use /tmp/saves/two' })).toBeEnabled();
    expect(within(added!).getByRole('button', { name: 'Remove /tmp/saves/two' })).toBeEnabled();
    const detected = screen.getByText('/home/demo/.minecraft').closest('li');
    expect(within(detected!).getByRole('button', { name: 'Use /home/demo/.minecraft' })).toBeEnabled();
  });

  it('shows an inline error and keeps the list when the path is not a game dir', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([DETECTED]);
    const onConfigChanged = vi.fn();
    render(
      <GameDirSettings
        client={client}
        activePath={null}
        onConfigChanged={onConfigChanged}
      />,
    );

    await user.type(
      screen.getByLabelText('Game dir path'),
      '/tmp/not-a-game-dir',
    );
    await user.click(screen.getByRole('button', { name: 'Add' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'invalid game dir /tmp/not-a-game-dir: expected one of versions, saves, or resourcepacks',
    );
    expect(screen.getByText('/home/demo/.minecraft')).toBeInTheDocument();
    expect(screen.queryByText('/tmp/not-a-game-dir')).not.toBeInTheDocument();
    expect(onConfigChanged).not.toHaveBeenCalled();
  });
});
