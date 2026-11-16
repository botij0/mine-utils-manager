import type {
  IpcError,
  ModrinthInstallOutcome,
  ModrinthKind,
  World,
} from '../../lib/types';
import { StatusNote } from '../packs/PackEmptyState';
import DialogShell from './DialogShell';

interface InstallSheetProps {
  open: boolean;
  kind: ModrinthKind;
  projectTitle: string;
  versionLabel: string;
  filename: string;
  worlds: World[];
  worldsLoading?: boolean;
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
  onCancel: () => void;
}

function resultText(result: ModrinthInstallOutcome): string {
  switch (result.outcome.status) {
    case 'imported':
      return result.kind === 'resourcepack'
        ? `Installed ${result.outcome.pack.name} and enabled it.`
        : `Installed ${result.outcome.filename} and enabled it.`;
    case 'locked':
      return `Installed ${result.outcome.filename}, but the world is locked so it stays disabled: ${result.outcome.detail}`;
    case 'duplicate':
      return `${result.outcome.name} is already installed (same content as ${result.outcome.existing}).`;
    case 'collision':
      return `${result.outcome.name} already exists with different content. Remove or rename the existing file before installing.`;
    default:
      return 'Import skipped.';
  }
}

export default function InstallSheet({
  open,
  kind,
  projectTitle,
  versionLabel,
  filename,
  worlds,
  worldsLoading = false,
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
  onCancel,
}: InstallSheetProps) {
  if (!open) return null;
  const installDisabled =
    busy ||
    writesBlocked ||
    !hasGameDir ||
    (kind === 'datapack' && (worlds.length === 0 || !world));
  return (
    <DialogShell label={`Install ${projectTitle}`} onClose={onClose} dismissOnEscape={!busy}>
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
          {worldsLoading ? (
            <StatusNote role="status">Loading worlds…</StatusNote>
          ) : worlds.length === 0 ? (
            <StatusNote role="status">
              No worlds yet. Create or open a world in Minecraft first, then
              install a datapack into it.
            </StatusNote>
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
        <StatusNote role="status">
          No game dir is active. Pick a game dir in Settings first, then install.
        </StatusNote>
      ) : null}
      {blockedReason ? <StatusNote role="status">{blockedReason}</StatusNote> : null}
      {busy ? <StatusNote role="status">Downloading. Cancel is available until installation starts. Leaving this tab keeps the install running; return here for its result.</StatusNote> : null}
      {error ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}
      {result ? <StatusNote role="status">{resultText(result)}</StatusNote> : null}
      {result?.provenance_warning ? (
        <StatusNote role="status">Warning: {result.provenance_warning}</StatusNote>
      ) : null}
      <div className="pack-confirm-actions">
        <button type="button" className="button" onClick={busy ? onCancel : onClose}>
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
