import type { Pack } from '../../lib/types';
import PackCard from './PackCard';
import PackEmptyState from './PackEmptyState';

interface PackGridProps {
  packs: Pack[];
  selectedId: string | null;
  loading: boolean;
  hasFilters: boolean;
  noGameDir: boolean;
  onSelect: (pack: Pack) => void;
  onClearFilters: () => void;
}

export default function PackGrid({
  packs,
  selectedId,
  loading,
  hasFilters,
  noGameDir,
  onSelect,
  onClearFilters,
}: PackGridProps) {
  if (loading && packs.length === 0) {
    return <PackEmptyState variant="loading" />;
  }
  if (noGameDir) {
    return <PackEmptyState variant="no-dir" />;
  }
  if (packs.length === 0) {
    return (
      <PackEmptyState
        variant={hasFilters ? 'no-results' : 'empty'}
        onClearFilters={onClearFilters}
      />
    );
  }
  return (
    <div className="pack-grid" role="list" aria-label="Installed packs">
      {packs.map((pack) => (
        <div role="listitem" key={pack.id}>
          <PackCard
            pack={pack}
            selected={selectedId === pack.id}
            onSelect={onSelect}
          />
        </div>
      ))}
    </div>
  );
}
