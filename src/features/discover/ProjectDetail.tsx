import type {
  IpcError,
  ModrinthProject,
  ModrinthSearchHit,
  ModrinthVersion,
} from '../../lib/types';
import VersionPicker from './VersionPicker';
import { formatCount } from './ProjectGrid';

interface ProjectDetailProps {
  hit: ModrinthSearchHit;
  project: ModrinthProject | null;
  versions: ModrinthVersion[];
  loading: boolean;
  error: IpcError | null;
  selectedVersionId: string | null;
  onSelectVersion: (id: string) => void;
  preferredId: string | null;
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
  preferredId,
  targetLabel,
  onInstall,
  onClose,
}: ProjectDetailProps) {
  const selected = versions.find((version) => version.id === selectedVersionId) ?? null;
  const file = selected?.files.find((item) => item.primary) ?? selected?.files[0] ?? null;
  return (
    <div className="sheet-backdrop">
      <section className="sheet" role="dialog" aria-label={hit.title}>
        <div className="pack-detail-head">
          {hit.icon_url ? (
            <img className="pack-detail-icon" src={hit.icon_url} alt="" loading="lazy" />
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
            preferredId={preferredId}
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
      </section>
    </div>
  );
}
