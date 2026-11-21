import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ipc, toAppError } from '../../lib/ipc';
import type { IpcClient } from '../../lib/ipc';
import type { GameRunning, ModrinthKind, World } from '../../lib/types';
import GameRunningBanner from '../packs/GameRunningBanner';
import type { GuardPause } from '../packs/GameRunningBanner';
import InstallSheet from './InstallSheet';
import ProjectDetail from './ProjectDetail';
import ProjectGrid from './ProjectGrid';
import SearchBar from './SearchBar';
import { useModrinth } from './useModrinth';

type WorldsLoad =
  | { status: 'loading' }
  | { status: 'failed'; message: string }
  | { status: 'ready'; worlds: World[] };

interface DiscoverPageProps {
  active?: boolean;
  reloadSignal?: string;
  gameDir?: string | null;
  client?: IpcClient;
  gameRunning: GameRunning | null;
  guardPause?: GuardPause;
  guardBusy?: boolean;
  lockDetectionSupported?: boolean;
  /** Stale locks from the app-level game guard. Discover has no resource stack. */
  staleLocks?: string[];
  /** False when no game dir is active: installs have nowhere to go. */
  hasGameDir?: boolean;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
  onInstalled?: (kind: ModrinthKind) => void;
}

export default function DiscoverPage({
  client = ipc,
  active = true,
  reloadSignal = '',
  gameDir = null,
  gameRunning,
  guardPause = null,
  guardBusy = false,
  lockDetectionSupported,
  staleLocks = [],
  hasGameDir = false,
  refreshGameRunning = () => {},
  onInstalled,
}: DiscoverPageProps) {
  const discover = useModrinth(client, active, reloadSignal, gameDir, refreshGameRunning, onInstalled);
  const [worldsLoad, setWorldsLoad] = useState<WorldsLoad>({ status: 'loading' });
  const [worldsAttempt, setWorldsAttempt] = useState(0);
  const worlds = worldsLoad.status === 'ready' ? worldsLoad.worlds : null;
  const [installOpen, setInstallOpen] = useState(false);

  const loadedWorlds = useRef<string | null>(null);

  useLayoutEffect(() => {
    loadedWorlds.current = null;
    setWorldsLoad({ status: 'loading' });
  }, [gameDir]);

  useEffect(() => {
    let cancelled = false;
    if (!active || discover.kind !== 'datapack' || loadedWorlds.current === reloadSignal) return;
    setWorldsLoad((current) => (current.status === 'ready' ? current : { status: 'loading' }));
    client
      .listWorlds()
      .then((loaded) => {
        if (!cancelled) {
          loadedWorlds.current = reloadSignal;
          setWorldsLoad({
            status: 'ready',
            worlds: loaded.filter((world) => !world.issue),
          });
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setWorldsLoad({ status: 'failed', message: toAppError(err).message });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, discover.kind, active, reloadSignal, gameDir, worldsAttempt]);

  const reloadWorlds = () => {
    loadedWorlds.current = null;
    setWorldsAttempt((attempt) => attempt + 1);
  };

  const { world: targetWorld, setWorld } = discover;
  useEffect(() => {
    if (targetWorld && worlds && !worlds.some((world) => world.dir_name === targetWorld)) setWorld(null);
  }, [worlds, targetWorld, setWorld]);

  const writesBlocked =
    gameRunning !== null ||
    guardPause !== null ||
    !hasGameDir ||
    discover.targetsStatus !== 'ready';
  const blockedReason = !hasGameDir
    ? null
    : gameRunning != null
      ? `Minecraft is running (${gameRunning.detail}); installs are paused until it closes.`
      : guardPause === 'unknown'
        ? 'Mine Utils could not check whether Minecraft is running; installs are paused until a check succeeds.'
        : guardPause === 'checking'
          ? 'Checking whether Minecraft is running; installs are paused until the check lands.'
          : null;

  const selectedVersion =
    discover.versions.find((version) => version.id === discover.selectedVersionId) ?? null;
  const selectedFile =
    selectedVersion?.files.find((item) => item.primary) ?? selectedVersion?.files[0] ?? null;
  // The "(compatible)" label is honest: only a version whose game_versions
  // actually contain the target gets it. A default newest pick is unlabeled.
  const compatibleId = discover.targetVersion
    ? (discover.versions.find((version) =>
        version.game_versions.includes(discover.targetVersion as string),
      )?.id ?? null)
    : null;

  const openInstall = () => setInstallOpen(true);
  const closeInstall = () => {
    setInstallOpen(false);
    discover.clearInstall();
  };

  return (
    <section className="panel">
      <h2>Discover</h2>
      <GameRunningBanner
        running={gameRunning}
        guardPause={guardPause}
        guardBusy={guardBusy}
        staleLocks={staleLocks}
        supported={lockDetectionSupported}
        onRecheck={() => void refreshGameRunning()}
      />
      <SearchBar
        query={discover.query}
        onQueryChange={discover.setQuery}
        kind={discover.kind}
        onKindChange={discover.setKind}
        gameVersion={discover.gameVersion}
        onGameVersionChange={discover.setGameVersion}
        installedVersions={discover.targets?.installed_versions ?? []}
        sort={discover.sort}
        onSortChange={discover.setSort}
        searching={discover.searching}
      />
      {discover.offline ? (
        <div className="packs-empty" role="alert">
          <p>Modrinth is offline. Check the network and try again.</p>
          <p className="empty-note">
            The local Resource Packs, Worlds, and Settings tabs keep working.
          </p>
          <button type="button" className="button" onClick={discover.retry}>
            Retry
          </button>
        </div>
      ) : null}
      {!discover.offline && discover.error ? (
        <p className="error" role="alert">
          {discover.error.message}{' '}
          <button type="button" className="button" onClick={discover.retry}>
            Retry
          </button>
        </p>
      ) : null}
      {!discover.offline && !discover.error && !discover.searched ? (
        <p className="empty">
          Search Modrinth for {discover.kind === 'datapack' ? 'datapacks' : 'resource packs'}.
          Nothing is downloaded until you install.
        </p>
      ) : null}
      {!discover.offline &&
      !discover.error &&
      discover.searched &&
      discover.hits.length === 0 &&
      !discover.searching ? (
        <p className="empty">
          No {discover.kind === 'datapack' ? 'datapacks' : 'resource packs'} match. Try another
          search.
        </p>
      ) : null}
      {discover.hits.length > 0 ? (
        <ProjectGrid
          hits={discover.hits}
          selectedId={discover.selected?.project_id ?? null}
          onSelect={(hit) => void discover.select(hit)}
        />
      ) : null}
      {discover.hits.length > 0 && (discover.hasMore || discover.loadingMore) ? (
        <div className="discover-more">
          {discover.loadMoreError ? (
            <p className="error" role="alert">
              {discover.loadMoreError.message}
            </p>
          ) : null}
          <button
            type="button"
            className="button"
            onClick={() => void discover.loadMore()}
            disabled={discover.loadingMore}
          >
            {discover.loadingMore ? 'Loading more…' : discover.loadMoreError ? 'Retry' : 'Load more'}
          </button>
        </div>
      ) : null}
      {/* One dialog at a time: the install sheet replaces the detail sheet
          instead of stacking on top of it. */}
      {active && (installOpen && discover.selected ? (
        <InstallSheet
          open
          kind={discover.kind}
          projectTitle={discover.selected.title}
          versionLabel={
            selectedVersion?.version_number ||
            selectedVersion?.name ||
            selectedVersion?.id ||
            ''
          }
          filename={selectedFile?.filename ?? ''}
          worlds={worlds ?? []}
          worldsLoading={worldsLoad.status === 'loading'}
          worldsError={worldsLoad.status === 'failed' ? worldsLoad.message : null}
          onRetryWorlds={reloadWorlds}
          world={discover.world}
          onWorldChange={discover.setWorld}
          phase={discover.installPhase}
          writesBlocked={writesBlocked}
          blockedReason={blockedReason}
          targetsStatus={discover.targetsStatus}
          onRetryTargets={discover.reloadTargets}
          hasGameDir={hasGameDir}
          result={discover.installResult}
          error={discover.installError}
          onClose={closeInstall}
          onInstall={() => void discover.install()}
          onCancel={() => void discover.cancelInstall()}
        />
      ) : discover.selected ? (
        <ProjectDetail
          hit={discover.selected}
          project={discover.project}
          versions={discover.versions}
          loading={discover.detailLoading}
          error={discover.detailError}
          selectedVersionId={discover.selectedVersionId}
          onSelectVersion={discover.setSelectedVersionId}
          compatibleId={compatibleId}
          targetLabel={discover.targetVersion}
          onInstall={openInstall}
          onClose={discover.closeDetail}
        />
      ) : null)}
    </section>
  );
}
