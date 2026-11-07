import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { WorldsClient } from '../../lib/ipc';
import { IpcError } from '../../lib/types';
import { worldsClient } from '../../test/fake-ipc';
import { datapack, stack, stackEntry, world } from '../../test/fake-packs';
import WorldsPage from './WorldsPage';

const BROKEN_DATAPACK = datapack('broken.zip', null, null, {
  id: '/saves/Survival/broken.zip',
  name: 'broken',
  path: '/saves/Survival/datapacks/broken.zip',
  description: '',
  meta: {
    pack_format: null,
    min_format: null,
    max_format: null,
    supported_formats: null,
    description: '',
  },
  error: { code: 'pack_broken_archive', message: 'broken.zip is not readable' },
  bytes_read: 8,
});

function pageClient(overrides: Partial<WorldsClient> = {}): WorldsClient {
  return worldsClient({
    listWorlds: async () => [
      world(),
      world({
        dir_name: 'Creative Flat',
        path: '/home/demo/.minecraft/saves/Creative Flat',
        level_name: 'Creative Flat',
      }),
      world({
        dir_name: 'Broken World',
        path: '/home/demo/.minecraft/saves/Broken World',
        level_name: 'Broken World',
        version: null,
        issue: { code: 'level_dat', message: 'level.dat is not valid gzip' },
      }),
    ],
    listDatapacks: async (name: string) => {
      if (name === 'Broken World') throw new Error('level.dat is not valid gzip');
      const enabled = (
        name === 'Survival 1_17'
          ? ['vanilla', 'file/a.zip', 'fabric', 'fabric-convention-tags-v2']
          : ['vanilla']
      ).map(stackEntry);
      return {
        packs: {
          packs: [
            datapack('a.zip', 7, 7, {
              id: `/saves/${name}/datapacks/a.zip`,
              name: 'a',
              path: `/saves/${name}/datapacks/a.zip`,
              bytes_read: 64,
            }),
          ],
          count: 1,
          bytes_read: 64,
          elapsed_ms: 1,
        },
        stack: stack({ enabled, disabled: [] }),
      };
    },
    ...overrides,
  });
}

