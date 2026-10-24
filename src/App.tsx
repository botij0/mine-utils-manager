import { useEffect, useState } from 'react';
import { ipc } from './lib/ipc';
import type { IpcClient } from './lib/ipc';
import type { AppError, Config, Theme } from './lib/types';
import { defaultConfig } from './lib/types';
import { toAppError } from './lib/ipc';
import DiscoverPage from './features/discover/DiscoverPage';
import PacksPage from './features/packs/PacksPage';
import SettingsPage from './features/settings/SettingsPage';
import WorldsPage from './features/worlds/WorldsPage';

type PageId = 'packs' | 'worlds' | 'discover' | 'settings';

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

  useEffect(() => {
    window.location.hash = page;
  }, [page]);

  useEffect(() => {
    client
      .getConfig()
      .then(setConfig)
      .catch((err: unknown) => setError(toAppError(err)));
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
          {page === 'packs' ? <PacksPage /> : null}
          {page === 'worlds' ? <WorldsPage /> : null}
          {page === 'discover' ? <DiscoverPage /> : null}
          {page === 'settings' ? (
            <SettingsPage
              client={client}
              config={config}
              onConfigChanged={setConfig}
            />
          ) : null}
        </main>
      </div>
    </div>
  );
}
