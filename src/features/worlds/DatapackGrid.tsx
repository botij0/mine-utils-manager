import type { DataStackEntry, Pack } from '../../lib/types';
import CompatibilityChip from './CompatibilityChip';

export interface DatapackRow {
  /** The installed pack, when its file is still on disk. */
  pack: Pack | null;
  entry: DataStackEntry;
  enabled: boolean;
  /** Foreign entries are locked: they never toggle or move. */
  locked: boolean;
  /** True when the `file/` entry has no pack on disk. */
  missing: boolean;
  warning: string | null;
  filename: string;
  /** False at the top of the enabled stack or under a locked entry. */
  canMoveUp: boolean;
  /** False at the bottom of the enabled stack or above a locked entry. */
  canMoveDown: boolean;
}

interface DatapackGridProps {
  rows: DatapackRow[];
  /** True while the first listing loads. Rows stay visible on every
   *  later refresh; the list is never replaced by a spinner mid-write. */
  loading: boolean;
  selectedFilename: string | null;
  onSelect: (filename: string) => void;
  writesBlocked: boolean;
  onToggle: (filename: string, enabled: boolean) => void;
  onMove: (index: number, direction: -1 | 1) => void;
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
  if (loading && rows.length === 0) {
    return <p className="empty-note">Loading datapacks…</p>;
  }
  if (rows.length === 0) {
    return <p className="empty-note">No datapacks yet. Import a zip or a folder to add one.</p>;
  }
  return (
    <ul className="datapack-grid" aria-label="Datapack stack" aria-busy={loading}>
      {rows.map((row, index) => {
        const name =
          row.entry.kind === 'file' ? row.entry.filename : row.entry.value;
        // Rows are keyed by position, not by value: `Enabled` may hold the
        // same `file/` value twice, and value keys would mis-render.
        const key = `${row.entry.value}#${index}`;
        const broken = row.pack?.error ?? null;
        if (broken) {
          // A broken pack is an error card, but an *enabled* broken row is
          // still a live `level.dat` entry: it keeps Disable (and any move
          // that stays between `file/` entries) so a corrupt pack can be
          // switched off. A disabled broken pack has nothing to act on.
          const actionable = row.enabled && !row.locked;
          return (
            <li
              key={key}
              className={`datapack-card datapack-card-broken${selectedFilename === name ? ' is-selected' : ''}`}
            >
              <button
                type="button"
                className="datapack-card-main"
                onClick={() => onSelect(name)}
                aria-label={`${name}, broken`}
              >
                <span className="datapack-name">{name}</span>
                <span className="datapack-kind">Broken datapack</span>
                <span className="datapack-error" role="alert">
                  {broken.message}
                </span>
              </button>
              {actionable ? (
                <span className="datapack-actions">
                  <button
                    type="button"
                    className="button"
                    disabled={writesBlocked}
                    onClick={() => onToggle(name, false)}
                  >
                    Disable
                  </button>
                  <button
                    type="button"
                    className="button"
                    disabled={writesBlocked || !row.canMoveUp}
                    title={row.canMoveUp ? undefined : 'Locked entries cannot move'}
                    aria-label={`Move ${name} up`}
                    onClick={() => onMove(index, -1)}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="button"
                    disabled={writesBlocked || !row.canMoveDown}
                    title={row.canMoveDown ? undefined : 'Locked entries cannot move'}
                    aria-label={`Move ${name} down`}
                    onClick={() => onMove(index, 1)}
                  >
                    ↓
                  </button>
                </span>
              ) : (
                <span className="datapack-lock" title={broken.message}>
                  Error
                </span>
              )}
            </li>
          );
        }
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
                  disabled={writesBlocked || row.missing}
                  title={row.missing ? 'The file is gone; remove the reference instead' : undefined}
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
                      onClick={() => onMove(index, -1)}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className="button"
                      disabled={writesBlocked || !row.canMoveDown}
                      title={row.canMoveDown ? undefined : 'Locked entries cannot move'}
                      aria-label={`Move ${name} down`}
                      onClick={() => onMove(index, 1)}
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
