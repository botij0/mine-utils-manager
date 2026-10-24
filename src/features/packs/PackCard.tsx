import type { Pack } from '../../lib/types';
import { packFormatLabel, packGameLabel, packKindLabel, shortDescription } from '../../lib/format';

interface PackCardProps {
  pack: Pack;
  selected: boolean;
  onSelect: (pack: Pack) => void;
}

function initials(name: string): string {
  return name.replace(/[^A-Za-z0-9]/g, '').slice(0, 2).toUpperCase() || 'PK';
}

export default function PackCard({ pack, selected, onSelect }: PackCardProps) {
  const broken = pack.error !== null;
  const label = broken
    ? `${pack.name}, error card`
    : `${pack.name}, ${packKindLabel(pack.kind)}, ${packGameLabel(pack)}`;
  return (
    <button
      type="button"
      className={`pack-card${selected ? ' is-selected' : ''}${broken ? ' is-broken' : ''}`}
      aria-pressed={selected}
      aria-label={label}
      onClick={() => onSelect(pack)}
    >
      <span className="pack-icon">
        {pack.icon ? (
          <img src={pack.icon.url} alt="" loading="lazy" />
        ) : (
          <span className="pack-icon-fallback" aria-hidden="true">
            {broken ? '!' : initials(pack.name)}
          </span>
        )}
      </span>
      <span className="pack-card-body">
        <span className="pack-name">{pack.name}</span>
        {broken ? (
          <span className="pack-broken-text">Cannot read this archive</span>
        ) : (
          <span className="pack-desc">
            {shortDescription(pack.description || 'No description')}
          </span>
        )}
        <span className="pack-chips">
          {broken ? (
            <span className="chip chip-danger">Error</span>
          ) : (
            <>
              <span className="chip">{packKindLabel(pack.kind)}</span>
              <span className="chip chip-muted">{packFormatLabel(pack)}</span>
            </>
          )}
        </span>
      </span>
    </button>
  );
}
