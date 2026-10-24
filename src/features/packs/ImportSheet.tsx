import { useState } from 'react';
import type { ImportChoice } from '../../lib/types';

export type ImportPrompt =
  | { status: 'duplicate'; name: string; existing: string }
  | { status: 'collision'; name: string };

interface ImportSheetProps {
  open: boolean;
  prompt: ImportPrompt | null;
  busy: boolean;
  onClose: () => void;
  onImportPath: (path: string) => Promise<void>;
  onChoose: (choice: ImportChoice) => Promise<void>;
  onBrowse: () => Promise<string | null>;
}

export default function ImportSheet({
  open,
  prompt,
  busy,
  onClose,
  onImportPath,
  onChoose,
  onBrowse,
}: ImportSheetProps) {
  const [path, setPath] = useState('');
  const [browseError, setBrowseError] = useState<string | null>(null);

  if (!open) return null;

  const browse = async () => {
    setBrowseError(null);
    const picked = await onBrowse();
    if (picked) {
      setPath(picked);
    } else {
      setBrowseError('The file picker is only available in the desktop app.');
    }
  };

  if (prompt) {
    const duplicate = prompt.status === 'duplicate';
    return (
      <div className="sheet-backdrop">
        <section
          className="sheet"
          role="dialog"
          aria-modal="true"
          aria-label={duplicate ? 'Duplicate pack' : 'Pack name in use'}
        >
          <h2>{duplicate ? 'That pack is already installed' : 'A pack with that name exists'}</h2>
          <p>
            <strong>{prompt.name}</strong>{' '}
            {duplicate
              ? `has the same SHA-256 as ${prompt.existing}.`
              : 'is already in the resourcepacks folder with different content.'}
          </p>
          <p className="sheet-note">
            Skip leaves the installed pack alone. Replace moves the old pack to the trash and
            copies this one in.
          </p>
          <div className="sheet-actions">
            <button
              type="button"
              className="button button-primary"
              disabled={busy}
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
        </section>
      </div>
    );
  }

  return (
    <div className="sheet-backdrop">
      <section className="sheet" role="dialog" aria-modal="true" aria-label="Import pack">
        <h2>Import a resource pack</h2>
        <p className="sheet-note">
          Choose a zip or a folder that contains pack.mcmeta. A folder pack is copied into the
          active game dir, never linked.
        </p>
        <form
          className="sheet-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (!path.trim()) return;
            void onImportPath(path.trim());
          }}
        >
          <label htmlFor="pack-path">Pack path</label>
          <input
            id="pack-path"
            value={path}
            placeholder="/home/you/Downloads/pack.zip"
            onChange={(event) => setPath(event.target.value)}
            autoFocus
          />
          <div className="sheet-actions">
            <button
              type="submit"
              className="button button-primary"
              disabled={busy || path.trim() === ''}
            >
              Import
            </button>
            <button type="button" className="button" onClick={() => void browse()}>
              Browse
            </button>
            <button type="button" className="button" onClick={onClose}>
              Close
            </button>
          </div>
        </form>
        {browseError ? (
          <p className="sheet-note" role="status">
            {browseError}
          </p>
        ) : null}
      </section>
    </div>
  );
}
