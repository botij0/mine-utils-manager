import type { ModrinthKind, ModrinthSort } from '../../lib/types';

interface SearchBarProps {
  query: string;
  onQueryChange: (query: string) => void;
  kind: ModrinthKind;
  onKindChange: (kind: ModrinthKind) => void;
  gameVersion: string;
  onGameVersionChange: (version: string) => void;
  installedVersions: string[];
  sort: ModrinthSort;
  onSortChange: (sort: ModrinthSort) => void;
  searching: boolean;
}

const SORTS: { value: ModrinthSort; label: string }[] = [
  { value: 'relevance', label: 'Relevance' },
  { value: 'downloads', label: 'Downloads' },
  { value: 'follows', label: 'Follows' },
  { value: 'newest', label: 'Newest' },
  { value: 'updated', label: 'Updated' },
];

export default function SearchBar({
  query,
  onQueryChange,
  kind,
  onKindChange,
  gameVersion,
  onGameVersionChange,
  installedVersions,
  sort,
  onSortChange,
  searching,
}: SearchBarProps) {
  return (
    <div className="discover-bar">
      <div className="discover-tabs" role="tablist" aria-label="Project type">
        <button
          type="button"
          role="tab"
          aria-selected={kind === 'resourcepack'}
          className={kind === 'resourcepack' ? 'button button-primary' : 'button'}
          onClick={() => onKindChange('resourcepack')}
        >
          Resource packs
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={kind === 'datapack'}
          className={kind === 'datapack' ? 'button button-primary' : 'button'}
          onClick={() => onKindChange('datapack')}
        >
          Datapacks
        </button>
      </div>
      <div className="field">
        <label htmlFor="discover-query">Search Modrinth</label>
        <input
          id="discover-query"
          type="search"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder="Fresh Animations"
          autoComplete="off"
        />
      </div>
      <div className="field">
        <label htmlFor="discover-version">Game version</label>
        <select
          id="discover-version"
          value={gameVersion}
          onChange={(event) => onGameVersionChange(event.target.value)}
        >
          <option value="">Any version</option>
          {installedVersions.map((version) => (
            <option key={version} value={version}>
              {version}
            </option>
          ))}
        </select>
      </div>
      <div className="field">
        <label htmlFor="discover-sort">Sort</label>
        <select
          id="discover-sort"
          value={sort}
          onChange={(event) => onSortChange(event.target.value as ModrinthSort)}
        >
          {SORTS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </div>
      {searching ? (
        <p className="empty-note" role="status">
          Searching…
        </p>
      ) : null}
    </div>
  );
}
