import type { ModrinthSearchHit } from '../../lib/types';
import { formatCount } from '../../lib/avatar';
import { isTrustedIconUrl } from './icon';

interface ProjectGridProps {
  hits: ModrinthSearchHit[];
  selectedId: string | null;
  onSelect: (hit: ModrinthSearchHit) => void;
}

export default function ProjectGrid({ hits, selectedId, onSelect }: ProjectGridProps) {
  return (
    <ul className="pack-grid discover-grid">
      {hits.map((hit) => (
        <li key={hit.project_id}>
          <button
            type="button"
            className={hit.project_id === selectedId ? 'pack-card is-selected' : 'pack-card'}
            onClick={() => onSelect(hit)}
            aria-label={`${hit.title}, ${hit.downloads} downloads`}
          >
            {isTrustedIconUrl(hit.icon_url) ? (
              // Remote Modrinth CDN icons: plain HTTPS, never the local
              // asset:// icon path (see issue #8 for the installed icons).
              // Untrusted hosts render the placeholder instead of being
              // fetched; no referrer is sent either way.
              <img
                className="pack-icon"
                src={hit.icon_url}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
              />
            ) : (
              <span className="pack-icon-fallback" aria-hidden="true">
                {hit.title.slice(0, 2).toUpperCase()}
              </span>
            )}
            <span className="pack-card-body">
              <span className="pack-name">{hit.title}</span>
              <span className="pack-desc">{hit.description}</span>
              <span className="pack-chips">
                <span className="chip">{formatCount(hit.downloads)} downloads</span>
                {hit.latest_version ? (
                  <span className="chip chip-muted">{hit.latest_version}</span>
                ) : null}
              </span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}
