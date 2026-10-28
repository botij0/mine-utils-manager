import type { Pack } from '../../lib/types';
import PackCard from './PackCard';
import { EmptyState } from './PackEmptyState';

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
    return <EmptyState>Reading the resourcepacks folder.</EmptyState>;
  }
  if (noGameDir) {
    return (
      <EmptyState title="No active game dir">
        Pick a game dir in Settings before listing resource packs.
      </EmptyState>
    );
  }
  if (packs.length === 0) {
    return hasFilters ? (
      <EmptyState
        title="No packs match those filters"
        action={
          <button type="button" className="button" onClick={onClearFilters}>
            Clear filters
          </button>
        }
      >
        Search by name or description, or reset the kind and format filters.
      </EmptyState>
    ) : (
      <EmptyState title="No resource packs yet">
        Drop a zip or a pack folder here, or use Import pack.
      </EmptyState>
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
