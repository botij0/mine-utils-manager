import { afterEach, expect, it, vi } from 'vitest';

const transport = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: transport.invoke, convertFileSrc: (path: string) => path }));

import { ipc } from './ipc';

afterEach(() => {
  Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
  vi.resetAllMocks();
});

it('registers cancellation before either install or cancel reaches the backend', async () => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
  let registered = () => {};
  transport.invoke.mockImplementation((command: string) => {
    if (command === 'begin_modrinth_install') return new Promise<void>((resolve) => { registered = resolve; });
    if (command === 'cancel_modrinth_install') return Promise.resolve(true);
    if (command === 'install_modrinth_pack') return Promise.reject({ code: 'download_cancelled', message: 'Install cancelled.' });
    throw new Error(`unexpected command ${command}`);
  });
  const install = ipc.installModrinthPack({ installId: 'install-1', projectId: 'fresh', versionId: 'v1', filename: 'fresh.zip', kind: 'resourcepack' });
  const cancelled = expect(install).rejects.toMatchObject({ code: 'download_cancelled' });
  const cancel = ipc.cancelModrinthInstall('install-1');
  expect(transport.invoke.mock.calls).toEqual([['begin_modrinth_install', { installId: 'install-1' }]]);
  registered();
  await expect(cancel).resolves.toBe(true);
  await cancelled;
  expect(transport.invoke.mock.calls.map(([command]) => command)).toEqual(['begin_modrinth_install', 'install_modrinth_pack', 'cancel_modrinth_install']);
  expect(await ipc.cancelModrinthInstall('install-1')).toBe(false);
});
