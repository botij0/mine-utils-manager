import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import GameRunningBanner from './GameRunningBanner';

describe('GameRunningBanner', () => {
  it('mounts a fresh alert when a pending check turns into a failure', () => {
    const props = { running: null, staleLocks: [], supported: true, onRecheck: () => {} };
    const { rerender } = render(<GameRunningBanner {...props} guardPause="checking" />);
    const status = screen.getByRole('status');
    expect(status).toHaveTextContent(
      'Checking whether Minecraft is running; writes are paused until the check lands.',
    );

    rerender(<GameRunningBanner {...props} guardPause="unknown" />);

    expect(screen.getByRole('alert')).toHaveTextContent(/^Game guard unavailable\./);
    expect(status).not.toBeInTheDocument();
  });
});
