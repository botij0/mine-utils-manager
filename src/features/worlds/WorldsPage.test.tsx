import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { WorldsClient } from '../../lib/ipc';
import type { World } from '../../lib/types';
import WorldsPage from './WorldsPage';

function world(overrides: Partial<World> = {}): World {
  return {
    dir_name: 'Survival 1_17',
    path: '/home/demo/.minecraft/saves/Survival 1_17',
    level_name: 'Survival 1.17',
    version: { name: '1.21.10', data_format: { major: 88, minor: 0 } },
    icon: null,
    running: null,
    world_locked: null,
    stale_lock: false,
    detection_supported: true,
    issue: null,
    ...overrides,
  };
}

function baseClient(overrides: Partial<WorldsClient> = {}): WorldsClient {
  return {
    listWorlds: async () => [
      world(),
      world({
        dir_name: 'Creative Flat',
        path: '/home/demo/.minecraft/saves/Creative Flat',
        level_name: 'Creative Flat',
        version: { name: '1.21.10', data_format: { major: 88, minor: 0 } },
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
      const enabled =
        name === 'Survival 1_17'
          ? [
              { kind: 'foreign' as const, value: 'vanilla' },
              { kind: 'file' as const, value: 'file/a.zip', filename: 'a.zip' },
              { kind: 'foreign' as const, value: 'fabric' },
              { kind: 'foreign' as const, value: 'fabric-convention-tags-v2' },
            ]
          : [{ kind: 'foreign' as const, value: 'vanilla' }];
      return {
        packs: {
          packs: [
            {
              id: `/saves/${name}/datapacks/a.zip`,
              name: 'a',
              path: `/saves/${name}/datapacks/a.zip`,
              location: 'zip' as const,
              kind: 'data' as const,
              description: 'A datapack',
              meta: {
                pack_format: null,
                min_format: { major: 7, minor: 0 },
                max_format: { major: 7, minor: 0 },
                supported_formats: null,
                description: 'A datapack',
              },
              format: { major: 7, minor: 0 },
              format_range: { min: { major: 7, minor: 0 }, max: { major: 7, minor: 0 } },
              game_label: '1.17 to 1.17.1',
              icon: null,
              error: null,
              bytes_read: 64,
            },
          ],
          count: 1,
          bytes_read: 64,
          elapsed_ms: 1,
        },
        stack: {
          enabled,
          disabled: [],
          running: null,
          world_locked: null,
          stale_locks: [],
          detection_supported: true,
        },
      };
    },
    importDatapack: async () => ({ status: 'skipped' }),
    deleteDatapack: async () => ({
      removed_references: true,
      stack: {
        enabled: [],
        disabled: [],
        running: null,
        world_locked: null,
        stale_locks: [],
        detection_supported: true,
      },
    }),
    setDatapackEnabled: async () => ({
      enabled: [],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }),
    reorderDatapacks: async () => ({
      enabled: [],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }),
    listLevelBackups: async () => [],
    listAllLevelBackups: async () => [],
    removeDatapackReference: async () => ({
      enabled: [],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }),
    restoreLevelBackup: async () => ({
      enabled: [],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }),
    ...overrides,
  };
}

describe('WorldsPage', () => {
  it('lists worlds with icons and versions', async () => {
    render(<WorldsPage client={baseClient()} />);
    expect(await screen.findAllByText('Survival 1.17')).toHaveLength(2);
    expect(screen.getByText('Creative Flat')).toBeInTheDocument();
    expect(screen.getAllByText('1.21.10', { selector: '.world-rail-version' })).toHaveLength(2);
  });

  it('renders datapacks with a compatibility warning', async () => {
    render(<WorldsPage client={baseClient()} />);
    await screen.findAllByText('Survival 1.17');
    expect(await screen.findByText('May not load on this world')).toBeInTheDocument();
  });

  it('keeps foreign entries locked', async () => {
    render(<WorldsPage client={baseClient()} />);
    await screen.findAllByText('Survival 1.17');
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    const locks = within(grid).getAllByText('Locked');
    expect(locks.length).toBeGreaterThanOrEqual(3);
  });

  it('hides write controls for a corrupt level.dat', async () => {
    const user = userEvent.setup();
    render(<WorldsPage client={baseClient()} />);
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
    const lockedClient = baseClient({
      listWorlds: async () => [lockedWorld],
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: {
          enabled: [{ kind: 'foreign' as const, value: 'vanilla' }],
          disabled: [{ kind: 'file' as const, value: 'file/a.zip', filename: 'a.zip' }],
          running: null,
          world_locked: lockedWorld.world_locked,
          stale_locks: [],
          detection_supported: true,
        },
      }),
    });
    render(<WorldsPage client={lockedClient} />);
    expect(await screen.findByText(/is locked/)).toBeInTheDocument();
    const enable = await screen.findByRole('button', { name: 'Enable' });
    expect(enable).toBeDisabled();
  });

  it('toggles a datapack and refreshes the stack', async () => {
    const user = userEvent.setup();
    const setEnabled = vi.fn(async () => ({
      enabled: [{ kind: 'foreign' as const, value: 'vanilla' }],
      disabled: [{ kind: 'file' as const, value: 'file/a.zip', filename: 'a.zip' }],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }));
    render(<WorldsPage client={baseClient({ setDatapackEnabled: setEnabled })} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    await user.click(within(grid).getByRole('button', { name: 'Disable' }));
    await waitFor(() => expect(setEnabled).toHaveBeenCalledWith('Survival 1_17', 'a.zip', false));
  });

  it('keys duplicate entries by position so both cards render', async () => {
    const dupClient = baseClient({
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: {
          enabled: [
            { kind: 'file' as const, value: 'file/dup.zip', filename: 'dup.zip' },
            { kind: 'file' as const, value: 'file/dup.zip', filename: 'dup.zip' },
          ],
          disabled: [],
          running: null,
          world_locked: null,
          stale_locks: [],
          detection_supported: true,
        },
      }),
    });
    render(<WorldsPage client={dupClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getAllByRole('button', { name: 'dup.zip, enabled' })).toHaveLength(2);
  });

  it('moves by row index so a duplicate card swaps with its neighbor', async () => {
    const user = userEvent.setup();
    const reorder = vi.fn(async () => ({
      enabled: [],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }));
    const orderClient = baseClient({
      reorderDatapacks: reorder,
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: {
          enabled: [
            { kind: 'file' as const, value: 'file/a.zip', filename: 'a.zip' },
            { kind: 'file' as const, value: 'file/b.zip', filename: 'b.zip' },
          ],
          disabled: [],
          running: null,
          world_locked: null,
          stale_locks: [],
          detection_supported: true,
        },
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
    const setEnabled = vi.fn(async () => ({
      enabled: [{ kind: 'file' as const, value: 'file/hand.zip', filename: 'hand.zip' }],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }));
    const handClient = baseClient({
      setDatapackEnabled: setEnabled,
      listDatapacks: async () => ({
        packs: {
          packs: [
            {
              id: '/saves/Survival/hand.zip',
              name: 'hand',
              path: '/saves/Survival/datapacks/hand.zip',
              location: 'zip' as const,
              kind: 'data' as const,
              description: 'Hand-dropped',
              meta: {
                pack_format: null,
                min_format: null,
                max_format: null,
                supported_formats: null,
                description: 'Hand-dropped',
              },
              format: null,
              format_range: null,
              game_label: null,
              icon: null,
              error: null,
              bytes_read: 8,
            },
          ],
          count: 1,
          bytes_read: 8,
          elapsed_ms: 1,
        },
        stack: {
          enabled: [{ kind: 'foreign' as const, value: 'vanilla' }],
          disabled: [],
          running: null,
          world_locked: null,
          stale_locks: [],
          detection_supported: true,
        },
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
    const remove = vi.fn(async () => ({
      enabled: [{ kind: 'foreign' as const, value: 'vanilla' }],
      disabled: [],
      running: null,
      world_locked: null,
      stale_locks: [],
      detection_supported: true,
    }));
    const missingClient = baseClient({
      removeDatapackReference: remove,
      listDatapacks: async () => ({
        packs: { packs: [], count: 0, bytes_read: 0, elapsed_ms: 0 },
        stack: {
          enabled: [{ kind: 'foreign' as const, value: 'vanilla' }],
          disabled: [{ kind: 'file' as const, value: 'file/gone.zip', filename: 'gone.zip' }],
          running: null,
          world_locked: null,
          stale_locks: [],
          detection_supported: true,
        },
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
    const brokenClient = baseClient({
      listDatapacks: async () => ({
        packs: {
          packs: [
            {
              id: '/saves/Survival/broken.zip',
              name: 'broken',
              path: '/saves/Survival/datapacks/broken.zip',
              location: 'zip' as const,
              kind: 'data' as const,
              description: '',
              meta: {
                pack_format: null,
                min_format: null,
                max_format: null,
                supported_formats: null,
                description: '',
              },
              format: null,
              format_range: null,
              game_label: null,
              icon: null,
              error: { code: 'pack_broken_archive', message: 'broken.zip is not readable' },
              bytes_read: 8,
            },
          ],
          count: 1,
          bytes_read: 8,
          elapsed_ms: 1,
        },
        stack: {
          enabled: [{ kind: 'file' as const, value: 'file/broken.zip', filename: 'broken.zip' }],
          disabled: [],
          running: null,
          world_locked: null,
          stale_locks: [],
          detection_supported: true,
        },
      }),
    });
    render(<WorldsPage client={brokenClient} />);
    const grid = await screen.findByRole('list', { name: 'Datapack stack' });
    expect(within(grid).getByText('Broken datapack')).toBeInTheDocument();
    expect(within(grid).getByText('broken.zip is not readable')).toBeInTheDocument();
  });
});
