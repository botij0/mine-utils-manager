import type { GameRunning, World } from '../../lib/types';

interface WorldHeaderProps {
  world: World;
  /** From the app-level game guard poll. Undefined until the first check. */
  gameRunning?: GameRunning | null;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
}

/** The selected world's name, version, icon, lock state, and path. A locked
 *  world names its lock in a banner; the write controls below stay disabled. */
export default function WorldHeader({ world, gameRunning, refreshGameRunning }: WorldHeaderProps) {
  const running = gameRunning !== undefined ? gameRunning : world.running;
  const locked = world.world_locked;
  return (
    <header className="world-header">
      <span className="world-icon" aria-hidden="true">
        {world.icon ? (
          <img src={world.icon.url} alt="" onError={hideBrokenIcon} />
        ) : (
          <span className="world-icon-fallback">{initials(world.level_name)}</span>
        )}
      </span>
      <div className="world-header-text">
        <h2>{world.level_name}</h2>
        <p className="world-meta">
          {world.version ? (
            <>
              <span>Minecraft {world.version.name}</span>
              {world.version.data_format ? (
                <span>
                  {' '}
                  · data format {world.version.data_format.major}.{world.version.data_format.minor}
                </span>
              ) : null}
            </>
          ) : (
            <span>Version unknown</span>
          )}
          <span className="world-path"> · {world.path}</span>
        </p>
        {world.issue ? (
          <p className="error" role="alert">
            This world is read-only: {world.issue.message}
          </p>
        ) : null}
        {locked ? (
          <p className="banner" role="status">
            This world is locked ({locked.detail}). Imports are copied but left disabled until the
            lock releases. Toggles, reorders, deletes, and restores are disabled.{' '}
            <button
              type="button"
              className="button"
              onClick={() => void refreshGameRunning?.()}
            >
              Re-check
            </button>
          </p>
        ) : running ? (
          <p className="banner" role="status">
            Minecraft is running ({running.detail}). Close it before Mine Utils writes level.dat.{' '}
            <button
              type="button"
              className="button"
              onClick={() => void refreshGameRunning?.()}
            >
              Re-check
            </button>
          </p>
        ) : null}
      </div>
    </header>
  );
}

function initials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  return trimmed.slice(0, 2).toUpperCase();
}

function hideBrokenIcon(event: React.SyntheticEvent<HTMLImageElement>) {
  event.currentTarget.style.display = 'none';
}
