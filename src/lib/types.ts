export type DiscoverySource =
  | 'std'
  | 'flatpak'
  | 'prism_launcher'
  | 'multi_mc'
  | 'manual';

export interface GameDir {
  path: string;
  source: DiscoverySource;
}

export interface GameDirEntry {
  path: string;
  source: DiscoverySource;
  present: boolean;
  stored: boolean;
}

export interface GameDirState {
  config: Config;
  entries: GameDirEntry[];
}

export type Theme = 'system' | 'light' | 'dark';

export interface BackupsConfig {
  enabled: boolean;
  keep: number;
}

export interface Config {
  schema: number;
  game_dirs: GameDir[];
  active_game_dir: string | null;
  last_world: string | null;
  theme: Theme;
  backups: BackupsConfig;
}

export interface AppError {
  code: string;
  message: string;
}

export class IpcError extends Error implements AppError {
  code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'IpcError';
    this.code = code;
  }
}

export const CONFIG_SCHEMA = 1;

export function defaultConfig(): Config {
  return {
    schema: CONFIG_SCHEMA,
    game_dirs: [],
    active_game_dir: null,
    last_world: null,
    theme: 'system',
    backups: { enabled: true, keep: 10 },
  };
}
