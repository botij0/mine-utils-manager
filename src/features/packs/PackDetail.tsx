import { useEffect, useState } from 'react';
import type { Pack } from '../../lib/types';
import { hasNestedSymlink } from '../../lib/packPath';
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
  writesBlocked: boolean;
  enabledInStack: boolean;
  onSetEnabled: (enabled: boolean) => void;
  onDelete: (pack: Pack) => Promise<void>;
}

function versionOrDash(value: { major: number; minor: number } | null): string {
  return value ? formatVersionLabel(value) : 'Not declared';
}

export default function PackDetail({
  pack,
  busy,
  writesBlocked,
  enabledInStack,
  onSetEnabled,
  onDelete,
}: PackDetailProps) {
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
  const stackEligible = pack.kind === 'resource' || pack.kind === 'both';
  const nestedSymlink = hasNestedSymlink(pack);
  const symlinkPack = pack.error?.code === 'pack_symlink' && !nestedSymlink;
  const nonUtf8Pack = pack.error?.code === 'pack_non_utf8_name';
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
        <dt>Read for this pack</dt>
        <dd>{Math.round(pack.bytes_read / 102.4) / 10} KB</dd>
        <dt>Path</dt>
        <dd className="pack-path">{pack.path}</dd>
      </dl>

      {range ? (
        <p className="pack-range-note">
          Mine Utils does not change this pack's declared format range
          ({formatRangeLabel(range)}).
        </p>
      ) : null}

      {pack.error && !nestedSymlink ? null : stackEligible || nestedSymlink ? (
        <label className="stack-toggle pack-stack-toggle">
          <input
            type="checkbox"
            checked={enabledInStack}
            disabled={busy || writesBlocked || (nestedSymlink && !enabledInStack)}
            onChange={(event) => onSetEnabled(event.target.checked)}
          />
          Enabled in the active stack
        </label>
      ) : (
        <p className="pack-range-note">
          {pack.kind === 'data'
            ? 'Data packs are not part of the resource pack stack.'
            : 'The pack kind is unknown, so the stack toggle is unavailable.'}
        </p>
      )}

      {nestedSymlink ? (
        <p className="pack-range-note">
          Remove the nested link <code>{pack.error?.symlink_path}</code> in a file manager
          before enabling this pack. You can disable it now. Deleting this pack moves the
          whole pack folder to the trash, including the link. The link target is never touched.
        </p>
      ) : null}

      {nonUtf8Pack ? (
        <p className="pack-range-note">Rename this file in a file manager to manage it.</p>
      ) : symlinkPack ? (
        <p className="pack-range-note">
          This entry is a symlink. Remove the link from resourcepacks in a file
          manager; Mine Utils does not delete links.
        </p>
      ) : (
        <div className="pack-danger-zone">
          {confirming ? (
            <div className="pack-confirm" role="alert">
              <p>Move this pack to the system trash?</p>
              <div className="pack-confirm-actions">
                <button
                  type="button"
                  className="button button-danger"
                  disabled={busy || writesBlocked}
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
              disabled={busy || writesBlocked}
              title={
                writesBlocked
                  ? 'Writes are paused until the game is confirmed closed'
                  : undefined
              }
              onClick={() => setConfirming(true)}
            >
              Delete pack
            </button>
          )}
        </div>
      )}
    </aside>
  );
}
