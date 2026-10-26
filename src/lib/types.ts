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

export type StackEntry =
  | { kind: 'file'; value: string; filename: string }
  | { kind: 'foreign'; value: string };

export type GameRunningKind = 'session_lock' | 'process';

export interface GameRunning {
  kind: GameRunningKind;
  detail: string;
  lock_path: string | null;
  pid: number | null;
  command: string | null;
}

/**
 * The enabled stack. `entries` is the `resourcePacks` array in load order,
 * low to high: the last entry wins. The UI shows the reverse, so the top card
 * is the highest priority and loads last. Foreign entries such as `vanilla`
 * and `fabric` are locked to their indices and never move.
 */
export interface ResourceStack {
  entries: StackEntry[];
  incompatible: string[];
  running: GameRunning | null;
  /** Stale `session.lock` files. They never block a write. */
  stale_locks: string[];
  /** False where POSIX lock probing is unsupported. */
  detection_supported: boolean;
}

/** What the UI rendered, so a write against a changed file set is refused. */
export interface StackSnapshot {
  resource: string[];
  incompatible: string[];
}

export interface BackupInfo {
  file_name: string;
  bytes: number;
  display: string;
}

export type DataStackEntry =
  | { kind: 'file'; value: string; filename: string }
  | { kind: 'foreign'; value: string };

/**
 * The datapack stack. `enabled` is the `Data.DataPacks.Enabled` array in load
 * order, low to high: the last entry wins. The UI shows the reverse, so the
 * top card is the highest priority. Foreign entries such as `vanilla` are
 * locked to their indices and never move.
 */
export interface DataStack {
  enabled: DataStackEntry[];
  disabled: DataStackEntry[];
  running: GameRunning | null;
  /** The lock on this world's own `session.lock`, when held. */
  world_locked: GameRunning | null;
  /** Stale `session.lock` files. They never block a write. */
  stale_locks: string[];
  /** False where POSIX lock probing is unsupported. */
  detection_supported: boolean;
}

/** What the UI rendered, so a write against a changed file set is refused. */
export interface DataSnapshot {
  enabled: string[];
  disabled: string[];
}

export interface WorldVersion {
  name: string;
  /** The data pack format the world's datapacks must declare, when known. */
  data_format: FormatVersion | null;
}

export interface WorldIssue {
  code: string;
  message: string;
}

export interface World {
  dir_name: string;
  path: string;
  level_name: string;
  version: WorldVersion | null;
  icon: PackIcon | null;
  running: GameRunning | null;
  world_locked: GameRunning | null;
  stale_lock: boolean;
  detection_supported: boolean;
  /** Set for corrupt, version-less, or datapack-less worlds: read-only. */
  issue: WorldIssue | null;
}

export interface DatapackListing {
  packs: PackListing;
  stack: DataStack;
}

export type DatapackImportOutcome =
  | { status: 'imported'; filename: string }
  | { status: 'locked'; filename: string; detail: string }
  | { status: 'duplicate'; name: string; sha256: string; existing: string }
  | { status: 'collision'; name: string }
  | { status: 'skipped' };

export interface DatapackDeleteOutcome {
  removed_references: boolean;
  stack: DataStack;
}

export interface LevelBackupInfo {
  file_name: string;
  world: string;
  bytes: number;
  display: string;
}

export type ModrinthKind = 'resourcepack' | 'datapack';

export type ModrinthSort = 'relevance' | 'downloads' | 'follows' | 'newest' | 'updated';

export interface ModrinthSearchHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  icon_url: string | null;
  downloads: number;
  follows: number;
  project_type: string | null;
  versions: string[];
  latest_version: string | null;
}

export interface ModrinthHashes {
  sha512: string;
  sha1: string;
}

export interface ModrinthFile {
  url: string;
  filename: string;
  size: number;
  primary: boolean;
  hashes: ModrinthHashes;
}

export interface ModrinthVersion {
  id: string;
  project_id: string;
  name: string;
  version_number: string;
  game_versions: string[];
  loaders: string[];
  files: ModrinthFile[];
}

export interface ModrinthProject {
  id: string;
  slug: string;
  title: string;
  description: string;
  body: string;
  icon_url: string | null;
  downloads: number;
  follows: number;
  project_type: string | null;
  game_versions: string[];
  versions: string[];
}

export interface ModrinthTargets {
  installed_versions: string[];
  preferred_launcher: string | null;
  world_version: string | null;
}

export interface ModrinthProvenanceEntry {
  project_id: string;
  version_id: string;
  file_name: string;
}

export interface ModrinthInstallArgs {
  projectId: string;
  versionId: string;
  filename: string;
  kind: ModrinthKind;
  /** Required for datapacks: the target world's dir name. */
  world?: string;
}

export type ModrinthInstallOutcome =
  | { kind: 'resourcepack'; outcome: ImportOutcome; provenance_warning?: string | null }
  | { kind: 'datapack'; outcome: DatapackImportOutcome; provenance_warning?: string | null };
