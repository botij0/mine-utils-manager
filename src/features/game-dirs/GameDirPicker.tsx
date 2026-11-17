import type { DiscoverySource, GameDirEntry } from '../../lib/types';

const SOURCE_LABELS: Record<DiscoverySource, string> = {
  std: 'Vanilla',
  flatpak: 'Flatpak',
  prism_launcher: 'Prism Launcher',
  multi_mc: 'MultiMC',
  manual: 'Added',
};

interface GameDirPickerProps {
  entries: GameDirEntry[];
  activePath: string | null;
  onSelect: (path: string) => void;
  onRemove?: (path: string) => void;
}

export default function GameDirPicker({
  entries,
  activePath,
  onSelect,
  onRemove,
}: GameDirPickerProps) {
  return (
    <ul className="dir-list">
      {entries.map((entry) => {
        const isActive = entry.path === activePath;
        return (
          <li key={entry.path} className={isActive ? 'dir-row is-active' : 'dir-row'}>
            <div className="dir-info">
              <span className="dir-path">{entry.path}</span>
              <span className="badges">
                <span className="badge">{SOURCE_LABELS[entry.source]}</span>
                <span className={entry.present ? 'badge ok' : 'badge warn'}>
                  {entry.present ? 'On disk' : 'Missing'}
                </span>
                {isActive ? <span className="badge active">Active</span> : null}
              </span>
            </div>
            <div className="row-actions">
              {isActive || !entry.present ? null : (
                <button
                  type="button"
                  className="button"
                  aria-label={`Use ${entry.path}`}
                  onClick={() => onSelect(entry.path)}
                >
                  Use
                </button>
              )}
              {onRemove && entry.stored ? (
                <button
                  type="button"
                  className="button"
                  aria-label={`Remove ${entry.path}`}
                  onClick={() => onRemove(entry.path)}
                >
                  Remove
                </button>
              ) : null}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
