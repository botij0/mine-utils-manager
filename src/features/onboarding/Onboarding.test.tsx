import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import Onboarding from './Onboarding';
import { gameDir, makeFakeClient } from '../../test/fake-ipc';

describe('Onboarding', () => {
  it('lists one dir and finishes when it is picked', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([gameDir('/home/demo/.minecraft', true, true)]);
    const onDone = vi.fn();
    render(<Onboarding client={client} onDone={onDone} />);

    expect(await screen.findByText('/home/demo/.minecraft')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Use /home/demo/.minecraft' }));
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(client.config.active_game_dir).toBe('/home/demo/.minecraft');
  });

  it('lists many dirs for the operator to choose from', async () => {
    const client = makeFakeClient([
      gameDir('/home/demo/.minecraft', true, true),
      gameDir('/home/demo/prism/1.21/.minecraft', true, true),
    ]);
    const onDone = vi.fn();
    render(<Onboarding client={client} onDone={onDone} />);

    expect(await screen.findByText('/home/demo/.minecraft')).toBeInTheDocument();
    expect(screen.getByText('/home/demo/prism/1.21/.minecraft')).toBeInTheDocument();
    expect(onDone).not.toHaveBeenCalled();
  });

  it('shows an empty state when no dir is found', async () => {
    const client = makeFakeClient([]);
    const onDone = vi.fn();
    render(<Onboarding client={client} onDone={onDone} />);

    expect(await screen.findByText(/No game folders found/)).toBeInTheDocument();
    expect(onDone).not.toHaveBeenCalled();
  });

  it('skips without touching the config', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([gameDir('/home/demo/.minecraft', true, true)]);
    const onDone = vi.fn();
    render(<Onboarding client={client} onDone={onDone} />);

    await user.click(await screen.findByRole('button', { name: 'Skip for now' }));
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(client.config.active_game_dir).toBeNull();
  });

  it('explains the safety rules up front', async () => {
    const client = makeFakeClient([]);
    render(<Onboarding client={client} onDone={() => {}} />);

    expect(await screen.findByText(/Writes are blocked while the/)).toBeInTheDocument();
  });
});
