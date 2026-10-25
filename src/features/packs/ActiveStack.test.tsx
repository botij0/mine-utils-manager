import { useEffect, useState } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { IpcClient } from '../../lib/ipc';
import type {
  BackupInfo,
  DataStack,
  GameRunning,
  Pack,
  PackListing,
  ResourceStack,
  StackEntry,
} from '../../lib/types';
import BackupsPanel from '../settings/BackupsPanel';
import ActiveStack from './ActiveStack';
import PacksPage from './PacksPage';

const FILES = ['vanilla', 'fabric', 'file/Alpha.zip', 'file/Beta.zip'];

function entry(value: string): StackEntry {
  return value.startsWith('file/')
    ? { kind: 'file', value, filename: value.slice('file/'.length) }
    : { kind: 'foreign', value };
}

function makePack(name: string): Pack {
  return {
    id: `/home/demo/.minecraft/resourcepacks/${name}.zip`,
    name,
    path: `/home/demo/.minecraft/resourcepacks/${name}.zip`,
    location: 'zip',
    kind: 'resource',
    description: 'A synthetic pack',
    meta: {
      pack_format: { major: 34, minor: 0 },
      min_format: null,
      max_format: null,
      supported_formats: null,
      description: 'A synthetic pack',
    },
    format: { major: 34, minor: 0 },
    format_range: { min: { major: 34, minor: 0 }, max: { major: 42, minor: 0 } },
    game_label: '1.21 to 1.21.3',
    icon: null,
    error: null,
    bytes_read: 512,
  };
}

interface StackClient extends IpcClient {
  resource: string[];
  running: GameRunning | null;
  staleLocks: string[];
  toggles: Array<{ filename: string; enabled: boolean }>;
  reorders: string[][];
  restores: string[];
  backups: BackupInfo[];
  restoredStack: ResourceStack | null;
}

function makeStackClient({
  resource = [...FILES],
  running = null,
  staleLocks = [],
  packs = [makePack('Alpha'), makePack('Beta')],
  backups = [
    { file_name: '20261025T090000.000000000-options.txt', bytes: 4066, display: '2026-10-25 09:00:00 UTC' },
  ],
}: {
  resource?: string[];
  running?: GameRunning | null;
  staleLocks?: string[];
  packs?: Pack[];
  backups?: BackupInfo[];
} = {}): StackClient {
  const client: StackClient = {
    resource: [...resource],
    running,
    staleLocks,
    toggles: [],
    reorders: [],
    restores: [],
    backups,
    restoredStack: null,
    async listResourcePacks(): Promise<PackListing> {
      return {
        packs,
        count: packs.length,
        bytes_read: 0,
        elapsed_ms: 1,
      };
    },
    async importResourcePack(): Promise<never> {
      throw new Error('not used');
    },
    async deleteResourcePack() {},
    async getResourceStack(): Promise<ResourceStack> {
      return {
        entries: client.resource.map(entry),
        incompatible: [],
        running: client.running,
        stale_locks: client.staleLocks,
        detection_supported: true,
      };
    },
    async setResourcePackEnabled(filename: string, enabled: boolean) {
      client.toggles.push({ filename, enabled });
      const value = `file/${filename}`;
      client.resource = enabled
        ? [...client.resource.filter((item) => item !== value), value]
        : client.resource.filter((item) => item !== value);
      return client.getResourceStack();
    },
    async reorderResourceStack(order: string[]) {
      client.reorders.push(order);
      client.resource = [...order];
      return client.getResourceStack();
    },
    async listOptionsBackups(): Promise<BackupInfo[]> {
      return client.backups;
    },
    async restoreOptionsBackup(backup: string) {
      client.restores.push(backup);
      return client.restoredStack ?? client.getResourceStack();
    },
    async listGameDirs() {
      return [];
    },
    async addGameDir(): Promise<never> {
      throw new Error('not used');
    },
    async removeGameDir(): Promise<never> {
      throw new Error('not used');
    },
    async setActiveGameDir(): Promise<never> {
      throw new Error('not used');
    },
    async getConfig(): Promise<never> {
      throw new Error('not used');
    },
    async setTheme(): Promise<never> {
      throw new Error('not used');
    },
    async getStartupWarning() {
      return null;
    },
    async listWorlds() {
      return [];
    },
    async listDatapacks(): Promise<never> {
      throw new Error('not used');
    },
    async importDatapack(): Promise<never> {
      throw new Error('not used');
    },
    async deleteDatapack(): Promise<never> {
      throw new Error('not used');
    },
    async setDatapackEnabled(): Promise<DataStack> {
      throw new Error('not used');
    },
    async reorderDatapacks(): Promise<DataStack> {
      throw new Error('not used');
    },
    async listLevelBackups() {
      return [];
    },
    async restoreLevelBackup(): Promise<DataStack> {
      throw new Error('not used');
    },
  };
  return client;
}

function card(name: string): HTMLElement {
  return screen.getByRole('listitem', { name });
}

