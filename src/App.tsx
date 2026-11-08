import { useCallback, useEffect, useRef, useState } from 'react';
import { ipc } from './lib/ipc';
import type { IpcClient } from './lib/ipc';
import type { AppError, Config, GameRunning, Theme } from './lib/types';
import { defaultConfig } from './lib/types';
import { activeGameRunning, toAppError } from './lib/ipc';
import DiscoverPage from './features/discover/DiscoverPage';
import Onboarding from './features/onboarding/Onboarding';
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

function pageFromHash(): PageId {
  const hash = window.location.hash.replace('#', '');
  return PAGES.some((item) => item.id === hash) ? (hash as PageId) : 'packs';
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
  const [page, setPage] = useState<PageId>(pageFromHash);
  const [config, setConfig] = useState<Config>(defaultConfig);
  const [error, setError] = useState<AppError | null>(null);
  const [startupWarning, setStartupWarning] = useState<string | null>(null);
  // Undefined until the first check lands, so the pages can tell "not yet
  // checked" apart from a fresh "not running" result.
  const [gameRunning, setGameRunning] = useState<GameRunning | null | undefined>(undefined);
  const [guardPause, setGuardPause] = useState<'checking' | 'unknown' | null>(null);
  // True once the stored config lands, so onboarding only shows for a real
  // first run and never for the default placeholder.
  const [configReady, setConfigReady] = useState(false);
  // Dismissed by choosing a dir or skipping; Settings stays open for later.
  const [onboarded, setOnboarded] = useState(false);
  // Bumped after a Discover install so Packs/Worlds refetch on return.
  const [refreshEpoch, setRefreshEpoch] = useState(0);
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
    dir?: string | null;
  }>({ busy: false, wanted: false, stale: false });

  useEffect(() => {
    window.location.hash = page;
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
      setGuardPause('unknown');
    }, GUARD_TIMEOUT_MS);
  }, []);

  const refreshGameRunning = useCallback(() => {
    const slot = guard.current;
    const send = () => {
      slot.busy = true;
      slot.wanted = false;
      slot.stale = false;
      armGuardDeadline();
      const settle = (apply: () => void) => {
        slot.busy = false;
        clearTimeout(slot.deadline);
        if (!slot.stale) apply();
        if (slot.wanted) send();
      };
      activeGameRunning(client).then(
        (running) =>
          settle(() => {
            setGameRunning(running);
            setGuardPause(null);
          }),
        // Keep the last known value: only a successful check may clear it.
        () => settle(() => setGuardPause('unknown')),
      );
    };
    if (slot.busy) slot.wanted = true;
    else send();
  }, [client, armGuardDeadline]);

  // Writes on any page stop while Minecraft runs. The callback is stable, so
  // pages can force a fresh check without re-subscribing the poll. A new
  // folder or a finished onboarding checks at once instead of waiting a tick.
  useEffect(() => {
    if (!configReady) return undefined;
    const slot = guard.current;
    const folderChanged = slot.dir !== undefined && slot.dir !== config.active_game_dir;
    slot.dir = config.active_game_dir;
    if (folderChanged) setGuardPause('checking');
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
          {error ? (
            <p className="error" role="alert">
              {error.message}
            </p>
          ) : null}
          {page === 'packs' ? (
            <PacksPage
              key={`packs-${refreshEpoch}`}
              client={client}
              gameRunning={gameRunning}
              gameGuardUnknown={guardPause !== null}
              gameGuardChecking={guardPause === 'checking'}
              refreshGameRunning={refreshGameRunning}
            />
          ) : null}
          {page === 'worlds' ? (
            <WorldsPage
              key={`worlds-${refreshEpoch}`}
              client={client}
              gameRunning={gameRunning}
              gameGuardUnknown={guardPause !== null}
              gameGuardChecking={guardPause === 'checking'}
              refreshGameRunning={refreshGameRunning}
            />
          ) : null}
          {page === 'discover' ? (
            <DiscoverPage
              client={client}
              gameRunning={gameRunning}
              gameGuardUnknown={guardPause !== null}
              gameGuardChecking={guardPause === 'checking'}
              hasGameDir={config.active_game_dir != null}
              refreshGameRunning={refreshGameRunning}
              onInstalled={() => setRefreshEpoch((epoch) => epoch + 1)}
            />
          ) : null}
          {page === 'settings' ? (
            <SettingsPage
              client={client}
              config={config}
              gameRunning={gameRunning}
              gameGuardUnknown={guardPause !== null}
              gameGuardChecking={guardPause === 'checking'}
              refreshGameRunning={refreshGameRunning}
              onConfigChanged={setConfig}
            />
          ) : null}
        </main>
      </div>
    </div>
  );
}
