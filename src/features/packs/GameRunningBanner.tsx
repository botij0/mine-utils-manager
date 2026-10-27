import type { GameRunning } from '../../lib/types';

interface GameRunningBannerProps {
  running: GameRunning | null;
  /** True when the last guard check failed, so the state is unknown. */
  guardUnknown: boolean;
  staleLocks: string[];
  supported: boolean;
  onRecheck: () => void;
}

/**
 * Blocks every write while Minecraft is open, naming the lock path or the
 * process so the operator knows what to close. A failed check is unknown, so it
 * pauses writes instead of claiming the game stopped. A stale `session.lock`
 * nobody holds is only noted; it never blocks.
 */
export default function GameRunningBanner({
  running,
  guardUnknown,
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

  if (guardUnknown) {
    return (
      <div className="banner banner-danger" role="alert">
        <div>
          <strong>Game guard unavailable.</strong> Mine Utils could not check
          whether Minecraft is running, so writes are paused.
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
    const [first, ...rest] = staleLocks;
    return (
      <div className="banner" role="status" aria-label="Stale session lock">
        <div>
          A stale session lock at <span className="banner-path">{first}</span>{' '}
          holds no process.
          {rest.length > 0
            ? ` ${rest.length} more stale lock${rest.length === 1 ? '' : 's'}.`
            : ''}{' '}
          Writes are allowed.
        </div>
      </div>
    );
  }

  return null;
}
