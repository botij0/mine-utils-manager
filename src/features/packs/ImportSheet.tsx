import type { ImportChoice } from '../../lib/types';
import DialogShell from '../discover/DialogShell';

export type ImportPrompt =
  | { status: 'duplicate'; name: string; existing: string }
  | { status: 'collision'; name: string };

interface ImportSheetProps {
  open: boolean;
  prompt: ImportPrompt | null;
  busy: boolean;
  /** Import and Replace errors render here, inside the sheet. */
  error: string | null;
  /** Extra dropped paths, reported when a drop carried more than one. */
  extraPaths: string[];
  path: string;
  onPathChange: (path: string) => void;
  /** False when no game dir is active, so nothing can be imported. */
  canImport: boolean;
  onClose: () => void;
  onImportPath: (path: string) => Promise<void>;
  onChoose: (choice: ImportChoice) => Promise<void>;
  onBrowse: (directory: boolean) => Promise<string | null>;
}

export default function ImportSheet({
  open,
  prompt,
  busy,
  error,
  extraPaths,
  path,
  onPathChange,
  canImport,
  onClose,
  onImportPath,
  onChoose,
  onBrowse,
}: ImportSheetProps) {
  if (!open) return null;

  const browse = async (directory: boolean) => {
    const picked = await onBrowse(directory);
    if (picked) {
      onPathChange(picked);
    }
  };

  const errorBlock = error ? (
    <p className="error" role="alert">
      {error}
    </p>
  ) : null;

  if (prompt) {
    const duplicate = prompt.status === 'duplicate';
    return (
      <DialogShell
        label={duplicate ? 'Duplicate pack' : 'Pack name in use'}
        onClose={onClose}
        dismissOnEscape={!busy}
      >
        <h2>{duplicate ? 'That pack is already installed' : 'A pack with that name exists'}</h2>
        <p>
          <strong>{prompt.name}</strong>{' '}
          {duplicate
            ? `has the same SHA-256 as ${prompt.existing}.`
            : 'is already in the resourcepacks folder with different content.'}
        </p>
        <p className="sheet-note">
          Skip leaves the installed pack alone. Replace moves one old pack to the trash and
          copies this one in. When a duplicate and a name clash both exist, each Replace
          removes one pack.
        </p>
        {errorBlock}
        <div className="sheet-actions">
          <button
            type="button"
            className="button button-primary"
            disabled={busy || !canImport}
            onClick={() => {
              void onChoose('replace');
            }}
          >
            Replace
          </button>
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={() => {
              if (duplicate) {
                void onChoose('skip');
              } else {
                onClose();
              }
            }}
          >
            {duplicate ? 'Skip' : 'Cancel'}
          </button>
        </div>
      </DialogShell>
    );
  }

  return (
    <DialogShell label="Import pack" onClose={onClose} dismissOnEscape={!busy}>
      <h2>Import a resource pack</h2>
      <p className="sheet-note">
        Choose a zip or a folder that contains pack.mcmeta. A folder pack is copied into the
        active game dir, never linked.
      </p>
      <form
        className="sheet-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (!path.trim() || !canImport) return;
          void onImportPath(path.trim());
        }}
      >
        <label htmlFor="pack-path">Pack path</label>
        <input
          id="pack-path"
          value={path}
          placeholder="/home/you/Downloads/pack.zip"
          onChange={(event) => onPathChange(event.target.value)}
          autoFocus
        />
        <div className="sheet-actions">
          <button
            type="submit"
            className="button button-primary"
            disabled={busy || !canImport || path.trim() === ''}
          >
            Import
          </button>
          <button type="button" className="button" disabled={busy} onClick={() => void browse(false)}>
            Browse file
          </button>
          <button type="button" className="button" disabled={busy} onClick={() => void browse(true)}>
            Browse folder
          </button>
          <button type="button" className="button" onClick={onClose}>
            Close
          </button>
        </div>
      </form>
      {extraPaths.length > 0 ? (
        <p className="sheet-note" role="status">
          Also dropped: {extraPaths.join(', ')}. Import them one at a time.
        </p>
      ) : null}
      {errorBlock}
    </DialogShell>
  );
}
