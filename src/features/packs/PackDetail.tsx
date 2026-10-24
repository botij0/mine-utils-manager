import { useEffect, useState } from 'react';
import type { Pack } from '../../lib/types';
import {
  formatRangeLabel,
  formatVersionLabel,
  packFormatLabel,
  packGameLabel,
  packKindLabel,
} from '../../lib/format';

interface PackDetailProps {
  pack: Pack | null;
  busy: boolean;
  onDelete: (pack: Pack) => Promise<void>;
}

function versionOrDash(value: { major: number; minor: number } | null): string {
  return value ? formatVersionLabel(value) : 'Not declared';
}

export default function PackDetail({ pack, busy, onDelete }: PackDetailProps) {
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    setConfirming(false);
  }, [pack?.id]);

  if (!pack) {
    return (
      <aside className="pack-detail" aria-label="Pack details">
        <p className="pack-detail-hint">
          Select a pack to see its metadata, format range, and file path.
        </p>
      </aside>
    );
  }

  const range = pack.format_range;
  return (
    <aside className="pack-detail" aria-label="Pack details">
      <div className="pack-detail-head">
        <span className="pack-detail-icon">
          {pack.icon ? <img src={pack.icon.url} alt="" /> : <span aria-hidden="true">PK</span>}
        </span>
        <div>
          <h3>{pack.name}</h3>
          <p className="pack-detail-sub">
            {packKindLabel(pack.kind)} pack, {pack.location === 'zip' ? 'zip archive' : 'folder'}
          </p>
        </div>
      </div>

      {pack.error ? (
        <p className="error" role="alert">
          {pack.error.message}
        </p>
      ) : (
        <p className="pack-detail-desc">
          {pack.description || 'No description in pack.mcmeta.'}
        </p>
      )}

      <dl className="pack-facts">
        <dt>Game versions</dt>
        <dd>{packGameLabel(pack)}</dd>
        <dt>Format range</dt>
        <dd>{packFormatLabel(pack)}</dd>
        <dt>Declared format</dt>
        <dd>{versionOrDash(pack.meta.pack_format)}</dd>
        <dt>Min and max</dt>
        <dd>
          {versionOrDash(pack.meta.min_format)} to {versionOrDash(pack.meta.max_format)}
        </dd>
        <dt>Supported</dt>
        <dd>{formatRangeLabel(pack.meta.supported_formats)}</dd>
        <dt>Read from disk</dt>
        <dd>{Math.round(pack.bytes_read / 102.4) / 10} KB</dd>
        <dt>Path</dt>
        <dd className="pack-path">{pack.path}</dd>
      </dl>

      {range ? (
        <p className="pack-range-note">
          This build is read-only. Format {formatRangeLabel(range)} is not changed by
          Mine Utils.
        </p>
      ) : null}

      <div className="pack-danger-zone">
        {confirming ? (
          <div className="pack-confirm" role="alert">
            <p>Move this pack to the system trash?</p>
            <div className="pack-confirm-actions">
              <button
                type="button"
                className="button button-danger"
                disabled={busy}
                onClick={() => {
                  void onDelete(pack).finally(() => setConfirming(false));
                }}
              >
                Move to trash
              </button>
              <button
                type="button"
                className="button"
                disabled={busy}
                onClick={() => setConfirming(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            className="button button-danger"
            disabled={busy}
            onClick={() => setConfirming(true)}
          >
            Delete pack
          </button>
        )}
      </div>
    </aside>
  );
}
