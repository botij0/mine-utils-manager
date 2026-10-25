import type { DataStackEntry, Pack } from '../../lib/types';
import CompatibilityChip from './CompatibilityChip';

export interface DatapackRow {
  /** The installed pack, when its file is still on disk. */
  pack: Pack | null;
  entry: DataStackEntry;
  enabled: boolean;
  /** Foreign entries are locked: they never toggle or move. */
  locked: boolean;
  warning: string | null;
  filename: string;
  /** False at the top of the enabled stack or under a locked entry. */
  canMoveUp: boolean;
  /** False at the bottom of the enabled stack or above a locked entry. */
  canMoveDown: boolean;
}

interface DatapackGridProps {
  rows: DatapackRow[];
  loading: boolean;
  selectedFilename: string | null;
  onSelect: (filename: string) => void;
  writesBlocked: boolean;
  onToggle: (filename: string, enabled: boolean) => void;
  onMove: (filename: string, direction: -1 | 1) => void;
}

/** The enabled stack, top-first: the first card loads last and wins. Foreign
 *  entries render locked; only `file/` entries toggle or move. Disabled packs
 *  follow with an enable action. */
export default function DatapackGrid({
  rows,
  loading,
  selectedFilename,
  onSelect,
  writesBlocked,
  onToggle,
  onMove,
}: DatapackGridProps) {
  if (loading) {
    return <p className="empty-note">Loading datapacks…</p>;
  }
  if (rows.length === 0) {
    return <p className="empty-note">No datapacks yet. Import a zip or a folder to add one.</p>;
  }
  return (
    <ul className="datapack-grid" aria-label="Datapack stack">
      {rows.map((row) => {
        const name =
          row.entry.kind === 'file' ? row.entry.filename : row.entry.value;
        const key = row.entry.value;
        return (
          <li
            key={key}
            className={`datapack-card${selectedFilename === name ? ' is-selected' : ''}${row.locked ? ' is-locked' : ''}`}
          >
            <button
              type="button"
              className="datapack-card-main"
              onClick={() => onSelect(name)}
              aria-label={`${name}${row.enabled ? ', enabled' : ', disabled'}`}
            >
              <span className="datapack-name">{name}</span>
              {row.pack ? (
                <span className="datapack-kind">{row.pack.game_label ?? row.pack.kind}</span>
              ) : row.entry.kind === 'file' ? (
                <span className="datapack-kind">File missing from datapacks</span>
              ) : null}
              <CompatibilityChip warning={row.warning} />
            </button>
            {row.locked ? (
              <span className="datapack-lock" title="Locked by Minecraft; it cannot move or toggle">
                Locked
              </span>
            ) : (
              <span className="datapack-actions">
                <button
                  type="button"
                  className="button"
                  disabled={writesBlocked}
                  onClick={() => onToggle(name, !row.enabled)}
                >
                  {row.enabled ? 'Disable' : 'Enable'}
                </button>
                {row.enabled ? (
                  <>
                    <button
                      type="button"
                      className="button"
                      disabled={writesBlocked || !row.canMoveUp}
                      title={row.canMoveUp ? undefined : 'Locked entries cannot move'}
                      aria-label={`Move ${name} up`}
                      onClick={() => onMove(name, -1)}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className="button"
                      disabled={writesBlocked || !row.canMoveDown}
                      title={row.canMoveDown ? undefined : 'Locked entries cannot move'}
                      aria-label={`Move ${name} down`}
                      onClick={() => onMove(name, 1)}
                    >
                      ↓
                    </button>
                  </>
                ) : null}
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}
