interface CompatibilityChipProps {
  /** Null when the pack declares no range or the world format is unknown. */
  warning: string | null;
}

/** A warning chip for a datapack built for a different game version. The
 *  warning never blocks an install or an enable; it only explains the risk. */
export default function CompatibilityChip({ warning }: CompatibilityChipProps) {
  if (!warning) return null;
  return (
    <span className="compat-chip compat-chip-warn" role="note" title={warning}>
      May not load on this world
    </span>
  );
}