describe('WorldsPage', () => {
  it('lists worlds with icons and versions', async () => {
    render(<WorldsPage client={pageClient()} />);
    expect(await screen.findAllByText('Survival 1.17')).toHaveLength(2);
    expect(screen.getByText('Creative Flat')).toBeInTheDocument();
    expect(screen.getAllByText('1.21.10', { selector: '.world-rail-version' })).toHaveLength(2);
  });

  it('says so when a world is newer than the format table instead of passing it as compatible', async () => {
    const client = pageClient({
      listWorlds: async () => [
        world({
          version: { name: '26.9', data_format: null, newest_known_version: '26.3' },
        }),
      ],
    });
    render(<WorldsPage client={client} />);
    expect(
      await screen.findByText(
        /Minecraft 26\.9 is newer than this build knows \(newest known: 26\.3\)\. Datapack compatibility is not checked\./,
      ),
    ).toBeInTheDocument();
  });

  it('says so when a world version is not in the format table at all', async () => {
    const client = pageClient({
      listWorlds: async () => [
        world({ version: { name: '1.2.3-odd', data_format: null, newest_known_version: null } }),
      ],
    });
    render(<WorldsPage client={client} />);
    expect(
      await screen.findByText(
        /Mine Utils does not know the data format of Minecraft 1\.2\.3-odd\. Datapack compatibility is not checked\./,
      ),
    ).toBeInTheDocument();
  });

  it('renders datapacks with a compatibility warning', async () => {
    render(<WorldsPage client={pageClient()} />);
    await screen.findAllByText('Survival 1.17');
    expect(await screen.findByText('May not load on this world')).toBeInTheDocument();
  });

  it('keeps foreign entries locked', async () => {
    render(<WorldsPage client={pageClient()} />);
    await screen.findAllByText('Survival 1.17');
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    const locks = within(grid).getAllByText('Locked');
    expect(locks.length).toBeGreaterThanOrEqual(3);
  });

  it('hides write controls for a corrupt level.dat', async () => {
    const user = userEvent.setup();
    render(<WorldsPage client={pageClient()} />);
    await user.click(await screen.findByRole('button', { name: /Broken World/ }));
    expect(await screen.findByText(/This world is read-only/)).toBeInTheDocument();
    expect(screen.queryByRole('list', { name: 'Datapack stack' })).not.toBeInTheDocument();
  });

  it('shows the locked banner and disables toggles', async () => {
    const lockedWorld = world({
      world_locked: {
        kind: 'session_lock',
        detail: 'saves/Survival 1_17/session.lock is locked by pid 7',
        lock_path: 'session.lock',
        pid: 7,
        command: null,
      },
    });
    const lockedClient = pageClient({
      listWorlds: async () => [lockedWorld],
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: stack({
          enabled: [{ kind: 'foreign', value: 'vanilla' }],
          disabled: [{ kind: 'file', value: 'file/a.zip', filename: 'a.zip' }],
          world_locked: lockedWorld.world_locked,
        }),
      }),
    });
    render(<WorldsPage client={lockedClient} />);
    expect(await screen.findByText(/is locked/)).toBeInTheDocument();
    const enable = await screen.findByRole('button', { name: 'Enable' });
    expect(enable).toBeDisabled();
  });

  it('toggles a datapack and refreshes the stack', async () => {
    const user = userEvent.setup();
    const setEnabled = vi.fn(async () =>
      stack({
        enabled: [{ kind: 'foreign', value: 'vanilla' }],
        disabled: [{ kind: 'file', value: 'file/a.zip', filename: 'a.zip' }],
      }),
    );
    render(<WorldsPage client={pageClient({ setDatapackEnabled: setEnabled })} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    await user.click(within(grid).getByRole('button', { name: 'Disable' }));
    await waitFor(() => expect(setEnabled).toHaveBeenCalledWith('Survival 1_17', 'a.zip', false));
  });

  it('keys duplicate entries by position so both cards render', async () => {
    const dupClient = pageClient({
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: stack({
          enabled: [
            { kind: 'file', value: 'file/dup.zip', filename: 'dup.zip' },
            { kind: 'file', value: 'file/dup.zip', filename: 'dup.zip' },
          ],
          disabled: [],
        }),
      }),
    });
    render(<WorldsPage client={dupClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getAllByRole('button', { name: 'dup.zip, enabled' })).toHaveLength(2);
  });

  it('moves by row index so a duplicate card swaps with its neighbor', async () => {
    const user = userEvent.setup();
    const reorder = vi.fn(async () => stack({ enabled: [], disabled: [] }));
    const orderClient = pageClient({
      reorderDatapacks: reorder,
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: stack({
          enabled: [
            { kind: 'file', value: 'file/a.zip', filename: 'a.zip' },
            { kind: 'file', value: 'file/b.zip', filename: 'b.zip' },
          ],
          disabled: [],
        }),
      }),
    });
    render(<WorldsPage client={orderClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    // Top-first display: b.zip is first, so "Move b.zip down" swaps it below a.zip.
    await user.click(within(grid).getByRole('button', { name: 'Move b.zip down' }));
    await waitFor(() =>
      expect(reorder).toHaveBeenCalledWith(
        'Survival 1_17',
        ['file/b.zip', 'file/a.zip'],
        expect.objectContaining({ enabled: ['file/a.zip', 'file/b.zip'] }),
      ),
    );
  });

  it('shows an installed-but-unlisted pack with an Enable action', async () => {
    const user = userEvent.setup();
    const setEnabled = vi.fn(async () =>
      stack({
        enabled: [{ kind: 'file', value: 'file/hand.zip', filename: 'hand.zip' }],
        disabled: [],
      }),
    );
    const handClient = pageClient({
      setDatapackEnabled: setEnabled,
      listDatapacks: async () => ({
        packs: {
          packs: [
            datapack('hand.zip', null, null, {
              id: '/saves/Survival/hand.zip',
              name: 'hand',
              path: '/saves/Survival/datapacks/hand.zip',
              description: 'Hand-dropped',
              meta: {
                pack_format: null,
                min_format: null,
                max_format: null,
                supported_formats: null,
                description: 'Hand-dropped',
              },
              bytes_read: 8,
            }),
          ],
          count: 1,
          bytes_read: 8,
          elapsed_ms: 1,
        },
        stack: stack({ enabled: [{ kind: 'foreign', value: 'vanilla' }], disabled: [] }),
      }),
    });
    render(<WorldsPage client={handClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByRole('button', { name: /hand\.zip/ })).toBeInTheDocument();
    await user.click(within(grid).getByRole('button', { name: 'Enable' }));
    await waitFor(() => expect(setEnabled).toHaveBeenCalledWith('Survival 1_17', 'hand.zip', true));
  });

  it('disables Enable for a missing file and offers to remove the reference', async () => {
    const user = userEvent.setup();
    const remove = vi.fn(async () =>
      stack({
        enabled: [{ kind: 'foreign', value: 'vanilla' }],
        disabled: [],
      }),
    );
    const missingClient = pageClient({
      removeDatapackReference: remove,
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: stack({
          enabled: [{ kind: 'foreign', value: 'vanilla' }],
          disabled: [{ kind: 'file', value: 'file/gone.zip', filename: 'gone.zip' }],
        }),
      }),
    });
    render(<WorldsPage client={missingClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByText('File missing from datapacks')).toBeInTheDocument();
    expect(within(grid).getByRole('button', { name: 'Enable' })).toBeDisabled();
    await user.click(within(grid).getByRole('button', { name: /gone\.zip/ }));
    await user.click(await screen.findByRole('button', { name: 'Remove reference' }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith('Survival 1_17', 'gone.zip'));
  });

  it('renders a broken datapack as an error card', async () => {
    const brokenClient = pageClient({
      listDatapacks: async () => ({
        packs: {
          packs: [BROKEN_DATAPACK],
          count: 1,
          bytes_read: 8,
          elapsed_ms: 1,
        },
        stack: stack({
          enabled: [{ kind: 'file', value: 'file/broken.zip', filename: 'broken.zip' }],
          disabled: [],
        }),
      }),
    });
    render(<WorldsPage client={brokenClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByText('Broken datapack')).toBeInTheDocument();
    expect(within(grid).getByText('broken.zip is not readable')).toBeInTheDocument();
  });

  it('keeps Disable on an enabled broken pack', async () => {
    const user = userEvent.setup();
    const setEnabled = vi.fn(async () =>
      stack({
        enabled: [{ kind: 'foreign', value: 'vanilla' }],
        disabled: [{ kind: 'file', value: 'file/broken.zip', filename: 'broken.zip' }],
      }),
    );
    const actionable = pageClient({
      setDatapackEnabled: setEnabled,
      listDatapacks: async () => ({
        packs: { packs: [BROKEN_DATAPACK], count: 1, bytes_read: 8, elapsed_ms: 1 },
        stack: stack({
          enabled: [{ kind: 'file', value: 'file/broken.zip', filename: 'broken.zip' }],
          disabled: [],
        }),
      }),
    });
    render(<WorldsPage client={actionable} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    // An enabled corrupt pack is still a live entry: it keeps Disable.
    expect(within(grid).getByRole('button', { name: 'Disable' })).toBeEnabled();
    // The detail offers the same Disable for the selected broken pack.
    await user.click(within(grid).getByRole('button', { name: 'broken.zip, broken' }));
    const detail = await screen.findByRole('region', { name: 'Datapack details' });
    expect(within(detail).getByRole('button', { name: 'Disable' })).toBeEnabled();
    await user.click(within(grid).getByRole('button', { name: 'Disable' }));
    await waitFor(() =>
      expect(setEnabled).toHaveBeenCalledWith('Survival 1_17', 'broken.zip', false),
    );
  });

  it('leaves a disabled broken pack as an error badge with no actions', async () => {
    const badgeClient = pageClient({
      listDatapacks: async () => ({
        packs: { packs: [BROKEN_DATAPACK], count: 1, bytes_read: 8, elapsed_ms: 1 },
        stack: stack({
          enabled: [{ kind: 'foreign', value: 'vanilla' }],
          disabled: [{ kind: 'file', value: 'file/broken.zip', filename: 'broken.zip' }],
        }),
      }),
    });
    render(<WorldsPage client={badgeClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByText('Broken datapack')).toBeInTheDocument();
    expect(within(grid).queryByRole('button', { name: 'Enable' })).not.toBeInTheDocument();
    expect(within(grid).queryByRole('button', { name: 'Disable' })).not.toBeInTheDocument();
  });

  it('explains the removal step for a symlinked pack', async () => {
    const user = userEvent.setup();
    const linkClient = pageClient({
      listDatapacks: async () => ({
        packs: {
          packs: [
            datapack('linked.zip', null, null, {
              id: '/saves/Survival/linked.zip',
              name: 'linked',
              path: '/saves/Survival/datapacks/linked.zip',
              description: '',
              meta: {
                pack_format: null,
                min_format: null,
                max_format: null,
                supported_formats: null,
                description: '',
              },
              error: { code: 'pack_symlink', message: 'linked.zip is a symlink and is never followed' },
              bytes_read: 8,
            }),
          ],
          count: 1,
          bytes_read: 8,
          elapsed_ms: 1,
        },
        stack: stack({
          enabled: [{ kind: 'foreign', value: 'vanilla' }],
          disabled: [{ kind: 'file', value: 'file/linked.zip', filename: 'linked.zip' }],
        }),
      }),
    });
    render(<WorldsPage client={linkClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    await user.click(within(grid).getByRole('button', { name: 'linked.zip, broken' }));
    expect(await screen.findByText(/removes only the link/)).toBeInTheDocument();
  });

  it('hides the stale-lock note while writes are paused', async () => {
    const lockedWorld = world({
      world_locked: {
        kind: 'session_lock',
        detail: 'saves/Survival 1_17/session.lock is locked by pid 7',
        lock_path: 'session.lock',
        pid: 7,
        command: null,
      },
    });
    const staleWhileLocked = pageClient({
      listWorlds: async () => [lockedWorld],
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: stack({
          enabled: [{ kind: 'foreign', value: 'vanilla' }],
          disabled: [],
          world_locked: lockedWorld.world_locked,
          stale_locks: ['/home/demo/.minecraft/saves/Survival 1_17/session.lock'],
        }),
      }),
    });
    render(<WorldsPage client={staleWhileLocked} />);
    expect(await screen.findByText(/is locked/)).toBeInTheDocument();
    // The stale note claims writes are allowed; it must not show while the
    // world lock pauses every write.
    expect(screen.queryByText(/Writes are allowed/)).not.toBeInTheDocument();
  });

  it('offers a Retry that refreshes after a stale stack error', async () => {
    const user = userEvent.setup();
    const inner = pageClient();
    const listSpy = vi.fn((name: string) => inner.listDatapacks(name));
    const staleClient = pageClient({
      listDatapacks: listSpy,
      setDatapackEnabled: async () => {
        throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
      },
    });
    render(<WorldsPage client={staleClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    await user.click(within(grid).getByRole('button', { name: 'Disable' }));
    expect(await screen.findByText(/the stack changed on disk/)).toBeInTheDocument();
    const callsBefore = listSpy.mock.calls.length;
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(listSpy.mock.calls.length).toBeGreaterThan(callsBefore));
    expect(await screen.findByRole('list', { name: 'Datapack stack' })).toBeInTheDocument();
  });

  it('shows the new loading state with no stale cards after a world switch', async () => {
    const user = userEvent.setup();
    const inner = pageClient();
    let resolveCreative!: (value: Awaited<ReturnType<WorldsClient['listDatapacks']>>) => void;
    const creativeListing = new Promise<Awaited<ReturnType<WorldsClient['listDatapacks']>>>(
      (resolve) => {
        resolveCreative = resolve;
      },
    );
    const switching = pageClient({
      listDatapacks: (name: string) =>
        name === 'Creative Flat' ? creativeListing : inner.listDatapacks(name),
    });
    render(<WorldsPage client={switching} />);
    // The old world's card is actionable before the switch.
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByRole('button', { name: 'a.zip, enabled' })).toBeInTheDocument();
    await user.click(await screen.findByRole('button', { name: /Creative Flat/ }));
    // The switch drops the old rows synchronously: no stale filename can be
    // clicked while the new world's listing is in flight.
    await waitFor(() => expect(screen.getByText('Loading datapacks…')).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'a.zip, enabled' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Disable' })).not.toBeInTheDocument();
    await act(async () => {
      resolveCreative({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: stack({ enabled: [], disabled: [] }),
      });
      await creativeListing;
    });
    await waitFor(() =>
      expect(screen.getByText('No datapacks yet. Import a zip or a folder to add one.')).toBeInTheDocument(),
    );
  });
});
