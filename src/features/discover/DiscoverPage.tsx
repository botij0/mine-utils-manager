import { useEffect, useState } from 'react';
import { ipc } from '../../lib/ipc';
import type { IpcClient } from '../../lib/ipc';
import type { GameRunning, World } from '../../lib/types';
import GameRunningBanner from '../packs/GameRunningBanner';
import InstallSheet from './InstallSheet';
import ProjectDetail from './ProjectDetail';
import ProjectGrid from './ProjectGrid';
import SearchBar from './SearchBar';
import { preferModrinthVersion, useModrinth } from './useModrinth';

interface DiscoverPageProps {
  client?: IpcClient;
  /** From the app-level game guard poll. Undefined until the first check. */
  gameRunning?: GameRunning | null;
  /** A failed guard check: unknown, so installs stay paused. */
  gameGuardUnknown?: boolean;
  /** Force the app-level game guard to re-check now. */
  refreshGameRunning?: () => void | Promise<void>;
  /** Fires after a successful install so the app refreshes Packs/Worlds. */
  onInstalled?: () => void;
}

export default function DiscoverPage({
  client = ipc,
  gameRunning,
  gameGuardUnknown = false,
  refreshGameRunning = () => {},
  onInstalled = () => {},
}: DiscoverPageProps) {
  const discover = useModrinth(client);
  const [worlds, setWorlds] = useState<World[]>([]);
  const [installOpen, setInstallOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    if (discover.kind !== 'datapack') return () => {};
    client
      .listWorlds()
      .then((loaded) => {
        if (!cancelled) setWorlds(loaded.filter((world) => !world.issue));
      })
      .catch(() => {
        if (!cancelled) setWorlds([]);
      });
    return () => {
      cancelled = true;
    };
  }, [client, discover.kind]);

  const writesBlocked = gameRunning !== undefined && (gameRunning !== null || gameGuardUnknown);
  const blockedReason =
    gameRunning != null
      ? `Minecraft is running (${gameRunning.detail}); installs are paused until it closes.`
      : gameGuardUnknown
        ? 'The game guard is unavailable, so installs are paused.'
        : null;

  const preferred = preferModrinthVersion(discover.versions, discover.targetVersion);
  const selectedVersion =
    discover.versions.find((version) => version.id === discover.selectedVersionId) ?? null;
  const selectedFile =
    selectedVersion?.files.find((item) => item.primary) ?? selectedVersion?.files[0] ?? null;

  const openInstall = () => setInstallOpen(true);
  const closeInstall = () => {
    setInstallOpen(false);
    discover.clearInstall();
  };

  const onInstall = async () => {
    const outcome = await discover.install();
    if (outcome) onInstalled();
  };

  return (
    <section className="panel">
      <h2>Discover</h2>
      {gameRunning !== undefined ? (
        <GameRunningBanner
          running={gameRunning}
          guardUnknown={gameGuardUnknown}
          staleLocks={[]}
          supported
          onRecheck={() => void refreshGameRunning()}
        />
      ) : null}
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
      {discover.selected ? (
        <ProjectDetail
          hit={discover.selected}
          project={discover.project}
          versions={discover.versions}
          loading={discover.detailLoading}
          error={discover.detailError}
          selectedVersionId={discover.selectedVersionId}
          onSelectVersion={discover.setSelectedVersionId}
          preferredId={preferred?.id ?? null}
          targetLabel={discover.targetVersion}
          onInstall={openInstall}
          onClose={discover.closeDetail}
        />
      ) : null}
      {installOpen && discover.selected ? (
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
          worlds={worlds}
          world={discover.world}
          onWorldChange={discover.setWorld}
          busy={discover.installBusy}
          writesBlocked={writesBlocked}
          blockedReason={blockedReason}
          result={discover.installResult}
          error={discover.installError}
          onClose={closeInstall}
          onInstall={() => void onInstall()}
        />
      ) : null}
    </section>
  );
}
