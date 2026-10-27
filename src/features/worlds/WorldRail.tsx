import type { World } from '../../lib/types';
import { hideBrokenImage, worldInitials } from '../../lib/avatar';

interface WorldRailProps {
  worlds: World[];
  loading: boolean;
  selected: string | null;
  onSelect: (dirName: string) => void;
}

/** The world list: icon, name, and version per world. Error cards stay in the
 *  rail so a corrupt world is visible without hiding the rest. */
export default function WorldRail({ worlds, loading, selected, onSelect }: WorldRailProps) {
  if (loading) {
    return (
      <nav className="world-rail" aria-label="Worlds">
        <p className="empty-note">Loading worlds…</p>
      </nav>
    );
  }
  if (worlds.length === 0) {
    return (
      <nav className="world-rail" aria-label="Worlds">
        <p className="empty-note">No worlds yet. Launch Minecraft once to create one.</p>
      </nav>
    );
  }
  return (
    <nav className="world-rail" aria-label="Worlds">
      <ul>
        {worlds.map((world) => (
          <li key={world.dir_name}>
            <button
              type="button"
              className={`world-rail-item${selected === world.dir_name ? ' is-selected' : ''}${world.issue ? ' has-error' : ''}`}
              aria-current={selected === world.dir_name ? 'true' : undefined}
              onClick={() => onSelect(world.dir_name)}
            >
              <span className="world-rail-icon" aria-hidden="true">
                {world.icon ? (
                  <img src={world.icon.url} alt="" onError={hideBrokenImage} />
                ) : (
                  <span className="world-rail-fallback">{worldInitials(world.level_name)}</span>
                )}
              </span>
              <span className="world-rail-text">
                <span className="world-rail-name">{world.level_name}</span>
                <span className="world-rail-version">
                  {world.version ? world.version.name : world.issue ? 'Unreadable' : 'Unknown'}
                </span>
              </span>
              {world.issue ? (
                <span className="world-rail-error" title={world.issue.message}>
                  !
                </span>
              ) : null}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}
