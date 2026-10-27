import type {
  IpcError,
  ModrinthInstallOutcome,
  ModrinthKind,
  World,
} from '../../lib/types';
import DialogShell from './DialogShell';

interface InstallSheetProps {
  open: boolean;
  kind: ModrinthKind;
  projectTitle: string;
  versionLabel: string;
  filename: string;
  worlds: World[];
  world: string | null;
  onWorldChange: (world: string | null) => void;
  busy: boolean;
  writesBlocked: boolean;
  blockedReason: string | null;
  /** False when no game dir is active: installs have nowhere to go. */
  hasGameDir: boolean;
  result: ModrinthInstallOutcome | null;
  error: IpcError | null;
  onClose: () => void;
  onInstall: () => void;
}

function resultText(result: ModrinthInstallOutcome): string {
  if (result.kind === 'resourcepack') {
    const outcome = result.outcome;
    if (outcome.status === 'imported') return `Installed ${outcome.pack.name} and enabled it.`;
    if (outcome.status === 'duplicate') {
      return `${outcome.name} is already installed (same content as ${outcome.existing}).`;
    }
    if (outcome.status === 'collision') {
      return `${outcome.name} already exists with different content; import it from the Packs page to choose Replace.`;
    }
    return 'Import skipped.';
  }
  const outcome = result.outcome;
  if (outcome.status === 'imported') return `Installed ${outcome.filename} and enabled it.`;
  if (outcome.status === 'locked') {
    return `Installed ${outcome.filename}, but the world is locked so it stays disabled: ${outcome.detail}`;
  }
  if (outcome.status === 'duplicate') {
    return `${outcome.name} is already installed (same content as ${outcome.existing}).`;
  }
  if (outcome.status === 'collision') {
    return `${outcome.name} already exists with different content; import it from the Worlds page to choose Replace.`;
  }
  return 'Import skipped.';
}

export default function InstallSheet({
  open,
  kind,
  projectTitle,
  versionLabel,
  filename,
  worlds,
  world,
  onWorldChange,
  busy,
  writesBlocked,
  blockedReason,
  hasGameDir,
  result,
  error,
  onClose,
  onInstall,
}: InstallSheetProps) {
  if (!open) return null;
  const installDisabled =
    busy ||
    writesBlocked ||
    !hasGameDir ||
    (kind === 'datapack' && (worlds.length === 0 || !world));
  return (
    <DialogShell label={`Install ${projectTitle}`} onClose={onClose}>
      <h3>Install {projectTitle}</h3>
      <dl className="pack-facts">
        <div>
          <dt>Version</dt>
          <dd>{versionLabel}</dd>
        </div>
        <div>
          <dt>File</dt>
          <dd>{filename}</dd>
        </div>
      </dl>
      {kind === 'datapack' ? (
        <div className="field">
          <label htmlFor="discover-world">Target world</label>
          {worlds.length === 0 ? (
            <p className="empty-note" role="status">
              No worlds yet. Create or open a world in Minecraft first, then
              install a datapack into it.
            </p>
          ) : (
            <select
              id="discover-world"
              value={world ?? ''}
              onChange={(event) => onWorldChange(event.target.value || null)}
            >
              <option value="">Pick a world</option>
              {worlds.map((item) => (
                <option key={item.dir_name} value={item.dir_name}>
                  {item.level_name} ({item.dir_name})
                </option>
              ))}
            </select>
          )}
        </div>
      ) : null}
      {!hasGameDir ? (
        <p className="empty-note" role="status">
          No game dir is active. Pick a game dir in Settings first, then install.
        </p>
      ) : null}
      {blockedReason ? (
        <p className="empty-note" role="status">
          {blockedReason}
        </p>
      ) : null}
      {error ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}
      {result ? (
        <p className="empty-note" role="status">
          {resultText(result)}
        </p>
      ) : null}
      {result?.provenance_warning ? (
        <p className="empty-note" role="status">
          Warning: {result.provenance_warning}
        </p>
      ) : null}
      <div className="pack-confirm-actions">
        <button type="button" className="button" onClick={onClose}>
          {result ? 'Close' : 'Cancel'}
        </button>
        {result ? null : (
          <button
            type="button"
            className="button button-primary"
            disabled={installDisabled}
            onClick={onInstall}
          >
            {busy ? 'Installing…' : 'Install'}
          </button>
        )}
      </div>
    </DialogShell>
  );
}
