import { useCallback, useEffect, useMemo, useState } from 'react';
import type { PackClient } from '../../lib/ipc';
import { ipc, isTauriRuntime, pickPackPath, toAppError } from '../../lib/ipc';
import type { GameRunning, ImportChoice, Pack, PackKind } from '../../lib/types';
import {
  formatFilterOptions,
  matchesFormat,
  matchesKind,
  matchesSearch,
} from '../../lib/format';
import ActiveStack from './ActiveStack';
import GameRunningBanner from './GameRunningBanner';
import ImportSheet from './ImportSheet';
import type { ImportPrompt } from './ImportSheet';
import PackDetail from './PackDetail';
import PackGrid from './PackGrid';
import { usePacks } from './usePacks';
import { packFilename } from '../../lib/packPath';

interface PacksPageProps {
  client?: PackClient;
  /** File and folder picker. Injected in tests. */
  pickPath?: (directory: boolean) => Promise<string | null>;
  /** From the app-level game guard poll. Undefined until the first check. */
  gameRunning?: GameRunning | null;
  /** No current guard answer: unknown, so writes stay paused. */
  gameGuardUnknown?: boolean;
  /** The unknown state is a new folder's first check, still pending. */
  gameGuardChecking?: boolean;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
}

interface ToggleToast {
  message: string;
  undo: () => void;
}

/** Tauri adds a real path to dropped files; a browser drop has none. */
function droppedPaths(transfer: DataTransfer | null): string[] {
  if (!transfer) return [];
  return Array.from(transfer.files)
    .map((file) => (file as File & { path?: string }).path)
    .filter((path): path is string => typeof path === 'string' && path !== '');
}