describe('ActiveStack', () => {
  it('shows the stack top-first with locked foreign entries', async () => {
    render(<PacksPage client={makeStackClient()} />);

    const list = await screen.findByRole('list', { name: 'Stack order' });
    const names = within(list)
      .getAllByRole('listitem')
      .map((item) => item.getAttribute('aria-label'));
    expect(names).toEqual(['Beta', 'Alpha', 'fabric', 'vanilla']);
    expect(screen.getByText(/Top card loads last and wins/)).toBeInTheDocument();

    for (const locked of ['fabric', 'vanilla']) {
      expect(within(card(locked)).queryByRole('checkbox')).not.toBeInTheDocument();
      expect(
        within(card(locked)).queryByRole('button', { name: `Drag ${locked}` }),
      ).not.toBeInTheDocument();
      expect(within(card(locked)).getByText('Locked')).toBeInTheDocument();
    }
  });

  it('toggles a pack off and offers undo, which toggles it back', async () => {
    const user = userEvent.setup();
    const client = makeStackClient();
    render(<PacksPage client={client} />);

    await screen.findByRole('list', { name: 'Stack order' });
    await user.click(within(card('Alpha')).getByRole('checkbox'));

    await waitFor(() =>
      expect(client.toggles).toEqual([{ filename: 'Alpha.zip', enabled: false }]),
    );
    const toast = await screen.findByRole('status', { name: 'Toggle result' });
    expect(toast).toHaveTextContent('Alpha.zip disabled');

    await user.click(within(toast).getByRole('button', { name: 'Undo' }));
    await waitFor(() =>
      expect(client.toggles).toEqual([
        { filename: 'Alpha.zip', enabled: false },
        { filename: 'Alpha.zip', enabled: true },
      ]),
    );
    await waitFor(() => expect(card('Alpha')).toBeInTheDocument());
  });

  it('reorders with the up and down buttons and sends the load order', async () => {
    const user = userEvent.setup();
    const client = makeStackClient();
    render(<PacksPage client={client} />);

    await screen.findByRole('list', { name: 'Stack order' });
    await user.click(
      within(card('Beta')).getByRole('button', { name: 'Move Beta down' }),
    );

    await waitFor(() =>
      expect(client.reorders).toEqual([
        ['vanilla', 'fabric', 'file/Beta.zip', 'file/Alpha.zip'],
      ]),
    );
    const list = screen.getByRole('list', { name: 'Stack order' });
    await waitFor(() =>
      expect(
        within(list)
          .getAllByRole('listitem')
          .map((item) => item.getAttribute('aria-label')),
      ).toEqual(['Alpha', 'Beta', 'fabric', 'vanilla']),
    );
  });

  it('removes a missing pack by filename', async () => {
    const user = userEvent.setup();
    const client = makeStackClient({
      resource: ['vanilla', 'file/Gone.zip'],
      packs: [],
    });
    render(<PacksPage client={client} />);

    const missing = await screen.findByRole('listitem', { name: 'Gone.zip' });
    expect(missing).toHaveTextContent('Gone.zip is not in resourcepacks');
    await user.click(within(missing).getByRole('button', { name: 'Remove' }));

    await waitFor(() =>
      expect(client.toggles).toEqual([{ filename: 'Gone.zip', enabled: false }]),
    );
  });

  it('blocks every write and names the lock while the game runs', async () => {
    const user = userEvent.setup();
    const client = makeStackClient({
      running: {
        kind: 'session_lock',
        detail: '/home/demo/.minecraft/saves/world/session.lock is locked by pid 4242',
        lock_path: '/home/demo/.minecraft/saves/world/session.lock',
        pid: 4242,
        command: null,
      },
    });
    render(<PacksPage client={client} />);

    const banner = await screen.findByRole('alert');
    expect(banner).toHaveTextContent('Minecraft is running');
    expect(banner).toHaveTextContent('session.lock is locked by pid 4242');

    expect(within(card('Alpha')).getByRole('checkbox')).toBeDisabled();
    expect(
      within(card('Beta')).getByRole('button', { name: 'Move Beta down' }),
    ).toBeDisabled();

    await user.click(screen.getByRole('button', { name: /^Alpha,/ }));
    expect(screen.getByRole('button', { name: 'Delete pack' })).toBeDisabled();

    await user.click(within(card('Alpha')).getByRole('checkbox').closest('label') as HTMLElement);
    expect(client.toggles).toEqual([]);
  });

  it('notes a stale session lock without blocking writes', async () => {
    const client = makeStackClient({
      staleLocks: ['/home/demo/.minecraft/saves/world/session.lock'],
    });
    render(<PacksPage client={client} />);

    const note = await screen.findByRole('status', { name: 'Stale session lock' });
    expect(note).toHaveTextContent('stale session lock');
    expect(note).toHaveTextContent('Writes are allowed');
    expect(within(card('Alpha')).getByRole('checkbox')).toBeEnabled();
  });

  it('refreshes the stack after a backup restore', async () => {
    const user = userEvent.setup();
    const client = makeStackClient({ resource: ['vanilla', 'file/Beta.zip'] });
    client.restoredStack = {
      entries: ['vanilla', 'fabric', 'file/Alpha.zip'].map(entry),
      incompatible: [],
      running: null,
      stale_locks: [],
      detection_supported: true,
    };

    function Harness() {
      const [stack, setStack] = useState<ResourceStack | null>(null);
      useEffect(() => {
        void client.getResourceStack().then(setStack);
      }, []);
      return (
        <div>
          {stack ? (
            <ActiveStack
              stack={stack}
              packs={[makePack('Alpha')]}
              busy={false}
              running={stack.running}
              onToggle={() => {}}
              onReorder={() => {}}
              onRecheck={() => {}}
            />
          ) : null}
          <BackupsPanel client={client} onRestored={setStack} />
        </div>
      );
    }

    render(<Harness />);

    await screen.findByRole('list', { name: 'Stack order' });
    expect(screen.queryByRole('listitem', { name: 'Alpha' })).not.toBeInTheDocument();

    await user.click(
      screen.getByRole('button', {
        name: 'Restore',
      }),
    );

    await waitFor(() =>
      expect(client.restores).toEqual(['20261025T090000.000000000-options.txt']),
    );
    expect(await screen.findByRole('listitem', { name: 'Alpha' })).toBeInTheDocument();
    expect(
      screen.getByText(/Restored 20261025T090000.000000000-options.txt/),
    ).toBeInTheDocument();
  });
});
