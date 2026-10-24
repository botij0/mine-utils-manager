import { beforeEach, describe, expect, it } from 'vitest';
import { ipc, isTauriRuntime, toAppError } from './ipc';

beforeEach(() => {
  window.localStorage.clear();
});

describe('transport switch', () => {
  it('serves the mock dirs when the tauri runtime is absent', async () => {
    expect(isTauriRuntime(window)).toBe(false);
    const dirs = await ipc.listGameDirs();
    expect(dirs.map((dir) => dir.path)).toEqual([
      '/home/demo/.minecraft',
      '/home/demo/.var/app/com.mojang.Minecraft/data/minecraft',
      '/home/demo/.local/share/PrismLauncher/instances/1.21/.minecraft',
    ]);
    expect(dirs[0].source).toBe('std');
    expect(dirs[0].stored).toBe(false);
  });

  it('detects the tauri runtime from window internals', () => {
    const withTauri = { __TAURI_INTERNALS__: {} } as unknown as Window;
    const withoutTauri = {} as Window;
    expect(isTauriRuntime(withTauri)).toBe(true);
    expect(isTauriRuntime(withoutTauri)).toBe(false);
  });
});

describe('mock client behavior', () => {
  it('rejects an absolute path without a game-dir marker', async () => {
    await expect(ipc.addGameDir('/tmp/not-a-game-dir')).rejects.toMatchObject({
      code: 'invalid_game_dir',
    });
  });

  it('keeps added dirs when the theme changes', async () => {
    const added = await ipc.addGameDir('/tmp/saves/one');
    expect(added.config.game_dirs.map((dir) => dir.path)).toContain(
      '/tmp/saves/one',
    );

    const config = await ipc.setTheme('dark');

    expect(config.theme).toBe('dark');
    expect(config.game_dirs.map((dir) => dir.path)).toContain('/tmp/saves/one');
  });

  it('returns a null startup warning', async () => {
    expect(await ipc.getStartupWarning()).toBeNull();
  });
});

describe('typed error mapping', () => {
  it('keeps the code and message of a wire error', () => {
    const mapped = toAppError({
      code: 'invalid_game_dir',
      message: 'invalid game dir /bad: not a game dir',
    });
    expect(mapped.code).toBe('invalid_game_dir');
    expect(mapped.message).toBe('invalid game dir /bad: not a game dir');
    expect(mapped).toBeInstanceOf(Error);
  });

  it('maps a thrown string to the unknown code', () => {
    const mapped = toAppError('boom');
    expect(mapped.code).toBe('unknown');
    expect(mapped.message).toBe('boom');
  });

  it('maps a thrown error to the unknown code', () => {
    const mapped = toAppError(new Error('nope'));
    expect(mapped.code).toBe('unknown');
    expect(mapped.message).toBe('nope');
  });
});