export default function PacksPage({
  client = ipc,
  pickPath = pickPackPath,
  gameRunning,
  gameGuardUnknown = false,
  gameGuardChecking = false,
  refreshGameRunning = () => {},
}: PacksPageProps) {
  const {
    packs,
    stats,
    loading,
    busy,
    error,
    stack,
    stackLoading,
    stackError,
    refreshStack,
    importPack,
    deletePack,
    togglePack,
    reorderStack,
  } = usePacks(client);
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState<PackKind | 'all'>('all');
  const [format, setFormat] = useState('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [prompt, setPrompt] = useState<ImportPrompt | null>(null);
  const [path, setPath] = useState('');
  const [extraPaths, setExtraPaths] = useState<string[]>([]);
  const [lastPath, setLastPath] = useState<string | null>(null);
  const [sheetError, setSheetError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [toast, setToast] = useState<ToggleToast | null>(null);

  // A fresh null from the app poll clears the banner; undefined means the
  // first check has not landed yet, so fall back to the stack response. A
  // failed check keeps the last known running value and blocks writes.
  const running = gameRunning !== undefined ? gameRunning : (stack?.running ?? null);
  const writesBlocked = running !== null || gameGuardUnknown;

  useEffect(() => {
    if (!toast) return undefined;
    const timer = setTimeout(() => setToast(null), 8000);
    return () => clearTimeout(timer);
  }, [toast]);

  const refreshAfterGuard = useCallback(
    (err: unknown) => {
      const appError = toAppError(err);
      setActionError(appError.message);
      if (appError.code !== 'game_running') return;
      // The backend just saw the game; refresh both sources so the banner and
      // every disabled control appear now instead of at the next poll.
      void refreshStack();
      void refreshGameRunning();
    },
    [refreshStack, refreshGameRunning],
  );

  const handleToggle = async (filename: string, enabled: boolean): Promise<void> => {
    setActionError(null);
    try {
      await togglePack(filename, enabled);
      setToast({
        message: `${filename} ${enabled ? 'enabled' : 'disabled'}`,
        undo: () => {
          setToast(null);
          void handleToggle(filename, !enabled);
        },
      });
    } catch (err) {
      refreshAfterGuard(err);
    }
  };

  const handleReorder = useCallback(
    async (order: string[]) => {
      setActionError(null);
      try {
        await reorderStack(order);
      } catch (err) {
        refreshAfterGuard(err);
      }
    },
    [reorderStack, refreshAfterGuard],
  );

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
  const noGameDir =
    error?.code === 'no_active_game_dir' || stackError?.code === 'no_active_game_dir';
  const stackErrorNotice =
    stackError && !noGameDir ? (
      <div className="stack-error" role="alert">
        <p className="error">{stackError.message}</p>
        <button type="button" className="button" onClick={() => void refreshStack()}>
          Retry
        </button>
      </div>
    ) : null;

  const clearFilters = () => {
    setQuery('');
    setKind('all');
    setFormat('all');
  };

  const closeSheet = () => {
    setSheetOpen(false);
    setPrompt(null);
    setPath('');
    setExtraPaths([]);
    setLastPath(null);
    setSheetError(null);
  };

  const handleImportPath = useCallback(
    async (source: string) => {
      if (writesBlocked) return;
      setSheetError(null);
      setLastPath(source);
      try {
        const outcome = await importPack(source);
        if (outcome.status === 'imported') {
          setSelectedId(outcome.pack.id);
          closeSheet();
        } else if (outcome.status === 'duplicate' || outcome.status === 'collision') {
          setPrompt(outcome);
          setSheetOpen(true);
        } else {
          closeSheet();
        }
      } catch (err) {
        // Import failures stay inside the sheet, over its own backdrop.
        const appError = toAppError(err);
        setSheetError(appError.message);
        if (appError.code === 'game_running') void refreshGameRunning();
        setSheetOpen(true);
      }
    },
    [importPack, writesBlocked, refreshGameRunning],
  );

  const handleChoose = async (choice: ImportChoice) => {
    if (!lastPath || writesBlocked) return;
    setSheetError(null);
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
      const appError = toAppError(err);
      setSheetError(appError.message);
      if (appError.code === 'game_running') void refreshGameRunning();
    }
  };

  const handleDelete = async (pack: Pack) => {
    setActionError(null);
    try {
      await deletePack(pack.path);
      if (selectedId === pack.id) setSelectedId(null);
    } catch (err) {
      refreshAfterGuard(err);
    }
  };

  /**
   * A drop with one path imports it. A drop with several opens the sheet on
   * the first path and names the rest, so nothing is imported silently.
   */
  const handleDroppedPaths = useCallback(
    (paths: string[]) => {
      const [first, ...rest] = paths.filter((item) => item.trim() !== '');
      if (!first) return;
      setActionError(null);
      setSheetError(null);
      if (rest.length === 0) {
        void handleImportPath(first);
        return;
      }
      setPrompt(null);
      setPath(first);
      setExtraPaths(rest);
      setSheetOpen(true);
    },
    [handleImportPath],
  );

  useEffect(() => {
    if (!isTauriRuntime()) return undefined;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type !== 'drop') return;
          handleDroppedPaths(event.payload.paths);
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
  }, [handleDroppedPaths]);

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
            disabled={noGameDir || writesBlocked}
            title={noGameDir ? 'Pick a game dir in Settings first' : undefined}
            onClick={() => {
              setPrompt(null);
              setSheetError(null);
              setExtraPaths([]);
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
          const paths = droppedPaths(event.dataTransfer);
          if (paths.length > 0) {
            handleDroppedPaths(paths);
          } else if (!isTauriRuntime()) {
            setActionError('Dropping files needs the desktop app. Use Import pack instead.');
          }
        }}
      >
        {stack ? (
          <>
            {stackErrorNotice}
            <ActiveStack
              stack={stack}
              packs={packs}
              busy={busy}
              running={running}
              guardUnknown={gameGuardUnknown}
              guardChecking={gameGuardChecking}
              onToggle={(filename, enabled) => void handleToggle(filename, enabled)}
              onReorder={(order) => void handleReorder(order)}
              onRecheck={() => {
                void refreshStack();
                void refreshGameRunning();
              }}
            />
          </>
        ) : (
          <section className="active-stack" aria-label="Active stack">
            <div className="active-stack-head">
              <h2>Active stack</h2>
            </div>
            <GameRunningBanner
              running={running}
              guardUnknown={gameGuardUnknown}
              checking={gameGuardChecking}
              staleLocks={[]}
              supported
              onRecheck={() => {
                void refreshStack();
                void refreshGameRunning();
              }}
            />
            {stackErrorNotice ?? (
              <p className="active-stack-empty">
                {noGameDir
                  ? 'Pick a game dir in Settings first.'
                  : stackLoading
                    ? 'Loading the stack…'
                    : 'The stack is unavailable.'}
              </p>
            )}
          </section>
        )}
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
        <PackDetail
          pack={selected}
          busy={busy}
          writesBlocked={writesBlocked}
          enabledInStack={
            selected
              ? stack?.entries.some(
                  (entry) => entry.kind === 'file' && entry.filename === packFilename(selected),
                ) ?? false
              : false
          }
          onSetEnabled={(enabled) =>
            selected && void handleToggle(packFilename(selected), enabled)
          }
          onDelete={handleDelete}
        />
      </div>

      <footer className="packs-stats">
        {packs.length} packs
        {stats
          ? `, ${(stats.bytesRead / 1024).toFixed(1)} KB read, ${stats.elapsedMs.toFixed(0)} ms scan`
          : ''}
      </footer>

      {toast ? (
        <div className="toast" role="status" aria-label="Toggle result">
          <span>{toast.message}</span>
          <button type="button" className="button" onClick={toast.undo}>
            Undo
          </button>
          <button
            type="button"
            className="button button-icon"
            aria-label="Dismiss"
            onClick={() => setToast(null)}
          >
            ×
          </button>
        </div>
      ) : null}

      <ImportSheet
        open={sheetOpen}
        prompt={prompt}
        busy={busy}
        error={sheetError}
        extraPaths={extraPaths}
        path={path}
        onPathChange={setPath}
        canImport={!noGameDir && !writesBlocked}
        onClose={closeSheet}
        onImportPath={handleImportPath}
        onChoose={handleChoose}
        onBrowse={pickPath}
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
