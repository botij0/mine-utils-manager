import { useCallback, useEffect, useMemo, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { ipc, isTauriRuntime, pickPackPath, toAppError } from '../../lib/ipc';
import type { ImportChoice, Pack, PackKind } from '../../lib/types';
import {
  formatFilterOptions,
  matchesFormat,
  matchesKind,
  matchesSearch,
} from '../../lib/format';
import ImportSheet from './ImportSheet';
import type { ImportPrompt } from './ImportSheet';
import PackDetail from './PackDetail';
import PackGrid from './PackGrid';
import { usePacks } from './usePacks';

interface PacksPageProps {
  client?: PackClient;
}

export default function PacksPage({ client = ipc }: PacksPageProps) {
  const { packs, stats, loading, busy, error, importPack, deletePack } = usePacks(client);
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState<PackKind | 'all'>('all');
  const [format, setFormat] = useState('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [prompt, setPrompt] = useState<ImportPrompt | null>(null);
  const [lastPath, setLastPath] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);

  const formats = useMemo(() => formatFilterOptions(packs), [packs]);
  const filtered = useMemo(
    () =>
      packs.filter(
        (pack) =>
          matchesSearch(pack, query) &&
          matchesKind(pack, kind) &&
          matchesFormat(pack, format),
      ),
    [packs, query, kind, format],
  );
  const selected = packs.find((pack) => pack.id === selectedId) ?? null;
  const hasFilters = query.trim() !== '' || kind !== 'all' || format !== 'all';
  const noGameDir = error?.code === 'no_active_game_dir';

  const clearFilters = () => {
    setQuery('');
    setKind('all');
    setFormat('all');
  };

  const closeSheet = () => {
    setSheetOpen(false);
    setPrompt(null);
    setLastPath(null);
  };

  const handleImportPath = useCallback(
    async (path: string) => {
      setActionError(null);
      setLastPath(path);
      try {
        const outcome = await importPack(path);
        if (outcome.status === 'imported') {
          setSelectedId(outcome.pack.id);
          setSheetOpen(false);
          setPrompt(null);
          setLastPath(null);
        } else if (outcome.status === 'duplicate' || outcome.status === 'collision') {
          setPrompt(outcome);
          setSheetOpen(true);
        } else {
          setSheetOpen(false);
          setPrompt(null);
          setLastPath(null);
        }
      } catch (err) {
        setActionError(toAppError(err).message);
        setSheetOpen(true);
      }
    },
    [importPack],
  );

  const handleChoose = async (choice: ImportChoice) => {
    if (!lastPath) return;
    setActionError(null);
    try {
      const outcome = await importPack(lastPath, choice);
      if (outcome.status === 'imported') {
        setSelectedId(outcome.pack.id);
        closeSheet();
      } else if (outcome.status === 'skipped') {
        closeSheet();
      } else {
        setPrompt(outcome);
      }
    } catch (err) {
      setActionError(toAppError(err).message);
    }
  };

  const handleDelete = async (pack: Pack) => {
    setActionError(null);
    try {
      await deletePack(pack.path);
      if (selectedId === pack.id) setSelectedId(null);
    } catch (err) {
      setActionError(toAppError(err).message);
    }
  };

  useEffect(() => {
    if (!isTauriRuntime()) return undefined;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type !== 'drop') return;
          const [path] = event.payload.paths;
          if (path) void handleImportPath(path);
        }),
      )
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [handleImportPath]);

  return (
    <section className="packs-page">
      <div className="packs-toolbar">
        <label className="field">
          <span>Search packs</span>
          <input
            type="search"
            value={query}
            placeholder="Name or description"
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <label className="field">
          <span>Kind</span>
          <select
            value={kind}
            onChange={(event) => setKind(event.target.value as PackKind | 'all')}
          >
            <option value="all">All kinds</option>
            <option value="resource">Resource</option>
            <option value="data">Data</option>
            <option value="both">Resource and data</option>
            <option value="unknown">Unknown</option>
          </select>
        </label>
        <label className="field">
          <span>Format</span>
          <select value={format} onChange={(event) => setFormat(event.target.value)}>
            <option value="all">All formats</option>
            {formats.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <div className="packs-toolbar-actions">
          <button
            type="button"
            className="button button-primary"
            onClick={() => {
              setPrompt(null);
              setSheetOpen(true);
            }}
          >
            Import pack
          </button>
        </div>
      </div>

      {actionError ? (
        <p className="error" role="alert">
          {actionError}
        </p>
      ) : null}
      {error && !noGameDir ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}

      <div
        className={`packs-body${dragging ? ' is-dragging' : ''}`}
        onDragOver={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          if (!isTauriRuntime()) {
            setActionError('Dropping files needs the desktop app. Use Import pack instead.');
          }
        }}
      >
        <div className="packs-list">
          <PackGrid
            packs={filtered}
            selectedId={selectedId}
            loading={loading || busy}
            hasFilters={hasFilters}
            noGameDir={noGameDir}
            onSelect={(pack) => setSelectedId(pack.id)}
            onClearFilters={clearFilters}
          />
          {dragReadyHint(dragging)}
        </div>
        <PackDetail pack={selected} busy={busy} onDelete={handleDelete} />
      </div>

      <footer className="packs-stats">
        {packs.length} packs
        {stats
          ? `, ${(stats.bytesRead / 1024).toFixed(1)} KB read, ${stats.elapsedMs.toFixed(0)} ms scan`
          : ''}
      </footer>

      <ImportSheet
        open={sheetOpen}
        prompt={prompt}
        busy={busy}
        onClose={closeSheet}
        onImportPath={handleImportPath}
        onChoose={handleChoose}
        onBrowse={pickPackPath}
      />
    </section>
  );
}

function dragReadyHint(dragging: boolean) {
  if (!dragging) return null;
  return (
    <p className="packs-drop-hint" role="status">
      Drop a zip or pack folder to import it.
    </p>
  );
}
