import GameRunningBanner from '../packs/GameRunningBanner';
import { formatVersionLabel } from '../../lib/format';
import { hideBrokenImage, worldInitials } from '../../lib/avatar';
import type { GameRunning, World } from '../../lib/types';

interface WorldHeaderProps {
  world: World;
  /** From the app-level game guard poll. Undefined until the first check. */
  gameRunning?: GameRunning | null;
  /** No current app-level guard answer is unknown, so writes stay paused. */
  gameGuardUnknown?: boolean;
  /** The unknown state is a new folder's first check, still pending. */
  gameGuardChecking?: boolean;
  /** Stale `session.lock` files from the world's stack. Never blocking. */
  staleLocks?: string[];
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
  /** Re-check the guard, the world's listing, and the world list. */
  onRefresh?: () => void | Promise<void>;
}

/** The selected world's name, version, icon, lock state, and path. A locked
 *  world names its lock in a banner; the write controls below stay disabled. */
export default function WorldHeader({
  world,
  gameRunning,
  gameGuardUnknown = false,
  gameGuardChecking = false,
  staleLocks = [],
  refreshGameRunning,
  onRefresh,
}: WorldHeaderProps) {
  const running = gameRunning ?? null;
  const locked = world.world_locked;
  const refresh = onRefresh ?? refreshGameRunning;
  return (
    <header className="world-header">
      <span className="world-icon" aria-hidden="true">
        {world.icon ? (
          <img src={world.icon.url} alt="" onError={hideBrokenImage} />
        ) : (
          <span className="world-icon-fallback">{worldInitials(world.level_name)}</span>
        )}
      </span>
      <div className="world-header-text">
        <h2>{world.level_name}</h2>
        <p className="world-meta">
          {world.version ? (
            <>
              <span>Minecraft {world.version.name}</span>
              {world.version.data_format ? (
                <span> · data format {formatVersionLabel(world.version.data_format)}</span>
              ) : null}
            </>
          ) : (
            <span>Version unknown</span>
          )}
          <span className="world-path"> · {world.path}</span>
        </p>
        {world.version && !world.version.data_format ? (
          <p className="banner" role="status">
            {world.version.newest_known_version
              ? `Minecraft ${world.version.name} is newer than this build knows (newest known: ${world.version.newest_known_version}). Datapack compatibility is not checked.`
              : `Mine Utils does not know the data format of Minecraft ${world.version.name}. Datapack compatibility is not checked.`}
          </p>
        ) : null}
        {world.issue ? (
          <p className="error" role="alert">
            This world is read-only: {world.issue.message}
          </p>
        ) : null}
        {locked ? (
          <p className="banner" role="status">
            This world is locked ({locked.detail}). Imports are copied but left disabled until the
            lock releases. Toggles, reorders, deletes, and restores are disabled.{' '}
            <button type="button" className="button" onClick={() => void refresh?.()}>
              Re-check
            </button>
          </p>
        ) : running ? (
          <p className="banner" role="status">
            Minecraft is running ({running.detail}). Close it before Mine Utils writes level.dat.{' '}
            <button type="button" className="button" onClick={() => void refresh?.()}>
              Re-check
            </button>
          </p>
        ) : null}
        <GameRunningBanner
          running={null}
          guardUnknown={!locked && !running && gameGuardUnknown}
          checking={gameGuardChecking}
          staleLocks={staleLocks}
          supported={world.detection_supported}
          onRecheck={() => void refresh?.()}
        />
      </div>
    </header>
  );
}
