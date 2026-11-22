import { useCallback, useEffect, useRef, useState } from 'react';
import { ipc } from './lib/ipc';
import type { IpcClient } from './lib/ipc';
import type { ClientError, Config, GameGuard, GameRunning, Theme } from './lib/types';
import { defaultConfig } from './lib/types';
import { activeGameRunning, toAppError } from './lib/ipc';
import DiscoverPage from './features/discover/DiscoverPage';
import Onboarding from './features/onboarding/Onboarding';
import type { GuardPause } from './features/packs/GameRunningBanner';
import PacksPage from './features/packs/PacksPage';
import SettingsPage from './features/settings/SettingsPage';
import WorldsPage from './features/worlds/WorldsPage';

type PageId = 'packs' | 'worlds' | 'discover' | 'settings';

const GUARD_POLL_MS = 5000;
const GUARD_TIMEOUT_MS = 3 * GUARD_POLL_MS;

const PAGES: { id: PageId; label: string }[] = [
  { id: 'packs', label: 'Resource Packs' },
  { id: 'worlds', label: 'Worlds' },
  { id: 'discover', label: 'Discover' },
  { id: 'settings', label: 'Settings' },
];

const DEFAULT_PAGE: PageId = 'packs';

function processProbeBanner(
  processProbeAvailable: boolean | undefined,
  lockDetectionSupported: boolean | undefined,
  guardPause: GuardPause,
  gameRunning: GameRunning | null,
) {
  if (
    processProbeAvailable !== false ||
    lockDetectionSupported !== true ||
    guardPause !== null ||
    gameRunning
  ) {
    return null;
  }
  return (
    <div className="banner" role="status">
      Minecraft process detection is unavailable. Session locks are still checked; writes are allowed.
    </div>
  );
}

function pageFromHash(): PageId | null {
  const hash = window.location.hash.replace('#', '');
  return PAGES.some((item) => item.id === hash) ? (hash as PageId) : null;
}

function resolvedTheme(theme: Theme): 'light' | 'dark' {
  if (theme === 'system') {
    return window.matchMedia('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light';
  }
  return theme;
}

function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = resolvedTheme(theme);
}

interface AppProps {
  client?: IpcClient;
}

