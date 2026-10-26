import type { ModrinthVersion } from '../../lib/types';

interface VersionPickerProps {
  versions: ModrinthVersion[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** The version the list prefers for the active game/world, if any. */
  preferredId: string | null;
  targetLabel: string | null;
}

export default function VersionPicker({
  versions,
  selectedId,
  onSelect,
  preferredId,
  targetLabel,
}: VersionPickerProps) {
  if (versions.length === 0) {
    return <p className="empty-note">No versions published yet.</p>;
  }
  return (
    <div className="field">
      <label htmlFor="discover-version-picker">
        Version
        {targetLabel ? ` (compatible with ${targetLabel} preselected)` : ''}
      </label>
      <select
        id="discover-version-picker"
        value={selectedId ?? ''}
        onChange={(event) => onSelect(event.target.value)}
      >
        {versions.map((version) => (
          <option key={version.id} value={version.id}>
            {version.version_number || version.name || version.id}
            {' — '}
            {version.game_versions.length > 0
              ? version.game_versions.join(', ')
              : 'unknown game versions'}
            {version.id === preferredId ? ' (compatible)' : ''}
          </option>
        ))}
      </select>
    </div>
  );
}
