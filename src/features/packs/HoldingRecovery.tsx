import type { HoldingEntry } from '../../lib/types';

interface HoldingRecoveryProps {
  entries: HoldingEntry[];
  disabled: boolean;
  onRestore: (holdingName: string) => void;
}

export default function HoldingRecovery({ entries, disabled, onRestore }: HoldingRecoveryProps) {
  return entries.map((entry) => (
    <div className="banner" role="status" key={entry.holdingName}>
      <p>An interrupted Replace left a copy of <strong>{entry.originalName}</strong>. Restore it when the original name is free.</p>
      <button type="button" className="button" disabled={disabled} onClick={() => onRestore(entry.holdingName)}>
        Restore {entry.originalName}
      </button>
    </div>
  ));
}