export default function App({ client = ipc }: AppProps) {
  const [page, setPage] = useState<PageId>(() => pageFromHash() ?? DEFAULT_PAGE);
  const [visited, setVisited] = useState<PageId[]>([]);
  useEffect(() => {
    setVisited((previous) => previous.includes(page) ? previous : [...previous, page]);
  }, [page]);
  const [config, setConfig] = useState<Config>(defaultConfig);
  const [error, setError] = useState<ClientError | null>(null);
  const [startupWarning, setStartupWarning] = useState<string | null>(null);
  const [guardState, setGuardState] = useState<{
    dir: string | null;
    answer: GameGuard | null;
    pause: GuardPause;
  }>({ dir: null, answer: null, pause: 'checking' });
  const [guardBusy, setGuardBusy] = useState(false);
  const currentGuard = guardState.dir === config.active_game_dir ? guardState : null;
  const guardPause = currentGuard ? currentGuard.pause : 'checking';
  const gameRunning = currentGuard?.answer?.running ?? null;
  const processProbeAvailable = currentGuard?.answer?.process_probe_available;
  const lockDetectionSupported = currentGuard?.answer?.detection_supported;
  const staleLocks = currentGuard?.answer?.stale_locks ?? [];
  // True once the stored config lands, so onboarding only shows for a real
  // first run and never for the default placeholder.
  const [configReady, setConfigReady] = useState(false);
  // Dismissed by choosing a dir or skipping; Settings stays open for later.
  const [onboarded, setOnboarded] = useState(false);
  const [dataEpoch, setDataEpoch] = useState({ packs: 0, worlds: 0, discover: 0 });
  // At most one get_game_guard is outstanding: a hung call parks a backend
  // thread, so only its real settle frees the slot. A check wanted meanwhile
  // runs once the slot frees. The outstanding answer goes stale when it
  // outlives its deadline, the folder changes, or App unmounts, and a stale
  // answer is dropped.
  const guard = useRef<{
    busy: boolean;
    wanted: boolean;
    stale: boolean;
    deadline?: ReturnType<typeof setTimeout>;
    dir: string | null;
  }>({ busy: false, wanted: false, stale: false, dir: null });

  useEffect(() => {
    const onHashChange = () => {
      const hashPage = pageFromHash();
      if (hashPage) setPage(hashPage);
      else window.history.replaceState(null, '', `#${page}`);
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, [page]);

  // A click to another page adds one history entry, so Back returns here. An
  // empty or unknown hash is rewritten in place, so it adds none.
  useEffect(() => {
    const hashPage = pageFromHash();
    if (hashPage === page) return;
    if (hashPage) window.location.hash = page;
    else window.history.replaceState(null, '', `#${page}`);
  }, [page]);

  useEffect(() => {
    client
      .getConfig()
      .then((loaded) => {
        setConfig(loaded);
        setConfigReady(true);
      })
      .catch((err: unknown) => {
        setError(toAppError(err));
        setConfigReady(true);
      });
  }, [client]);

  const armGuardDeadline = useCallback(() => {
    const slot = guard.current;
    clearTimeout(slot.deadline);
    slot.deadline = setTimeout(() => {
      slot.stale = true;
      setGuardState((state) => ({ ...state, pause: 'unknown' }));
    }, GUARD_TIMEOUT_MS);
  }, []);

  const refreshGameRunning = useCallback(() => {
    const slot = guard.current;
    const send = () => {
      const dir = slot.dir;
      slot.busy = true;
      slot.wanted = false;
      slot.stale = false;
      setGuardBusy(true);
      armGuardDeadline();
      const settle = (apply: () => void) => {
        slot.busy = false;
        clearTimeout(slot.deadline);
        if (!slot.stale) apply();
        if (slot.wanted && !document.hidden) send();
        else setGuardBusy(false);
      };
      activeGameRunning(client).then(
        (answer) => settle(() => setGuardState({ dir, answer, pause: null })),
        // Keep the last known value: only a successful check may clear it.
        () => settle(() => setGuardState((state) => ({ ...state, pause: 'unknown' }))),
      );
    };
    if (slot.busy || document.hidden) slot.wanted = true;
    else send();
  }, [client, armGuardDeadline]);

  // Writes on any page stop while Minecraft runs. The callback is stable, so
  // pages can force a fresh check without re-subscribing the poll. A new
  // folder or a finished onboarding checks at once instead of waiting a tick.
  useEffect(() => {
    if (!configReady) return undefined;
    const slot = guard.current;
    const folderChanged = slot.dir !== config.active_game_dir;
    slot.dir = config.active_game_dir;
    if (folderChanged) setGuardState({ dir: slot.dir, answer: null, pause: 'checking' });
    if (folderChanged && slot.busy) {
      slot.stale = true;
      armGuardDeadline();
    }
    const tick = () => {
      if (!document.hidden) refreshGameRunning();
    };
    tick();
    const timer = setInterval(tick, GUARD_POLL_MS);
    document.addEventListener('visibilitychange', tick);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', tick);
    };
  }, [configReady, config.active_game_dir, onboarded, refreshGameRunning, armGuardDeadline]);

  useEffect(() => {
    const slot = guard.current;
    return () => {
      clearTimeout(slot.deadline);
      slot.stale = true;
      slot.wanted = false;
    };
  }, []);

  useEffect(() => {
    client
      .getStartupWarning()
      .then(setStartupWarning)
      .catch(() => setStartupWarning(null));
  }, [client]);

  useEffect(() => {
    applyTheme(config.theme);
    if (config.theme !== 'system') return undefined;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => applyTheme('system');
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  }, [config.theme]);

  const worldsChanged = useCallback(() => {
    setDataEpoch((epoch) => ({ ...epoch, discover: epoch.discover + 1 }));
  }, []);
  const installed = useCallback((kind: 'resourcepack' | 'datapack') => {
    const page = kind === 'resourcepack' ? 'packs' : 'worlds';
    setDataEpoch((epoch) => ({ ...epoch, [page]: epoch[page] + 1 }));
  }, []);

  const backupRestored = useCallback((file: 'options' | 'level') => {
    setDataEpoch((epoch) =>
      file === 'options'
        ? { ...epoch, packs: epoch.packs + 1 }
        : { ...epoch, worlds: epoch.worlds + 1, discover: epoch.discover + 1 },
    );
  }, []);

  const activeLabel = config.active_game_dir ?? 'No game dir selected';
  const showOnboarding =
    configReady && !onboarded && !config.active_game_dir && !error;

  const finishOnboarding = useCallback(() => {
    setOnboarded(true);
    client
      .getConfig()
      .then(setConfig)
      .catch((err: unknown) => setError(toAppError(err)));
  }, [client]);

  if (showOnboarding) {
    return (
      <div className="app">
        <main className="content">
          {processProbeBanner(
            processProbeAvailable,
            lockDetectionSupported,
            guardPause,
            gameRunning,
          )}
          <Onboarding client={client} onDone={finishOnboarding} />
        </main>
      </div>
    );
  }

  return (
    <div className="app">
      <nav className="sidebar" aria-label="Sections">
        <div className="brand">Mine Utils</div>
        <ul className="nav">
          {PAGES.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className="nav-button"
                aria-current={page === item.id ? 'page' : undefined}
                onClick={() => setPage(item.id)}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <div className="main">
        {startupWarning ? (
          <div className="banner" role="status">
            {startupWarning}
          </div>
        ) : null}
        <header className="header">
          <h1>{PAGES.find((item) => item.id === page)?.label}</h1>
          <button
            type="button"
            className="active-dir"
            onClick={() => setPage('settings')}
          >
            <span className="label">Active game dir</span>
            <span className="path">{activeLabel}</span>
          </button>
        </header>
        <main className="content">
          {processProbeBanner(
            processProbeAvailable,
            lockDetectionSupported,
            guardPause,
            gameRunning,
          )}
          {error ? (
            <p className="error" role="alert">
              {error.message}
            </p>
          ) : null}
          {page === 'packs' || visited.includes('packs') ? (
            <div hidden={page !== 'packs'}>
              <PacksPage
                active={page === 'packs'}
                reloadSignal={`${dataEpoch.packs}`}
                gameDir={config.active_game_dir}
                client={client}
                gameRunning={gameRunning}
                lockDetectionSupported={lockDetectionSupported}
                staleLocks={staleLocks}
                guardPause={guardPause}
                guardBusy={guardBusy}
                refreshGameRunning={refreshGameRunning}
              />
            </div>
          ) : null}
          {page === 'worlds' || visited.includes('worlds') ? (
            <div hidden={page !== 'worlds'}>
              <WorldsPage
                active={page === 'worlds'}
                onDataChanged={worldsChanged}
                reloadSignal={`${dataEpoch.worlds}`}
                gameDir={config.active_game_dir}
                client={client}
                gameRunning={gameRunning}
                guardPause={guardPause}
                guardBusy={guardBusy}
                refreshGameRunning={refreshGameRunning}
              />
            </div>
          ) : null}
          {page === 'discover' || visited.includes('discover') ? (
            <div hidden={page !== 'discover'}>
              <DiscoverPage
                active={page === 'discover'}
                reloadSignal={`${dataEpoch.discover}`}
                gameDir={config.active_game_dir}
                client={client}
                gameRunning={gameRunning}
                lockDetectionSupported={lockDetectionSupported}
                staleLocks={staleLocks}
                guardPause={guardPause}
                guardBusy={guardBusy}
                hasGameDir={config.active_game_dir != null}
                refreshGameRunning={refreshGameRunning}
                onInstalled={installed}
              />
            </div>
          ) : null}
          {page === 'settings' || visited.includes('settings') ? (
            <div hidden={page !== 'settings'}>
              <SettingsPage
                active={page === 'settings'}
                client={client}
                config={config}
                gameRunning={gameRunning}
                guardPause={guardPause}
                refreshGameRunning={refreshGameRunning}
                onConfigChanged={setConfig}
                onBackupRestored={backupRestored}
              />
            </div>
          ) : null}
        </main>
      </div>
    </div>
  );
}
