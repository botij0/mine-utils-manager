import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { Pack, StackEntry as StackEntryData } from '../../lib/types';
import { packInitials } from '../../lib/avatar';

interface StackEntryProps {
  id: string;
  entry: StackEntryData;
  pack: Pack | null;
  disabled: boolean;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onToggle: (enabled: boolean) => void;
  onMove: (direction: -1 | 1) => void;
}

function LockIcon() {
  return (
    <svg className="stack-lock-icon" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <rect x="3" y="7" width="10" height="7" rx="1.5" fill="currentColor" />
      <path
        d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
      />
    </svg>
  );
}

/** One card in the active stack. File entries drag, toggle, and move; foreign
 *  entries such as `vanilla` are locked and carry no controls. */
export default function StackEntry({
  id,
  entry,
  pack,
  disabled,
  canMoveUp,
  canMoveDown,
  onToggle,
  onMove,
}: StackEntryProps) {
  const sortable = useSortable({
    id,
    disabled: disabled || entry.kind !== 'file',
  });
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = sortable;

  if (entry.kind === 'foreign') {
    return (
      <li
        className="stack-entry stack-entry-foreign"
        aria-label={entry.value}
        ref={setNodeRef}
      >
        <span className="stack-grip stack-grip-locked" aria-hidden="true">
          <LockIcon />
        </span>
        <span className="stack-name">{entry.value}</span>
        <span className="chip chip-muted">
          <LockIcon /> Locked
        </span>
      </li>
    );
  }

  const label = pack ? pack.name : entry.filename;
  const missing = pack === null;
  return (
    <li
      className={`stack-entry${isDragging ? ' is-dragging' : ''}${missing ? ' is-missing' : ''}`}
      aria-label={label}
      title={entry.value}
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
    >
      <button
        type="button"
        className="stack-grip"
        aria-label={`Drag ${label}`}
        disabled={disabled}
        {...attributes}
        {...listeners}
      >
        <span aria-hidden="true">⋮⋮</span>
      </button>
      <span className="stack-icon" aria-hidden="true">
        {pack?.icon ? (
          <img src={pack.icon.url} alt="" />
        ) : (
          <span className="stack-icon-fallback">{missing ? '!' : packInitials(label)}</span>
        )}
      </span>
      <span className="stack-body">
        <span className="stack-name">{label}</span>
        <span className="stack-sub">
          {missing ? `${entry.filename} is not in resourcepacks` : entry.value}
        </span>
      </span>
      {missing ? (
        <button
          type="button"
          className="button button-danger stack-remove"
          disabled={disabled}
          onClick={() => onToggle(false)}
        >
          Remove
        </button>
      ) : (
        <label className="stack-toggle">
          <input
            type="checkbox"
            aria-label={`Enabled: ${label}`}
            title={`Enabled: ${label}`}
            checked
            disabled={disabled}
            onChange={(event) => onToggle(event.target.checked)}
          />
        </label>
      )}
      <span className="stack-moves">
        <button
          type="button"
          className="button button-icon"
          aria-label={`Move ${label} up`}
          disabled={disabled || !canMoveUp}
          onClick={() => onMove(-1)}
        >
          ↑
        </button>
        <button
          type="button"
          className="button button-icon"
          aria-label={`Move ${label} down`}
          disabled={disabled || !canMoveDown}
          onClick={() => onMove(1)}
        >
          ↓
        </button>
      </span>
    </li>
  );
}
