import type { ErrorCode, ModrinthInstallOutcome } from './generated/wire';

type Assert<T extends true> = T;
type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends
  (<T>() => T extends B ? 1 : 2) ? true : false;

type ResourceInstall = Extract<ModrinthInstallOutcome, { kind: 'resourcepack' }>;
type Imported = Extract<ResourceInstall['outcome'], { status: 'imported' }>;
export type InstallIconPath = Assert<Equal<NonNullable<Imported['pack']['icon']>['path'], string>>;
export type KnownErrorCode = Assert<'no_active_game_dir' extends ErrorCode ? true : false>;
export type RejectUnknownCode = Assert<Equal<string extends ErrorCode ? true : false, false>>;
