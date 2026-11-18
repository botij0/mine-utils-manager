import type * as Wire from './generated/wire';

export type * from './generated/wire';

export type PackIcon = Omit<Wire.PackIcon, 'path'> & { url: string };
export type Pack = Omit<Wire.Pack, 'icon'> & { icon: PackIcon | null };
export type PackListing = Omit<Wire.PackListing, 'packs'> & { packs: Pack[] };
export type World = Omit<Wire.World, 'icon'> & { icon: PackIcon | null };
export type ImportRefusal = Exclude<Wire.ImportOutcome, { status: 'imported' }>;
export type ImportOutcome = { status: 'imported'; pack: Pack } | ImportRefusal;
export type DatapackListing = Omit<Wire.DatapackListing, 'packs'> & { packs: PackListing };
export type ModrinthInstallOutcome =
  | (Omit<Extract<Wire.ModrinthInstallOutcome, { kind: 'resourcepack' }>, 'outcome'> & { outcome: ImportOutcome })
  | Extract<Wire.ModrinthInstallOutcome, { kind: 'datapack' }>;
export type DataStackEntry = Wire.StackEntry;
export type StackGuard = Pick<Wire.GameGuard, 'running' | 'stale_locks' | 'detection_supported'>;
export type PackIssueCode = Wire.PackIssue['code'];

export type ClientErrorCode = Wire.ErrorCode | 'unknown' | 'settings_write_timeout';
export type ClientError = Omit<Wire.AppError, 'code'> & {
  code: ClientErrorCode;
  rawCode?: string;
};

export class SettingsWriteTimeoutError extends Error {
  readonly code = 'settings_write_timeout' as const;

  constructor() {
    super('Settings write timed out. It may still finish in the background.');
    this.name = 'SettingsWriteTimeoutError';
  }
}

export function isSettingsWriteTimeout(error: Pick<ClientError, 'code'>): boolean {
  return error.code === 'settings_write_timeout';
}

export class IpcError extends Error implements ClientError {
  constructor(
    readonly code: ClientErrorCode,
    message: string,
    readonly rawCode?: string,
  ) {
    super(message);
    this.name = 'IpcError';
  }
}

export const CONFIG_SCHEMA = 1;

export function defaultConfig(): Wire.Config {
  return {
    schema: CONFIG_SCHEMA,
    game_dirs: [],
    active_game_dir: null,
    last_world: null,
    theme: 'system',
    backups: { enabled: true, keep: 10 },
  };
}

export const MOCK_MODRINTH_PAGE_SIZE = 20;

export interface ModrinthInstallArgs {
  installId?: string;
  projectId: string;
  versionId: string;
  filename: string;
  kind: Wire.ModrinthKind;
  world?: string;
}
