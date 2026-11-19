import { useEffect, useState } from 'react';
import type { DataStackEntry, Pack } from '../../lib/types';
import { StatusNote } from '../packs/PackEmptyState';
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
  const brokenCount = rows.filter((row) => row.pack?.error).length;
  const brokenSummary =
    brokenCount === 0 ? '' : `${brokenCount} broken ${brokenCount === 1 ? 'datapack' : 'datapacks'}`;
  const [announcedBroken, setAnnouncedBroken] = useState('');
  useEffect(() => {
    setAnnouncedBroken(brokenSummary);
  }, [brokenSummary]);
  if (loading && rows.length === 0) {
    return <StatusNote>Loading datapacks…</StatusNote>;
  }
  if (rows.length === 0) {
    return <StatusNote>No datapacks yet. Import a zip or a folder to add one.</StatusNote>;
  }
  return (
    <>
      <p className="visually-hidden" role="status">
        {announcedBroken}
      </p>
      <ul className="datapack-grid" aria-label="Datapack stack" aria-busy={loading}>
        {rows.map((row, index) => {
          const name =
            row.entry.kind === 'file' ? row.entry.filename : row.entry.value;
          // Rows are keyed by position, not by value: `Enabled` may hold the
          // same `file/` value twice, and value keys would mis-render.
          const key = `${row.entry.value}#${index}`;
          const broken = row.pack?.error ?? null;
          const selectionId = broken?.code === 'pack_non_utf8_name' ? row.pack!.id : name;
          if (broken) {
            const errorId = `datapack-broken-${index}`;
            // A broken pack is an error card, but an *enabled* broken row is
            // still a live `level.dat` entry: it keeps Disable (and any move
            // that stays between `file/` entries) so a corrupt pack can be
            // switched off. A disabled broken pack has nothing to act on.
            const actionable = row.enabled && !row.locked;
            return (
              <li
                key={key}
                className={`datapack-card datapack-card-broken${selectedFilename === selectionId ? ' is-selected' : ''}`}
              >
                <button
                  type="button"
                  className="datapack-card-main"
                  onClick={() => onSelect(selectionId)}
                  aria-label={`${name}, broken`}
                  aria-describedby={errorId}
                >
                  <span className="datapack-name">{name}</span>
                  <span className="datapack-kind">Broken datapack</span>
                  <span id={errorId} className="datapack-error">{broken.message}</span>
                </button>
                {actionable ? (
                  <span className="datapack-actions">
                    <button
                      type="button"
                      className="button"
                      disabled={writesBlocked}
                      aria-label={`Disable ${name}`}
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
          const toggleLabel = row.enabled ? 'Disable' : 'Enable';
          return (
            <li
              key={key}
              className={`datapack-card${selectedFilename === selectionId ? ' is-selected' : ''}${row.locked ? ' is-locked' : ''}`}
            >
              <button
                type="button"
                className="datapack-card-main"
                onClick={() => onSelect(selectionId)}
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
                    aria-label={`${toggleLabel} ${name}`}
                    onClick={() => onToggle(name, !row.enabled)}
                  >
                    {toggleLabel}
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
    </>
  );
}
