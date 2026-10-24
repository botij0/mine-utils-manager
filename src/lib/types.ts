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

export type PackKind = 'resource' | 'data' | 'both' | 'unknown';

export type PackLocation = 'zip' | 'directory';

export interface FormatVersion {
  major: number;
  minor: number;
}

export interface FormatRange {
  /** Absent when the pack only declares a max_format. */
  min: FormatVersion | null;
  /** Absent when the pack only declares a min_format. */
  max: FormatVersion | null;
}

export interface PackMeta {
  pack_format: FormatVersion | null;
  min_format: FormatVersion | null;
  max_format: FormatVersion | null;
  supported_formats: FormatRange | null;
  description: string;
}

export interface PackIcon {
  sha256: string;
  url: string;
}

export interface PackIssue {
  code: string;
  message: string;
}

export interface Pack {
  id: string;
  name: string;
  path: string;
  location: PackLocation;
  kind: PackKind;
  description: string;
  meta: PackMeta;
  format: FormatVersion | null;
  format_range: FormatRange | null;
  game_label: string | null;
  icon: PackIcon | null;
  error: PackIssue | null;
  bytes_read: number;
}

export interface PackListing {
  packs: Pack[];
  count: number;
  bytes_read: number;
  elapsed_ms: number;
}

export type ImportChoice = 'default' | 'replace' | 'skip';

export type ImportOutcome =
  | { status: 'imported'; pack: Pack }
  | { status: 'duplicate'; name: string; sha256: string; existing: string }
  | { status: 'collision'; name: string }
  | { status: 'skipped' };
