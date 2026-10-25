import { useMemo, useState } from 'react';
import type { WorldsClient } from '../../lib/ipc';
import { ipc, pickPackPath, toAppError } from '../../lib/ipc';
import type { DatapackImportOutcome, GameRunning } from '../../lib/types';
import DatapackDetail from './DatapackDetail';
import DatapackGrid, { type DatapackRow } from './DatapackGrid';
import WorldHeader from './WorldHeader';
import WorldRail from './WorldRail';
import { compatibilityWarning, useWorlds } from './useWorlds';

interface WorldsPageProps {
  client?: WorldsClient;
  /** From the app-level game guard poll. Undefined until the first check. */
  gameRunning?: GameRunning | null;
  /** A failed guard check is unknown, so writes stay paused. */
  gameGuardUnknown?: boolean;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
}

export default function WorldsPage({
  client = ipc,
  gameRunning,
  gameGuardUnknown = false,
  refreshGameRunning = () => {},
}: WorldsPageProps) {
  const {
    worlds,
    loading,
    error,
    selected,
    select,
    packs,
    packsLoading,
    packsError,
    stack,
    stackLoading,
    busy,
    actionError,
    refreshListing,
    toggleDatapack,
    reorderDatapacks,
    importDatapack,
    deleteDatapack,
    clearActionError,
  } = useWorlds(client);
  const [selectedFilename, setSelectedFilename] = useState<string | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [importPrompt, setImportPrompt] = useState<
    Extract<DatapackImportOutcome, { status: 'duplicate' } | { status: 'collision' }> | null
  >(null);
  const [importPath, setImportPath] = useState<string | null>(null);
  const [lockedNotice, setLockedNotice] = useState<string | null>(null);

  const worldFormat = selected?.version?.data_format ?? null;
  const worldLabel = selected?.version?.name ?? selected?.level_name ?? null;

  const rows = useMemo<DatapackRow[]>(() => {
    if (!stack) return [];
    const byFilename = new Map(packs.map((pack) => [filenameOf(pack.path), pack]));
    const loadOrder = stack.enabled.map((entry) => entry.value);
    // Moving up raises load priority (later in Enabled); moving down lowers
    // it. A swap that touches a locked entry or the array edge is refused, so
    // the button is disabled instead of failing on click.
    const moveable = (value: string, delta: 1 | -1): boolean => {
      const index = loadOrder.indexOf(value);
      const target = index + delta;
      return (
        index >= 0 && target >= 0 && target < loadOrder.length && isFileValue(loadOrder[target])
      );
    };
    const topFirst = [...stack.enabled].reverse().map((entry) => {
      const filename = entry.kind === 'file' ? entry.filename : entry.value;
      const pack = entry.kind === 'file' ? (byFilename.get(entry.filename) ?? null) : null;
      return {
        pack,
        entry,
        enabled: true,
        locked: entry.kind === 'foreign',
        warning:
          entry.kind === 'file' && pack
            ? compatibilityWarning(pack, worldFormat, worldLabel)
            : null,
        filename,
        canMoveUp: entry.kind === 'file' && moveable(entry.value, 1),
        canMoveDown: entry.kind === 'file' && moveable(entry.value, -1),
      };
    });
    const disabled = stack.disabled.flatMap((entry) => {
      if (entry.kind !== 'file') return [];
      const pack = byFilename.get(entry.filename) ?? null;
      const row: DatapackRow = {
        pack,
        entry,
        enabled: false,
        locked: false,
        warning: pack ? compatibilityWarning(pack, worldFormat, worldLabel) : null,
        filename: entry.filename,
        canMoveUp: false,
        canMoveDown: false,
      };
      return [row];
    });
    return [...topFirst, ...disabled];
  }, [stack, packs, worldFormat, worldLabel]);

  const selectedRow = rows.find((row) => row.filename === selectedFilename) ?? null;

  // Writes stop while the game runs, while this world is locked, while the
  // file is unreadable, or while the app-level guard check failed.
  const running = gameRunning !== undefined ? gameRunning : (stack?.running ?? null);
  const locked = stack?.world_locked ?? selected?.world_locked ?? null;
  const readOnly = selected?.issue !== null && selected?.issue !== undefined;
  const writesBlocked = running !== null || locked !== null || readOnly || gameGuardUnknown;
  const noGameDir = error?.code === 'no_active_game_dir' || packsError?.code === 'no_active_game_dir';

  const handleToggle = async (filename: string, enabled: boolean) => {
    clearActionError();
    try {
      await toggleDatapack(filename, enabled);
      void refreshGameRunning();
    } catch (err) {
      if (toAppError(err).code === 'game_running') void refreshGameRunning();
    }
  };

  const handleMove = async (filename: string, direction: -1 | 1) => {
    if (!stack) return;
    const order = stack.enabled.map((entry) => entry.value);
    const value = `file/${filename}`;
    const index = order.indexOf(value);
    // The grid is top-first (reverse load order), so moving a card up raises
    // its load priority: one step later in the Enabled array.
    const target = index - direction;
    if (index < 0 || target < 0 || target >= order.length) return;
    // Only file entries move; a swap that touches a foreign entry is refused.
    if (!isFileValue(order[index]) || !isFileValue(order[target])) return;
    const next = [...order];
    [next[index], next[target]] = [next[target], next[index]];
    clearActionError();
    try {
      await reorderDatapacks(next);
    } catch (err) {
      if (toAppError(err).code === 'game_running') void refreshGameRunning();
    }
  };

  const handleDelete = async (path: string) => {
    clearActionError();
    try {
      await deleteDatapack(path);
      setSelectedFilename(null);
    } catch (err) {
      if (toAppError(err).code === 'game_running') void refreshGameRunning();
    }
  };

  const runImport = async (world: string, path: string, choice: 'default' | 'skip' | 'replace' = 'default') => {
    setImportError(null);
    try {
      const outcome = await importDatapack(world, path, choice);
      if (outcome.status === 'imported') {
        setSelectedFilename(outcome.filename);
        setImportPrompt(null);
        setImportPath(null);
      } else if (outcome.status === 'locked') {
        setLockedNotice(
          `Copied ${outcome.filename} but left it disabled: ${outcome.detail}. Enable it after the lock releases.`,
        );
        setSelectedFilename(outcome.filename);
        setImportPrompt(null);
        setImportPath(null);
      } else if (outcome.status === 'skipped') {
        setImportPrompt(null);
        setImportPath(null);
      } else {
        setImportPrompt(outcome);
        setImportPath(path);
      }
    } catch (err) {
      setImportError(toAppError(err).message);
    }
  };

  const handleImportPick = async (directory: boolean) => {
    if (!selected) return;
    setImportError(null);
    const path = await pickPackPath(directory);
    if (!path) return;
    await runImport(selected.dir_name, path);
  };


  return (
    <section className="worlds-page">
      {error && !noGameDir ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : null}
      {noGameDir ? (
        <p className="empty-note">Pick a game dir in Settings first.</p>
      ) : (
        <div className="worlds-body">
          <WorldRail
            worlds={worlds}
            loading={loading}
            selected={selected?.dir_name ?? null}
            onSelect={(dirName) => {
              select(dirName);
              setSelectedFilename(null);
              setLockedNotice(null);
              setImportPrompt(null);
              setImportError(null);
            }}
          />
          <div className="worlds-main">
            {selected ? (
              <>
                <WorldHeader
                  world={selected}
                  gameRunning={gameRunning}
                  refreshGameRunning={refreshGameRunning}
                />
                {lockedNotice ? (
                  <p className="banner" role="status">
                    {lockedNotice}
                  </p>
                ) : null}
                {actionError ? (
                  <p className="error" role="alert">
                    {actionError.message}
                  </p>
                ) : null}
                {packsError && !stack ? (
                  <p className="error" role="alert">
                    {packsError.message}{' '}
                    <button type="button" className="button" onClick={() => void refreshListing()}>
                      Retry
                    </button>
                  </p>
                ) : null}
                <div className="worlds-toolbar">
                  <button
                    type="button"
                    className="button button-primary"
                    disabled={!selected || readOnly || busy}
                    title={readOnly ? 'This world is read-only' : undefined}
                    onClick={() => void handleImportPick(false)}
                  >
                    Import zip
                  </button>
                  <button
                    type="button"
                    className="button"
                    disabled={!selected || readOnly || busy}
                    title={readOnly ? 'This world is read-only' : undefined}
                    onClick={() => void handleImportPick(true)}
                  >
                    Import folder
                  </button>
                </div>
                {importError ? (
                  <p className="error" role="alert">
                    {importError}
                  </p>
                ) : null}
                {importPrompt ? (
                  <div className="import-prompt" role="alert">
                    <p>
                      {importPrompt.status === 'duplicate'
                        ? `${importPrompt.name} is already installed.`
                        : `${importPrompt.name} already exists.`}
                    </p>
                    <button
                      type="button"
                      className="button"
                      onClick={() => importPath && selected && void runImport(selected.dir_name, importPath, 'replace')}
                    >
                      Replace
                    </button>
                    <button
                      type="button"
                      className="button"
                      onClick={() => importPath && selected && void runImport(selected.dir_name, importPath, 'skip')}
                    >
                      Skip
                    </button>
                    <button type="button" className="button" onClick={() => setImportPrompt(null)}>
                      Cancel
                    </button>
                  </div>
                ) : null}
                {!readOnly ? (
                  <DatapackGrid
                    rows={rows}
                    loading={packsLoading || stackLoading || busy}
                    selectedFilename={selectedFilename}
                    onSelect={setSelectedFilename}
                    writesBlocked={writesBlocked || busy}
                    onToggle={(filename, enabled) => void handleToggle(filename, enabled)}
                    onMove={(filename, direction) => void handleMove(filename, direction)}
                  />
                ) : (
                  <p className="empty-note">
                    The datapack controls are hidden while this world is read-only.
                  </p>
                )}
                <DatapackDetail
                  pack={selectedRow?.pack ?? null}
                  filename={selectedRow?.filename ?? null}
                  enabled={selectedRow?.enabled ?? false}
                  locked={selectedRow?.locked ?? false}
                  warning={selectedRow?.warning ?? null}
                  writesBlocked={writesBlocked || busy}
                  busy={busy}
                  onToggle={(filename, enabled) => void handleToggle(filename, enabled)}
                  onDelete={(path) => void handleDelete(path)}
                />
              </>
            ) : (
              <p className="empty-note">
                {loading ? 'Loading worlds…' : 'Select a world to see its datapacks.'}
              </p>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

function filenameOf(path: string): string {
  return path.split('/').filter(Boolean).at(-1) ?? path;
}

function isFileValue(value: string): boolean {
  return value.startsWith('file/');
}
