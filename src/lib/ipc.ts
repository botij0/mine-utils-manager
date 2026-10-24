import { invoke } from '@tauri-apps/api/core';
import { mockClient } from './ipc.mock';
import { IpcError } from './types';
import type { Config, GameDirEntry } from './types';

export interface IpcClient {
  listGameDirs(): Promise<GameDirEntry[]>;
  addGameDir(path: string): Promise<GameDirEntry[]>;
  removeGameDir(path: string): Promise<GameDirEntry[]>;
  setActiveGameDir(path: string): Promise<Config>;
  getConfig(): Promise<Config>;
  saveConfig(config: Config): Promise<Config>;
}

export function isTauriRuntime(win: Window = window): boolean {
  return '__TAURI_INTERNALS__' in win;
}

export function toAppError(err: unknown): IpcError {
  if (typeof err === 'object' && err !== null && 'code' in err && 'message' in err) {
    const wire = err as { code: unknown; message: unknown };
    if (typeof wire.code === 'string' && typeof wire.message === 'string') {
      return new IpcError(wire.code, wire.message);
    }
  }
  if (err instanceof Error) return new IpcError('unknown', err.message);
  return new IpcError('unknown', String(err));
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toAppError(err);
  }
}

const tauriClient: IpcClient = {
  listGameDirs: () => call<GameDirEntry[]>('list_game_dirs'),
  addGameDir: (path) => call<GameDirEntry[]>('add_game_dir', { path }),
  removeGameDir: (path) => call<GameDirEntry[]>('remove_game_dir', { path }),
  setActiveGameDir: (path) => call<Config>('set_active_game_dir', { path }),
  getConfig: () => call<Config>('get_config'),
  saveConfig: (config) => call<Config>('save_config', { config }),
};

/**
 * The one client the UI talks to. It picks the Tauri invoke transport when the
 * runtime is present and the localStorage mock otherwise.
 */
export const ipc: IpcClient = {
  listGameDirs: () => pick().listGameDirs(),
  addGameDir: (path) => pick().addGameDir(path),
  removeGameDir: (path) => pick().removeGameDir(path),
  setActiveGameDir: (path) => pick().setActiveGameDir(path),
  getConfig: () => pick().getConfig(),
  saveConfig: (config) => pick().saveConfig(config),
};

function pick(): IpcClient {
  return isTauriRuntime() ? tauriClient : mockClient;
}
