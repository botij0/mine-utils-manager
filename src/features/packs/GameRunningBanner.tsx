import type { GameRunning } from '../../lib/types';

interface GameRunningBannerProps {
  running: GameRunning | null;
  staleLocks: string[];
  supported: boolean;
  onRecheck: () => void;
}

/**
 * Blocks every write while Minecraft is open, naming the lock path or the
 * process so the operator knows what to close. A stale `session.lock` nobody
 * holds is only noted; it never blocks.
 */
export default function GameRunningBanner({
  running,
  staleLocks,
  supported,
  onRecheck,
}: GameRunningBannerProps) {
  if (running) {
    return (
      <div className="banner banner-danger" role="alert">
        <div>
          <strong>Minecraft is running.</strong> Writes are paused: {running.detail}.
        </div>
        <button type="button" className="button" onClick={onRecheck}>
          Recheck
        </button>
      </div>
    );
  }

  if (!supported) {
    return (
      <div className="banner" role="status">
        Game lock detection is not supported here, so writes are never blocked
        automatically.
      </div>
    );
  }

  if (staleLocks.length > 0) {
    return (
      <div className="banner" role="status" aria-label="Stale session lock">
        A stale session lock at {staleLocks.join(', ')} holds no process. Writes
        are allowed.
      </div>
    );
  }

  return null;
}
