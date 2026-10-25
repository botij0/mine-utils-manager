interface PackEmptyStateProps {
  variant: 'loading' | 'no-dir' | 'empty' | 'no-results';
  onClearFilters?: () => void;
}

export default function PackEmptyState({ variant, onClearFilters }: PackEmptyStateProps) {
  if (variant === 'loading') {
    return (
      <p className="packs-empty">
        Reading the resourcepacks folder.
      </p>
    );
  }
  if (variant === 'no-dir') {
    return (
      <div className="packs-empty">
        <h3>No active game dir</h3>
        <p>Pick a game dir in Settings before listing resource packs.</p>
      </div>
    );
  }
  if (variant === 'no-results') {
    return (
      <div className="packs-empty">
        <h3>No packs match those filters</h3>
        <p>Search by name or description, or reset the kind and format filters.</p>
        {onClearFilters ? (
          <button type="button" className="button" onClick={onClearFilters}>
            Clear filters
          </button>
        ) : null}
      </div>
    );
  }
  return (
    <div className="packs-empty">
      <h3>No resource packs yet</h3>
      <p>Drop a zip or a pack folder here, or use Import pack.</p>
    </div>
  );
}
