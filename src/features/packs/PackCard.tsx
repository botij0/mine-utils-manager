import type { ReactNode } from 'react';
import type { Pack } from '../../lib/types';
import { packFormatLabel, packGameLabel, packKindLabel, shortDescription } from '../../lib/format';
import { packInitials } from '../../lib/avatar';

interface CardShellProps {
  icon: ReactNode;
  name: string;
  description: string;
  chips: ReactNode;
  label: string;
  selected: boolean;
  broken?: boolean;
  /** Pack cards expose their selected state; discover cards do not. */
  pressed?: boolean;
  onSelect: () => void;
}

export function CardShell({
  icon,
  name,
  description,
  chips,
  label,
  selected,
  broken = false,
  pressed,
  onSelect,
}: CardShellProps) {
  return (
    <button
      type="button"
      className={`pack-card${selected ? ' is-selected' : ''}${broken ? ' is-broken' : ''}`}
      aria-pressed={pressed}
      aria-label={label}
      onClick={onSelect}
    >
      {icon}
      <span className="pack-card-body">
        <span className="pack-name">{name}</span>
        <span className={broken ? 'pack-broken-text' : 'pack-desc'}>{description}</span>
        <span className="pack-chips">{chips}</span>
      </span>
    </button>
  );
}

interface PackCardProps {
  pack: Pack;
  selected: boolean;
  onSelect: (pack: Pack) => void;
}

export default function PackCard({ pack, selected, onSelect }: PackCardProps) {
  const broken = pack.error !== null;
  const label = broken
    ? `${pack.name}, error card`
    : `${pack.name}, ${packKindLabel(pack.kind)}, ${packGameLabel(pack)}`;
  return (
    <CardShell
      selected={selected}
      broken={broken}
      pressed={selected}
      label={label}
      onSelect={() => onSelect(pack)}
      icon={
        <span className="pack-icon">
          {pack.icon ? (
            <img src={pack.icon.url} alt="" loading="lazy" />
          ) : (
            <span className="pack-icon-fallback" aria-hidden="true">
              {broken ? '!' : packInitials(pack.name)}
            </span>
          )}
        </span>
      }
      name={pack.name}
      description={
        broken
          ? 'Cannot read this archive'
          : shortDescription(pack.description || 'No description')
      }
      chips={
        broken ? (
          <span className="chip chip-danger">Error</span>
        ) : (
          <>
            <span className="chip">{packKindLabel(pack.kind)}</span>
            <span className="chip chip-muted">{packFormatLabel(pack)}</span>
            {pack.warnings.length > 0 ? <span className="chip chip-warn">Warning</span> : null}
          </>
        )
      }
    />
  );
}
