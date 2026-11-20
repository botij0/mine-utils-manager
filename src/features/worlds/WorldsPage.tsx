import HoldingRecovery from '../packs/HoldingRecovery';
import { useCallback, useMemo, useRef, useState } from 'react';
import type { WorldsClient } from '../../lib/ipc';
import { ipc, pickPackPath, toAppError } from '../../lib/ipc';
import { formatVersionLabel } from '../../lib/format';
import { packFilename } from '../../lib/packPath';
import type { DatapackImportOutcome, GameRunning } from '../../lib/types';
import type { GuardPause } from '../packs/GameRunningBanner';
import { StatusNote } from '../packs/PackEmptyState';
import DatapackDetail from './DatapackDetail';
import DatapackGrid, { type DatapackRow } from './DatapackGrid';
import WorldHeader from './WorldHeader';
import WorldRail from './WorldRail';
import { compatibilityWarning, useWorlds } from './useWorlds';

interface WorldsPageProps {
  active?: boolean;
  reloadSignal?: string;
  gameDir?: string | null;
  client?: WorldsClient;
  onDataChanged?: () => void;
  gameRunning: GameRunning | null;
  guardPause?: GuardPause;
  guardBusy?: boolean;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
}

export default function WorldsPage({
  client = ipc,
  active = true,
  reloadSignal = '',
  gameDir = null,
  onDataChanged,
  gameRunning: running,
  guardPause = null,
  guardBusy = false,
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
    holdings,
    restoreHolding,
    busy,
    listingWorld,
    actionError,
    clearStaleActionError,
    reload,
    refreshListing,
    toggleDatapack,
    reorderDatapacks,
    importDatapack,
    deleteDatapack,
    removeDatapackReference,
    clearActionError,
  } = useWorlds(client, active, reloadSignal, gameDir);
  const [selectedFilename, setSelectedFilename] = useState<string | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [importPrompt, setImportPrompt] = useState<
    Extract<DatapackImportOutcome, { status: 'duplicate' } | { status: 'collision' }> | null
  >(null);
  const [importPath, setImportPath] = useState<string | null>(null);
  const [lockedNotice, setLockedNotice] = useState<string | null>(null);
  const [referenceNotice, setReferenceNotice] = useState<string | null>(null);
  // The selection at the time an import started: an outcome that lands after
  // a world switch applies to the wrong world, so it is dropped.
  const selectedDirRef = useRef<string | null>(null);
  selectedDirRef.current = selected?.dir_name ?? null;

  // Re-check everything after a lock release: queue an app guard check, then
  // reread the world's listing and the world list itself (a release is never
  // blocked by issue).
  const refreshAfterGuard = useCallback(async () => {
    void refreshGameRunning();
    const applied = await refreshListing();
    await reload();
    onDataChanged?.();
    return applied;
  }, [refreshGameRunning, refreshListing, reload, onDataChanged]);

  const retryStaleListing = () => {
    void refreshAfterGuard().then((applied) => {
      if (!applied) return;
      clearStaleActionError();
    });
  };

  const worldFormat = selected?.version?.data_format ?? null;
  const worldLabel = selected?.version?.name ?? selected?.level_name ?? null;

  const rows = useMemo<DatapackRow[]>(() => {
    if (!stack) return [];
    const byFilename = new Map(packs.filter((pack) => pack.error?.code !== 'pack_non_utf8_name').map((pack) => [packFilename(pack), pack]));
    const listed = new Set([
      ...stack.enabled.map((entry) => entry.value),
      ...stack.disabled.map((entry) => entry.value),
    ]);
    const reversed = [...stack.enabled].reverse();
    const topFirst = reversed.map((entry, display) => {
      const filename = entry.kind === 'file' ? entry.filename : entry.value;
      const pack = entry.kind === 'file' ? (byFilename.get(entry.filename) ?? null) : null;
      // Positions, not values: duplicates share a value but not a position.
      // Up the display means later in load order; the neighbor on that side
      // must be a movable `file/` entry.
      const neighborUp = display > 0 ? reversed[display - 1] : null;
      const neighborDown = display < reversed.length - 1 ? reversed[display + 1] : null;
      return {
        pack,
        entry,
        enabled: true,
        locked: entry.kind === 'foreign',
        missing: entry.kind === 'file' && pack === null,
        warning:
          entry.kind === 'file' && pack
            ? compatibilityWarning(pack, worldFormat, worldLabel)
            : null,
        filename,
        canMoveUp: entry.kind === 'file' && neighborUp?.kind === 'file',
        canMoveDown: entry.kind === 'file' && neighborDown?.kind === 'file',
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
        missing: pack === null,
        warning: pack ? compatibilityWarning(pack, worldFormat, worldLabel) : null,
        filename: entry.filename,
        canMoveUp: false,
        canMoveDown: false,
      };
      return [row];
    });
    // On-disk packs that `level.dat` never names (hand-dropped folders) get
    // an installed row with an Enable action instead of being invisible.
    const installed = packs.flatMap((pack) => {
      const filename = packFilename(pack);
      const value = `file/${filename}`;
      if (listed.has(value) && pack.error?.code !== 'pack_non_utf8_name') return [];
      const row: DatapackRow = {
        pack,
        entry: { kind: 'file', value, filename },
        enabled: false,
        locked: false,
        missing: false,
        warning: compatibilityWarning(pack, worldFormat, worldLabel),
        filename,
        canMoveUp: false,
        canMoveDown: false,
      };
      return [row];
    });
    return [...topFirst, ...disabled, ...installed];
  }, [stack, packs, worldFormat, worldLabel]);

  const selectedRow = rows.find((row) => (row.pack?.error?.code === 'pack_non_utf8_name' ? row.pack.id : row.filename) === selectedFilename) ?? null;

  const locked = stack?.world_locked ?? selected?.world_locked ?? null;
  const readOnly = selected?.issue !== null && selected?.issue !== undefined;
  const listingStale = listingWorld !== selected?.dir_name;
  const writesBlocked = running !== null || locked !== null || readOnly || guardPause !== null || listingStale;
  const noGameDir = error?.code === 'no_active_game_dir' || packsError?.code === 'no_active_game_dir';

  const refreshAfterWriteError = (err: unknown) => {
    const code = toAppError(err).code;
    // A stale stack or a lock release needs a fresh listing, guard, and
    // world list; anything else keeps its error card without a refetch.
    if (code === 'game_running' || code === 'stale_stack') void refreshAfterGuard();
  };

  const handleToggle = async (filename: string, enabled: boolean) => {
    clearActionError();
    try {
      await toggleDatapack(filename, enabled);
      void refreshGameRunning();
    } catch (err) {
      refreshAfterWriteError(err);
    }
  };

  // The grid is top-first, so position `index` maps to load order reversed.
  const handleMove = async (index: number, direction: -1 | 1) => {
    if (!stack) return;
    const topFirst = [...stack.enabled].reverse();
    const from = index;
    const to = index + direction;
    if (from < 0 || to < 0 || from >= topFirst.length || to >= topFirst.length) return;
    const moving = topFirst[from];
    const target = topFirst[to];
    // Only file entries move; a swap that touches a foreign entry is refused.
    if (moving.kind !== 'file' || target.kind !== 'file') return;
    const next = [...topFirst];
    [next[from], next[to]] = [next[to], next[from]];
    const order = [...next].reverse().map((entry) => entry.value);
    clearActionError();
    try {
      await reorderDatapacks(order);
    } catch (err) {
      refreshAfterWriteError(err);
    }
  };

  const handleDelete = async (path: string) => {
    const world = selectedDirRef.current;
    const worldName = selected?.level_name ?? selected?.dir_name ?? 'this world';
    clearActionError();
    setReferenceNotice(null);
    try {
      const outcome = await deleteDatapack(path);
      if (selectedDirRef.current !== world) return;
      setSelectedFilename(null);
      if (!outcome.removedReferences) {
        setReferenceNotice(
          `${packFilename({ path })} was moved to the trash, but ${worldName} still references it.`,
        );
      }
    } catch (err) {
      refreshAfterWriteError(err);
    }
  };

  const handleRemoveReference = async (filename: string) => {
    clearActionError();
    try {
      await removeDatapackReference(filename);
    } catch (err) {
      refreshAfterWriteError(err);
    }
  };

  const runImport = async (world: string, path: string, choice: 'default' | 'skip' | 'replace' = 'default') => {
    setImportError(null);
    try {
      const outcome = await importDatapack(world, path, choice);
      // A switch mid-import leaves this outcome addressing the old world.
      if (selectedDirRef.current !== world) return outcome;
      if (outcome.status === 'imported' || outcome.status === 'locked') {
        onDataChanged?.();
        void refreshGameRunning();
      }
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
      if (selectedDirRef.current !== world) return;
      setImportError(toAppError(err).message);
      refreshAfterWriteError(err);
    }
  };

  const handleImportPick = async (directory: boolean) => {
    if (!selected) return;
    setImportError(null);
    const path = await pickPackPath(directory, 'Import a datapack');
    if (!path) return;
    await runImport(selected.dir_name, path);
  };

  return (
    <section className="worlds-page">
      {error && !noGameDir ? (
        <p className="error" role="alert">
          {error.message}{' '}
          <button type="button" className="button" onClick={() => void reload()}>
            Retry
          </button>
        </p>
      ) : null}
      {noGameDir ? (
        <StatusNote>Pick a game dir in Settings first.</StatusNote>
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
              setReferenceNotice(null);
              setImportPrompt(null);
              setImportError(null);
            }}
          />
          <div className="worlds-main">
            {selected ? (
              <>
                <WorldHeader
                  world={selected}
                  gameRunning={running}
                  guardPause={guardPause}
                  guardBusy={guardBusy}
                  staleLocks={locked || running ? [] : (stack?.stale_locks ?? [])}
                  refreshGameRunning={refreshGameRunning}
                  onRefresh={() => void refreshAfterGuard()}
                />
                {lockedNotice ? (
                  <p className="banner" role="status">
                    {lockedNotice}
                  </p>
                ) : null}
                {referenceNotice ? (
                  <p className="banner" role="status">
                    {referenceNotice}
                  </p>
                ) : null}
                {actionError ? (
                  <p className="error" role="alert">
                    {actionError.message}{' '}
                    {actionError.code === 'stale_stack' ? (
                      <button type="button" className="button" onClick={retryStaleListing}>
                        Retry
                      </button>
                    ) : null}
                  </p>
                ) : null}
                {packsError && !stack && !readOnly ? (
                  <p className="error" role="alert">
                    {packsError.message}{' '}
                    <button type="button" className="button" onClick={() => void refreshAfterGuard()}>
                      Retry
                    </button>
                  </p>
                ) : null}
                <HoldingRecovery entries={holdings} disabled={busy || writesBlocked} onRestore={(name) => {
                  void restoreHolding(name).catch(() => { void refreshGameRunning(); });
                }} />
                <div className="worlds-toolbar">
                  <button
                    type="button"
                    className="button button-primary"
                    disabled={!selected || readOnly || busy || running !== null || guardPause !== null || listingStale}
                    title={readOnly ? 'This world is read-only' : undefined}
                    onClick={() => void handleImportPick(false)}
                  >
                    Import zip
                  </button>
                  <button
                    type="button"
                    className="button"
                    disabled={!selected || readOnly || busy || running !== null || guardPause !== null || listingStale}
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
                      disabled={writesBlocked || busy}
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
                    loading={packsLoading || stackLoading}
                    selectedFilename={selectedFilename}
                    onSelect={setSelectedFilename}
                    writesBlocked={writesBlocked || busy}
                    onToggle={(filename, enabled) => void handleToggle(filename, enabled)}
                    onMove={(index, direction) => void handleMove(index, direction)}
                  />
                ) : (
                  <StatusNote>
                    The datapack controls are hidden while this world is read-only.
                  </StatusNote>
                )}
                <DatapackDetail
                  pack={selectedRow?.pack ?? null}
                  filename={selectedRow?.filename ?? null}
                  enabled={selectedRow?.enabled ?? false}
                  locked={selectedRow?.locked ?? false}
                  missing={selectedRow?.missing ?? false}
                  warning={selectedRow?.warning ?? null}
                  writesBlocked={writesBlocked || busy}
                  busy={busy}
                  onToggle={(filename, enabled) => void handleToggle(filename, enabled)}
                  onDelete={(path) => void handleDelete(path)}
                  onRemoveReference={(filename) => void handleRemoveReference(filename)}
                />
                {worldFormat ? (
                  <p className="panel-note">
                    This world needs data format {formatVersionLabel(worldFormat)}.
                  </p>
                ) : null}
              </>
            ) : (
              <StatusNote>
                {loading ? 'Loading worlds…' : 'Select a world to see its datapacks.'}
              </StatusNote>
            )}
          </div>
        </div>
      )}
    </section>
  );
}