import { useEffect, useState } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { BackupInfo, ResourceStack } from '../../lib/types';
import { makeFakeClient, type FakeClient, type FakeClientOptions } from '../../test/fake-ipc';
import { pack, resourceStack } from '../../test/fake-packs';
import BackupsPanel from '../settings/BackupsPanel';
import ActiveStack from './ActiveStack';
import PacksPage from './PacksPage';

const FILES = ['vanilla', 'fabric', 'file/Alpha.zip', 'file/Beta.zip'];
const PACKS = [pack('Alpha'), pack('Beta')];
const BACKUPS: BackupInfo[] = [
  { file_name: '20261025T090000.000000000-options.txt', bytes: 4066, display: '2026-10-25 09:00:00 UTC' },
];

function stackClient(options: FakeClientOptions = {}): FakeClient {
  return makeFakeClient({
    resource: [...FILES],
    packs: PACKS,
    backups: BACKUPS,
    ...options,
  });
}

function card(name: string): HTMLElement {
  return screen.getByRole('listitem', { name });
}

describe('ActiveStack', () => {
  it('refreshes enabled references after a replacement import', async () => {
    const client = stackClient();
    let replaced = false;
    client.getResourceStack = async () => resourceStack([replaced ? 'file/Beta.zip' : 'file/Alpha.zip']);
    client.importResourcePack = async () => {
      replaced = true;
      return { status: 'imported', pack: PACKS[1] };
    };
    render(<PacksPage client={client} />);
    const user = userEvent.setup();
    expect(await screen.findByRole('listitem', { name: 'Alpha' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    await user.type(screen.getByLabelText('Pack path'), '/tmp/Beta.zip');
    await user.click(screen.getByRole('button', { name: 'Import' }));
    expect(await screen.findByRole('listitem', { name: 'Beta' })).toBeInTheDocument();
    expect(screen.queryByRole('listitem', { name: 'Alpha' })).not.toBeInTheDocument();
  });

  it('shows the stack top-first with locked foreign entries', async () => {
    render(<PacksPage client={stackClient()} />);

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
    const client = stackClient();
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
    const client = stackClient();
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
    const client = stackClient({
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
    const client = stackClient({
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
    const client = stackClient({
      staleLocks: ['/home/demo/.minecraft/saves/world/session.lock'],
    });
    render(<PacksPage client={client} />);

    const note = await screen.findByRole('status', { name: 'Stale session lock' });
    expect(note).toHaveTextContent('stale session lock');
    expect(note).toHaveTextContent('Writes are allowed');
    expect(within(card('Alpha')).getByRole('checkbox')).toBeEnabled();
  });

  it('summarizes many stale session locks instead of printing every path', async () => {
    const staleLocks = Array.from(
      { length: 18 },
      (_, index) => `/home/demo/.minecraft/saves/world-${index}/session.lock`,
    );
    const client = stackClient({ staleLocks });
    render(<PacksPage client={client} />);

    const note = await screen.findByRole('status', { name: 'Stale session lock' });
    expect(note).toHaveTextContent(
      'A stale session lock at /home/demo/.minecraft/saves/world-0/session.lock',
    );
    expect(note).toHaveTextContent('17 more stale locks');
    expect(note).toHaveTextContent('Writes are allowed');
    expect(note).not.toHaveTextContent('world-17');
    expect(within(card('Alpha')).getByRole('checkbox')).toBeEnabled();
  });

  it('refreshes the stack after a backup restore', async () => {
    const user = userEvent.setup();
    const client = stackClient({ resource: ['vanilla', 'file/Beta.zip'] });
    client.restoredStack = resourceStack(['vanilla', 'fabric', 'file/Alpha.zip']);

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
              packs={[pack('Alpha')]}
              busy={false}
              running={stack.running}
              onToggle={() => {}}
              onReorder={() => {}}
              onRecheck={() => {}}
            />
          ) : null}
          <BackupsPanel client={client} gameRunning={null} onRestored={setStack} />
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
