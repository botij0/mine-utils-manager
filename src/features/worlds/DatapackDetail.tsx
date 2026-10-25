import type { Pack } from '../../lib/types';
import CompatibilityChip from './CompatibilityChip';

interface DatapackDetailProps {
  pack: Pack | null;
  filename: string | null;
  enabled: boolean;
  locked: boolean;
  /** True when the `file/` entry has no pack on disk. */
  missing: boolean;
  warning: string | null;
  writesBlocked: boolean;
  busy: boolean;
  onToggle: (filename: string, enabled: boolean) => void;
  onDelete: (path: string) => void;
  onRemoveReference: (filename: string) => void;
}

/** The selected datapack: path, format range, compatibility, and actions. */
export default function DatapackDetail({
  pack,
  filename,
  enabled,
  locked,
  missing,
  warning,
  writesBlocked,
  busy,
  onToggle,
  onDelete,
  onRemoveReference,
}: DatapackDetailProps) {
  if (!filename) {
    return <p className="empty-note">Select a datapack to see its details.</p>;
  }
  if (locked) {
    return (
      <section className="datapack-detail" aria-label="Datapack details">
        <h3>{filename}</h3>
        <p className="panel-note">This entry is locked by Minecraft and cannot change.</p>
      </section>
    );
  }
  if (pack?.error) {
    return (
      <section className="datapack-detail" aria-label="Datapack details">
        <h3>{filename}</h3>
        <p className="error" role="alert">
          This datapack cannot be read: {pack.error.message}
        </p>
        <div className="datapack-detail-actions">
          <button
            type="button"
            className="button button-danger"
            disabled={writesBlocked || busy}
            onClick={() => onDelete(pack.path)}
          >
            Delete
          </button>
        </div>
      </section>
    );
  }
  if (missing) {
    return (
      <section className="datapack-detail" aria-label="Datapack details">
        <h3>{filename}</h3>
        <p className="empty-note">
          This entry points at a file that is no longer in datapacks. Enabling it would only write
          a dead reference.
        </p>
        <div className="datapack-detail-actions">
          <button
            type="button"
            className="button"
            disabled={writesBlocked || busy}
            onClick={() => onRemoveReference(filename)}
          >
            Remove reference
          </button>
        </div>
      </section>
    );
  }
  return (
    <section className="datapack-detail" aria-label="Datapack details">
      <h3>{filename}</h3>
      {pack ? (
        <dl>
          <div>
            <dt>Path</dt>
            <dd>{pack.path}</dd>
          </div>
          <div>
            <dt>Kind</dt>
            <dd>{pack.kind}</dd>
          </div>
          <div>
            <dt>Built for</dt>
            <dd>{pack.game_label ?? 'Unknown'}</dd>
          </div>
          <div>
            <dt>State</dt>
            <dd>{enabled ? 'Enabled' : 'Disabled'}</dd>
          </div>
        </dl>
      ) : (
        <p className="empty-note">Select a datapack to see its details.</p>
      )}
      <CompatibilityChip warning={warning} />
      {warning ? <p className="panel-note">{warning}</p> : null}
      <div className="datapack-detail-actions">
        <button
          type="button"
          className="button"
          disabled={writesBlocked || busy}
          onClick={() => onToggle(filename, !enabled)}
        >
          {enabled ? 'Disable' : 'Enable'}
        </button>
        {pack ? (
          <button
            type="button"
            className="button button-danger"
            disabled={writesBlocked || busy}
            onClick={() => onDelete(pack.path)}
          >
            Delete
          </button>
        ) : null}
      </div>
    </section>
  );
}
