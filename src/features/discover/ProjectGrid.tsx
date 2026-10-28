import type { ModrinthSearchHit } from '../../lib/types';
import { formatCount } from '../../lib/avatar';
import { CardShell } from '../packs/PackCard';
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
          <CardShell
            selected={hit.project_id === selectedId}
            label={`${hit.title}, ${hit.downloads} downloads`}
            onSelect={() => onSelect(hit)}
            icon={
              isTrustedIconUrl(hit.icon_url) ? (
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
              )
            }
            name={hit.title}
            description={hit.description}
            chips={
              <>
                <span className="chip">{formatCount(hit.downloads)} downloads</span>
                {hit.latest_version ? (
                  <span className="chip chip-muted">{hit.latest_version}</span>
                ) : null}
              </>
            }
          />
        </li>
      ))}
    </ul>
  );
}
