import type {
  IpcError,
  ModrinthProject,
  ModrinthSearchHit,
  ModrinthVersion,
} from '../../lib/types';
import DialogShell from './DialogShell';
import VersionPicker from './VersionPicker';
import { formatCount } from '../../lib/avatar';
import { isTrustedIconUrl } from './icon';

interface ProjectDetailProps {
  hit: ModrinthSearchHit;
  project: ModrinthProject | null;
  versions: ModrinthVersion[];
  loading: boolean;
  error: IpcError | null;
  selectedVersionId: string | null;
  onSelectVersion: (id: string) => void;
  /** The version compatible with the active game/world, if any. Only this
   *  id renders the "(compatible)" label; the picker still lists all. */
  compatibleId: string | null;
  targetLabel: string | null;
  onInstall: () => void;
  onClose: () => void;
}

export default function ProjectDetail({
  hit,
  project,
  versions,
  loading,
  error,
  selectedVersionId,
  onSelectVersion,
  compatibleId,
  targetLabel,
  onInstall,
  onClose,
}: ProjectDetailProps) {
  const selected = versions.find((version) => version.id === selectedVersionId) ?? null;
  const file = selected?.files.find((item) => item.primary) ?? selected?.files[0] ?? null;
  return (
    <DialogShell label={hit.title} onClose={onClose}>
      <div className="pack-detail-head">
        {isTrustedIconUrl(hit.icon_url) ? (
          <img
            className="pack-detail-icon"
            src={hit.icon_url}
            alt=""
            loading="lazy"
            referrerPolicy="no-referrer"
          />
        ) : null}
        <div>
          <h3>{hit.title}</h3>
          <p className="pack-detail-sub">
            {formatCount(hit.downloads)} downloads · {formatCount(hit.follows)} follows
          </p>
        </div>
        <button type="button" className="button button-icon" onClick={onClose} aria-label="Close details">
          ×
        </button>
      </div>
      <p className="pack-detail-desc">{project?.body || hit.description}</p>
      {loading ? (
        <p className="empty-note" role="status">
          Loading versions…
        </p>
      ) : null}
      {error ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}
      {!loading && !error ? (
        <VersionPicker
          versions={versions}
          selectedId={selectedVersionId}
          onSelect={onSelectVersion}
          compatibleId={compatibleId}
          targetLabel={targetLabel}
        />
      ) : null}
      {file ? (
        <dl className="pack-facts">
          <div>
            <dt>File</dt>
            <dd>{file.filename}</dd>
          </div>
          <div>
            <dt>Hash</dt>
            <dd>SHA-512 verified on download</dd>
          </div>
        </dl>
      ) : null}
      <div className="pack-confirm-actions">
        <button type="button" className="button" onClick={onClose}>
          Back
        </button>
        <button
          type="button"
          className="button button-primary"
          disabled={!selected || !file}
          onClick={onInstall}
        >
          Install…
        </button>
      </div>
    </DialogShell>
  );
}
