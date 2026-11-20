import { useState } from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { PackClient } from '../../lib/ipc';
import type { GameRunning } from '../../lib/types';
import { IpcError } from '../../lib/types';
import { makeFakeClient, type FakeClient } from '../../test/fake-ipc';
import {
  AUTUMN_PACK,
  BROKEN_PACK,
  SYMLINK_PACK,
  VANILLA_PACK,
  resourceStack,
} from '../../test/fake-packs';
import PacksPage from './PacksPage';

function renderPage(client: FakeClient) {
  return render(<PacksPage client={client} gameRunning={null} />);
}

/** The app guard answers `answer()` each time the page asks for a recheck. */
function GuardedPage({ client, answer }: { client: FakeClient; answer: () => GameRunning | null }) {
  const [gameRunning, setGameRunning] = useState<GameRunning | null>(null);
  return (
    <PacksPage client={client} gameRunning={gameRunning} refreshGameRunning={() => setGameRunning(answer())} />
  );
}

describe('PacksPage', () => {
  it.each(['running', 'unknown'] as const)('disables import while the guard is %s', async (guard) => {
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    const running: GameRunning = {
      kind: 'process', detail: 'java', lock_path: null, pid: 42, command: 'java',
    };
    const { rerender } = render(<PacksPage client={client} gameRunning={null} />);
    await screen.findByText('Vanilla');
    expect(screen.getByRole('button', { name: 'Import pack' })).toBeEnabled();
    await userEvent.setup().click(screen.getByRole('button', { name: 'Import pack' }));
    await userEvent.setup().type(screen.getByLabelText('Pack path'), '/tmp/Fresh.zip');
    rerender(<PacksPage client={client} gameRunning={guard === 'running' ? running : null} gameGuardUnknown={guard === 'unknown'} />);
    expect(screen.getByRole('button', { name: 'Import pack' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Import' })).toBeDisabled();
  });

  it('refreshes packs and the stack after an import commits and then rejects', async () => {
    const user = userEvent.setup();
    const base = makeFakeClient({ packs: [VANILLA_PACK] });
    let committed = false;
    const after = makeFakeClient({ packs: [VANILLA_PACK, AUTUMN_PACK], resource: ['vanilla', 'file/Autumn.zip'] });
    const client: PackClient = {
      ...base,
      async listResourcePacks() { return (committed ? after : base).listResourcePacks(); },
      async getResourceStack() { return (committed ? after : base).getResourceStack(); },
      async importResourcePack() {
        committed = true;
        throw new IpcError('io', 'New pack installed; original pack retained.');
      },
    };
    render(<PacksPage client={client} gameRunning={null} />);
    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    await user.type(screen.getByLabelText('Pack path'), '/tmp/Autumn.zip');
    await user.click(screen.getByRole('button', { name: 'Import' }));
    expect(await screen.findByText('New pack installed; original pack retained.')).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: /^Autumn,/ })).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: 'Enabled: Autumn' })).toBeChecked();
  });

  it.each(['default', 'replace'] as const)('refreshes the game guard after %s import is refused', async (choice) => {
    const user = userEvent.setup();
    const base = makeFakeClient({ packs: [VANILLA_PACK] });
    const running: GameRunning = { kind: 'process', detail: 'java', lock_path: null, pid: 42, command: 'java' };
    let refused = false;
    const client: PackClient = {
      ...base,
      async getResourceStack() { return { ...base.stack, running: refused ? running : null }; },
      async importResourcePack() {
        if (choice === 'replace' && !refused) {
          refused = true;
          return { status: 'collision', name: 'Fresh' };
        }
        refused = true;
        throw new IpcError('game_running', 'Minecraft started during import.');
      },
    };
    function Page() {
      const [gameRunning, setRunning] = useState<GameRunning | null>(null);
      return <PacksPage client={client} gameRunning={gameRunning} refreshGameRunning={() => setRunning(running)} />;
    }
    render(<Page />);
    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    await user.type(screen.getByLabelText('Pack path'), '/tmp/Fresh.zip');
    await user.click(screen.getByRole('button', { name: 'Import' }));
    if (choice === 'replace') await user.click(await screen.findByRole('button', { name: 'Replace' }));
    expect(await screen.findByText('Minecraft started during import.')).toBeInTheDocument();
    expect(await screen.findByText('Minecraft is running.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Import pack' })).toBeDisabled();
  });

  it('lists packs with descriptions, chips, icons, and an isolated error card', async () => {
    const icon = { sha256: 'icon', url: 'data:image/png;base64,abc' };
    const client = makeFakeClient({ packs: [{ ...VANILLA_PACK, icon }, AUTUMN_PACK, BROKEN_PACK] });
    renderPage(client);

    expect(await screen.findByText('Vanilla')).toBeInTheDocument();
    expect(screen.getByText('Small UI tweaks')).toBeInTheDocument();
    expect(screen.getByText('Seasonal recipes for a server')).toBeInTheDocument();
    expect(screen.getByText('Cannot read this archive')).toBeInTheDocument();
    expect(screen.getByText('Error')).toBeInTheDocument();
    expect(screen.getByAltText('')).toHaveAttribute('src', icon.url);
    expect(screen.getByText(/3 packs/)).toBeInTheDocument();
  });

  it('filters by search and clears filters from the empty state', async () => {
    const user = userEvent.setup();
    renderPage(makeFakeClient({ packs: [VANILLA_PACK, AUTUMN_PACK] }));

    await screen.findByText('Vanilla');
    await user.type(screen.getByLabelText('Search packs'), 'autumn');
    expect(screen.getByText('Autumn')).toBeInTheDocument();
    expect(screen.queryByText('Vanilla')).not.toBeInTheDocument();

    await user.clear(screen.getByLabelText('Search packs'));
    await user.type(screen.getByLabelText('Search packs'), 'missing');
    expect(await screen.findByText('No packs match those filters')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(await screen.findByText('Vanilla')).toBeInTheDocument();
  });

  it('filters by kind and by format', async () => {
    const user = userEvent.setup();
    renderPage(makeFakeClient({ packs: [VANILLA_PACK, AUTUMN_PACK] }));

    await screen.findByText('Vanilla');
    await user.selectOptions(screen.getByLabelText('Kind'), 'data');
    expect(screen.queryByText('Vanilla')).not.toBeInTheDocument();
    expect(screen.getByText('Autumn')).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText('Kind'), 'all');
    await user.selectOptions(screen.getByLabelText('Format'), '48 to 57');
    expect(screen.queryByText('Vanilla')).not.toBeInTheDocument();
    expect(screen.getByText('Autumn')).toBeInTheDocument();
  });

  it('imports a pack, adds a card, and opens the detail panel', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    renderPage(client);

    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    const dialog = screen.getByRole('dialog', { name: 'Import pack' });
    await user.type(
      within(dialog).getByLabelText('Pack path'),
      '/tmp/packs/Fresh.zip',
    );
    await user.click(within(dialog).getByRole('button', { name: 'Import' }));

    expect(await screen.findByRole('heading', { name: 'Fresh' })).toBeInTheDocument();
    expect(screen.getByText('/home/demo/.minecraft/resourcepacks/Fresh.zip')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('offers Skip and Replace on a duplicate and skip leaves one entry', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    renderPage(client);

    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    await user.type(
      within(screen.getByRole('dialog')).getByLabelText('Pack path'),
      '/tmp/packs/duplicate.zip',
    );
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Import' }));

    const prompt = await screen.findByRole('dialog', {
      name: 'Duplicate pack',
    });
    expect(within(prompt).getAllByRole('button')).toHaveLength(2);
    await user.click(within(prompt).getByRole('button', { name: 'Skip' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(client.choices).toContain('skip');
    expect(client.packs).toHaveLength(1);
    expect(screen.getAllByText('Vanilla')).toHaveLength(1);
  });

  it('replaces on a duplicate prompt', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    renderPage(client);

    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    await user.type(
      within(screen.getByRole('dialog')).getByLabelText('Pack path'),
      '/tmp/packs/duplicate.zip',
    );
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Import' }));
    const prompt = await screen.findByRole('dialog', { name: 'Duplicate pack' });
    await user.click(within(prompt).getByRole('button', { name: 'Replace' }));

    expect(await screen.findByRole('button', { name: /^duplicate,/ })).toBeInTheDocument();
    expect(client.choices).toContain('replace');
  });

  it('deletes a pack after confirmation and removes the card', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK, AUTUMN_PACK] });
    renderPage(client);

    await user.click(await screen.findByRole('button', { name: /^Autumn,/ }));
    const detail = screen.getByRole('complementary', { name: 'Pack details' });
    await user.click(within(detail).getByRole('button', { name: 'Delete pack' }));
    await user.click(within(detail).getByRole('button', { name: 'Move to trash' }));

    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /^Autumn,/ })).not.toBeInTheDocument(),
    );
    expect(screen.getByText('Vanilla')).toBeInTheDocument();
    expect(client.packs).toHaveLength(1);
  });

  it('refreshes the banner and disabled controls after a game_running delete', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    const running: GameRunning = {
      kind: 'process',
      detail: 'pid 4242 runs java',
      lock_path: null,
      pid: 4242,
      command: 'java -jar client.jar',
    };
    let nowRunning: GameRunning | null = null;
    client.getResourceStack = async () => ({
      entries: [{ kind: 'file', value: 'file/Vanilla.zip', filename: 'Vanilla.zip' }],
      incompatible: [],
      running: nowRunning,
      stale_locks: [],
      detection_supported: true,
    });
    client.deleteResourcePack = async () => {
      nowRunning = running;
      throw new IpcError('game_running', 'Minecraft is running; close it first');
    };
    render(<GuardedPage client={client} answer={() => nowRunning} />);

    await user.click(await screen.findByRole('button', { name: /^Vanilla,/ }));
    const detail = screen.getByRole('complementary', { name: 'Pack details' });
    await user.click(within(detail).getByRole('button', { name: 'Delete pack' }));
    await user.click(within(detail).getByRole('button', { name: 'Move to trash' }));

    expect(await screen.findByText('Minecraft is running.')).toBeInTheDocument();
    const list = screen.getByRole('list', { name: 'Stack order' });
    expect(within(list).getByRole('checkbox')).toBeDisabled();
    expect(within(detail).getByRole('button', { name: 'Delete pack' })).toBeDisabled();
  });

  it('offers no delete for a symlink pack and says how to remove it', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [SYMLINK_PACK] });
    renderPage(client);

    await user.click(await screen.findByRole('button', { name: /^Linked,/ }));
    const detail = screen.getByRole('complementary', { name: 'Pack details' });
    expect(
      within(detail).queryByRole('button', { name: 'Delete pack' }),
    ).not.toBeInTheDocument();
    expect(
      within(detail).getByText(/Remove the link from resourcepacks in a file manager/),
    ).toBeInTheDocument();
  });

  it('shows an import failure inside the import sheet', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    renderPage(client);

    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    const dialog = screen.getByRole('dialog', { name: 'Import pack' });
    await user.type(
      within(dialog).getByLabelText('Pack path'),
      '/tmp/packs/broken.zip',
    );
    await user.click(within(dialog).getByRole('button', { name: 'Import' }));

    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveTextContent(
      'broken.zip is not a readable pack: invalid Zip archive',
    );
    expect(screen.getByRole('dialog', { name: 'Import pack' })).toBeInTheDocument();
  });

  it('opens the sheet on the first dropped path and reports the rest', async () => {
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    const { container } = renderPage(client);
    await screen.findByText('Vanilla');

    const body = container.querySelector('.packs-body');
    fireEvent.drop(body as Element, {
      dataTransfer: {
        files: [
          { path: '/drop/One.zip' },
          { path: '/drop/Two.zip' },
          { path: '/drop/Three.zip' },
        ],
      },
    });

    const dialog = await screen.findByRole('dialog', { name: 'Import pack' });
    expect(within(dialog).getByLabelText('Pack path')).toHaveValue('/drop/One.zip');
    expect(within(dialog).getByRole('status')).toHaveTextContent(
      'Also dropped: /drop/Two.zip, /drop/Three.zip. Import them one at a time.',
    );
    expect(client.imports).toEqual([]);
  });

  it('imports a folder picked with the folder picker', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    const picked: boolean[] = [];
    const pickPath = async (directory: boolean) => {
      picked.push(directory);
      return directory ? '/drop/PackFolder' : '/drop/Pack.zip';
    };
    render(<PacksPage client={client} pickPath={pickPath} gameRunning={null} />);

    await screen.findByText('Vanilla');
    await user.click(screen.getByRole('button', { name: 'Import pack' }));
    const dialog = screen.getByRole('dialog', { name: 'Import pack' });
    await user.click(within(dialog).getByRole('button', { name: 'Browse file' }));
    expect(within(dialog).getByLabelText('Pack path')).toHaveValue('/drop/Pack.zip');
    await user.click(within(dialog).getByRole('button', { name: 'Browse folder' }));
    expect(within(dialog).getByLabelText('Pack path')).toHaveValue('/drop/PackFolder');
    await user.click(within(dialog).getByRole('button', { name: 'Import' }));

    expect(await screen.findByRole('heading', { name: 'PackFolder' })).toBeInTheDocument();
    expect(picked).toEqual([false, true]);
    expect(client.imports).toEqual(['/drop/PackFolder']);
  });

  it('renders a distinct stack error with the launch hint and a retry action', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    let failing = true;
    client.getResourceStack = async () => {
      if (failing) {
        throw new IpcError(
          'options_txt',
          'options.txt is missing at /games/one/options.txt; launch Minecraft once so it writes the file, then try again',
        );
      }
      return {
        entries: [{ kind: 'foreign', value: 'vanilla' }],
        incompatible: [],
        running: null,
        stale_locks: [],
        detection_supported: true,
      };
    };
    renderPage(client);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/options.txt is missing/);
    expect(alert).toHaveTextContent(/launch Minecraft once/i);
    expect(
      screen.queryByText('Launch Minecraft once so it writes options.txt, then retry.'),
    ).not.toBeInTheDocument();
    expect(screen.queryByText('Loading the stack…')).not.toBeInTheDocument();

    failing = false;
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByRole('list', { name: 'Stack order' })).toBeInTheDocument();
  });

  it('lets a malformed options.txt speak without a second launch hint', async () => {
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    client.getResourceStack = async () => {
      throw new IpcError(
        'options_txt',
        'options.txt is malformed: no resourcePacks line; Mine Utils never rewrites a malformed file',
      );
    };
    renderPage(client);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/malformed/);
    expect(alert).toHaveTextContent(/never rewrites a malformed file/);
    expect(
      screen.queryByText('Launch Minecraft once so it writes options.txt, then retry.'),
    ).not.toBeInTheDocument();
  });

  it('pauses writes and explains an unknown guard', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    client.getResourceStack = async () => ({
      entries: [{ kind: 'file', value: 'file/Vanilla.zip', filename: 'Vanilla.zip' }],
      incompatible: [],
      running: null,
      stale_locks: [],
      detection_supported: true,
    });
    render(
      <PacksPage client={client} gameRunning={null} gameGuardUnknown />,
    );

    const list = await screen.findByRole('list', { name: 'Stack order' });
    expect(
      screen.getByText(/could not check\s+whether Minecraft is running, so writes are paused/i),
    ).toBeInTheDocument();
    expect(within(list).getByRole('checkbox')).toBeDisabled();

    await user.click(screen.getByRole('button', { name: /^Vanilla,/ }));
    const detail = screen.getByRole('complementary', { name: 'Pack details' });
    expect(within(detail).getByRole('button', { name: 'Delete pack' })).toBeDisabled();
  });

  it('refreshes the stack and shows the banner after a game_running toggle error', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    const running: GameRunning = {
      kind: 'session_lock',
      detail: '/games/one/saves/world/session.lock is locked by pid 4242',
      lock_path: '/games/one/saves/world/session.lock',
      pid: 4242,
      command: null,
    };
    let nowRunning: GameRunning | null = null;
    client.getResourceStack = async () => ({
      entries: [{ kind: 'file', value: 'file/Vanilla.zip', filename: 'Vanilla.zip' }],
      incompatible: [],
      running: nowRunning,
      stale_locks: [],
      detection_supported: true,
    });
    client.setResourcePackEnabled = async () => {
      nowRunning = running;
      throw new IpcError('game_running', 'Minecraft is running; close it first');
    };
    render(<GuardedPage client={client} answer={() => nowRunning} />);

    const list = await screen.findByRole('list', { name: 'Stack order' });
    expect(screen.queryByText(/Minecraft is running/)).not.toBeInTheDocument();
    await user.click(within(list).getByRole('checkbox'));

    const banner = (await screen.findByText('Minecraft is running.')).closest(
      '.banner',
    ) as HTMLElement;
    expect(banner).toHaveTextContent('session.lock is locked by pid 4242');
    expect(within(list).getByRole('checkbox')).toBeDisabled();
  });

  it('rechecks both the stack and the app guard from the banner', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    let stackReads = 0;
    let guardChecks = 0;
    client.getResourceStack = async () => {
      stackReads += 1;
      return {
        entries: [{ kind: 'file', value: 'file/Vanilla.zip', filename: 'Vanilla.zip' }],
        incompatible: [],
        running: {
          kind: 'process',
          detail: 'pid 4242 runs java',
          lock_path: null,
          pid: 4242,
          command: 'java',
        },
        stale_locks: [],
        detection_supported: true,
      };
    };
    render(
      <PacksPage
        client={client}
        gameRunning={{
          kind: 'process',
          detail: 'pid 4242 runs java',
          lock_path: null,
          pid: 4242,
          command: 'java',
        }}
        refreshGameRunning={() => {
          guardChecks += 1;
        }}
      />,
    );

    await screen.findByText('Minecraft is running.');
    const before = stackReads;
    await user.click(screen.getByRole('button', { name: 'Recheck' }));
    await waitFor(() => expect(stackReads).toBeGreaterThan(before));
    expect(guardChecks).toBeGreaterThanOrEqual(1);
  });

  it('lets a fresh null app guard clear a stale stack banner', async () => {
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    client.getResourceStack = async () => ({
      entries: [{ kind: 'file', value: 'file/Vanilla.zip', filename: 'Vanilla.zip' }],
      incompatible: [],
      running: {
        kind: 'process',
        detail: 'pid 4242 runs java',
        lock_path: null,
        pid: 4242,
        command: 'java',
      },
      stale_locks: [],
      detection_supported: true,
    });
    render(<PacksPage client={client} gameRunning={null} refreshGameRunning={() => {}} />);

    const list = await screen.findByRole('list', { name: 'Stack order' });
    expect(screen.queryByText(/Minecraft is running/)).not.toBeInTheDocument();
    expect(within(list).getByRole('checkbox')).toBeEnabled();
  });

  it('holds writes with the checking text until the first guard answer, even when the stored stack says running', async () => {
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    client.getResourceStack = async () => ({
      entries: [{ kind: 'file', value: 'file/Vanilla.zip', filename: 'Vanilla.zip' }],
      incompatible: [],
      running: {
        kind: 'process',
        detail: 'pid 4242 runs java',
        lock_path: null,
        pid: 4242,
        command: 'java',
      },
      stale_locks: [],
      detection_supported: true,
    });
    const { rerender } = render(<PacksPage client={client} />);

    const list = await screen.findByRole('list', { name: 'Stack order' });
    expect(screen.getByRole('button', { name: 'Import pack' })).toBeDisabled();
    expect(within(list).getByRole('checkbox')).toBeDisabled();
    expect(screen.getByText(/^Checking whether Minecraft is running; writes are paused/)).toBeInTheDocument();
    expect(screen.queryByText('Minecraft is running.')).not.toBeInTheDocument();

    rerender(<PacksPage client={client} gameRunning={null} />);
    expect(screen.getByRole('button', { name: 'Import pack' })).toBeEnabled();
    expect(within(list).getByRole('checkbox')).toBeEnabled();
  });

  it('does not claim lock detection before the stack loads when the guard reports it unsupported', async () => {
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    client.getResourceStack = () => new Promise<never>(() => {});
    render(<PacksPage client={client} gameRunning={null} lockDetectionSupported={false} />);

    expect(await screen.findByText('Loading the stack…')).toBeInTheDocument();
    expect(
      screen.getByText(/Game lock detection is not supported here, so writes are never blocked automatically\./),
    ).toBeInTheDocument();
  });

  it('does not offer the stack toggle for a data pack', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK, AUTUMN_PACK] });
    renderPage(client);

    await user.click(await screen.findByRole('button', { name: /^Autumn,/ }));
    const detail = screen.getByRole('complementary', { name: 'Pack details' });
    expect(
      within(detail).queryByRole('checkbox', { name: /Enabled in the active stack/ }),
    ).not.toBeInTheDocument();
    expect(
      within(detail).getByText('Data packs are not part of the resource pack stack.'),
    ).toBeInTheDocument();
  });

  it('disables import when no game dir is active', async () => {
    const client: PackClient = {
      async listResourcePacks() {
        throw new IpcError(
          'no_active_game_dir',
          'no active game dir; pick one in Settings first',
        );
      },
      async importResourcePack() {
        throw new IpcError('no_active_game_dir', 'no active game dir');
      },
      async restoreResourceHolding() {},
      async deleteResourcePack() {
        throw new IpcError('pack_not_found', 'no packs');
      },
      async getResourceStack() {
        throw new IpcError('no_active_game_dir', 'no active game dir');
      },
      async setResourcePackEnabled() {
        throw new IpcError('no_active_game_dir', 'no active game dir');
      },
      async reorderResourceStack() {
        throw new IpcError('no_active_game_dir', 'no active game dir');
      },
      async listOptionsBackups() {
        throw new IpcError('no_active_game_dir', 'no active game dir');
      },
      async restoreOptionsBackup() {
        throw new IpcError('no_active_game_dir', 'no active game dir');
      },
    };
    render(<PacksPage client={client} gameRunning={null} />);

    expect(await screen.findByRole('button', { name: 'Import pack' })).toBeDisabled();
  });

  it('toggles with one client call and renders the stack the write returns', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({
      packs: [VANILLA_PACK],
      resource: ['vanilla', 'file/Vanilla.zip'],
    });
    const calls: string[] = [];
    const read = client.getResourceStack;
    const write = client.setResourcePackEnabled;
    client.getResourceStack = async () => {
      calls.push('getResourceStack');
      return read();
    };
    client.setResourcePackEnabled = async (filename, enabled) => {
      calls.push('setResourcePackEnabled');
      return write(filename, enabled);
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });
    calls.length = 0;

    await user.click(screen.getByRole('checkbox', { name: 'Enabled: Vanilla' }));
    await settle();

    expect(calls).toEqual(['setResourcePackEnabled']);
    expect(stackNames()).toEqual(['vanilla']);
  });

  it('reorders with one client call and renders the stack the write returns', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({
      packs: [VANILLA_PACK],
      resource: ['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip'],
    });
    const calls: string[] = [];
    const read = client.getResourceStack;
    const write = client.reorderResourceStack;
    client.getResourceStack = async () => {
      calls.push('getResourceStack');
      return read();
    };
    client.reorderResourceStack = async (order, expected) => {
      calls.push('reorderResourceStack');
      return write(order, expected);
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });
    expect(stackNames()).toEqual(['Autumn.zip', 'Vanilla', 'vanilla']);
    calls.length = 0;

    await user.click(screen.getByRole('button', { name: 'Move Autumn.zip down' }));
    await settle();

    expect(calls).toEqual(['reorderResourceStack']);
    expect(stackNames()).toEqual(['Vanilla', 'Autumn.zip', 'vanilla']);
  });

  it('keeps the toggled stack on screen when a read fails after the write', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({
      packs: [VANILLA_PACK],
      resource: ['vanilla', 'file/Vanilla.zip'],
    });
    const read = client.getResourceStack;
    const write = client.setResourcePackEnabled;
    let committed = false;
    client.setResourcePackEnabled = async (filename, enabled) => {
      const next = await write(filename, enabled);
      committed = true;
      return next;
    };
    client.getResourceStack = async () => {
      if (committed) throw new IpcError('options_txt', 'options.txt is unreadable');
      return read();
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });

    await user.click(screen.getByRole('checkbox', { name: 'Enabled: Vanilla' }));
    await settle();

    expect(committed).toBe(true);
    expect(stackNames()).toEqual(['vanilla']);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('keeps the stack on screen when a recheck read fails', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({
      packs: [VANILLA_PACK],
      resource: ['vanilla', 'file/Vanilla.zip'],
    });
    const read = client.getResourceStack;
    let failing = false;
    client.getResourceStack = async () => {
      if (failing) throw new IpcError('options_txt', 'options.txt is unreadable');
      return read();
    };
    render(<PacksPage client={client} gameRunning={null} gameGuardUnknown />);
    await screen.findByRole('list', { name: 'Stack order' });

    failing = true;
    await user.click(screen.getByRole('button', { name: 'Recheck' }));
    await settle();

    expect(stackNames()).toEqual(['Vanilla', 'vanilla']);
    expect(screen.getByText('options.txt is unreadable')).toBeInTheDocument();

    failing = false;
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await settle();

    expect(screen.queryByText('options.txt is unreadable')).not.toBeInTheDocument();
    expect(stackNames()).toEqual(['Vanilla', 'vanilla']);
  });

  it('shows the disk stack after a reorder is refused as stale', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    let disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip']);
    client.getResourceStack = async () => disk;
    client.reorderResourceStack = async () => {
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });

    disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip', 'file/Brand.zip']);
    await user.click(screen.getByRole('button', { name: 'Move Autumn.zip down' }));
    await settle();

    expect(screen.getByRole('alert')).toHaveTextContent('the stack changed on disk');
    expect(stackNames()).toEqual(['Brand.zip', 'Autumn.zip', 'Vanilla', 'vanilla']);
  });

  it('reads the stack once when a reorder is refused as stale', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    let disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip']);
    let reads = 0;
    client.getResourceStack = async () => {
      reads += 1;
      return disk;
    };
    client.reorderResourceStack = async () => {
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });
    const readsBefore = reads;

    disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip', 'file/Brand.zip']);
    await user.click(screen.getByRole('button', { name: 'Move Autumn.zip down' }));
    await settle();

    expect(reads - readsBefore).toBe(1);
    expect(screen.getByRole('alert')).toHaveTextContent('the stack changed on disk');
    expect(stackNames()).toEqual(['Brand.zip', 'Autumn.zip', 'Vanilla', 'vanilla']);
  });

  it('clears the stale-stack banner after Retry reloads the stack', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    let disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip']);
    let reads = 0;
    client.getResourceStack = async () => {
      reads += 1;
      return disk;
    };
    client.reorderResourceStack = async () => {
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });

    disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip', 'file/Brand.zip']);
    await user.click(screen.getByRole('button', { name: 'Move Autumn.zip down' }));
    await settle();

    const banner = screen.getByRole('alert');
    expect(banner).toHaveTextContent('the stack changed on disk');
    const readsAtBanner = reads;
    await user.click(within(banner).getByRole('button', { name: 'Retry' }));
    await settle();

    expect(screen.queryByText(/the stack changed on disk/)).not.toBeInTheDocument();
    expect(reads).toBe(readsAtBanner + 1);
    expect(stackNames()).toEqual(['Brand.zip', 'Autumn.zip', 'Vanilla', 'vanilla']);
  });

  it('keeps the stale-stack banner when Retry cannot reload the stack', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    const disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip']);
    let failReload = false;
    client.getResourceStack = async () => {
      if (failReload) throw new IpcError('options_txt', 'options.txt is unreadable');
      return disk;
    };
    client.reorderResourceStack = async () => {
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });

    await user.click(screen.getByRole('button', { name: 'Move Autumn.zip down' }));
    await settle();

    const banner = screen.getByRole('alert');
    expect(banner).toHaveTextContent('the stack changed on disk');
    failReload = true;
    await user.click(within(banner).getByRole('button', { name: 'Retry' }));
    await settle();

    expect(screen.getByText(/the stack changed on disk/)).toBeInTheDocument();
    expect(screen.getByText('options.txt is unreadable')).toBeInTheDocument();
  });

  it('reloads the stack after a toggle is refused as stale', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient({ packs: [VANILLA_PACK] });
    let disk = resourceStack(['vanilla', 'file/Vanilla.zip']);
    client.getResourceStack = async () => disk;
    client.setResourcePackEnabled = async () => {
      throw new IpcError('stale_stack', 'the stack changed on disk; reload it and try again');
    };
    renderPage(client);
    await screen.findByRole('list', { name: 'Stack order' });
    expect(stackNames()).toEqual(['Vanilla', 'vanilla']);

    disk = resourceStack(['vanilla', 'file/Vanilla.zip', 'file/Autumn.zip']);
    await user.click(screen.getByRole('checkbox', { name: 'Enabled: Vanilla' }));
    await settle();

    expect(screen.getByRole('alert')).toHaveTextContent('the stack changed on disk');
    expect(stackNames()).toEqual(['Autumn.zip', 'Vanilla', 'vanilla']);
  });
});

/** Flushes the resolved fake calls and the React updates they trigger. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

/** The stack list's entries, top first, by their accessible names. */
function stackNames(): Array<string | null> {
  return within(screen.getByRole('list', { name: 'Stack order' }))
    .getAllByRole('listitem')
    .map((item) => item.getAttribute('aria-label'));
}

it('restores an interrupted Replace and refreshes the pack listing', async () => {
  const client = makeFakeClient();
  let restored = false;
  const listing = await client.listResourcePacks();
  client.listResourcePacks = async () => ({
    ...listing,
    packs: restored ? [VANILLA_PACK] : [],
    holding_entries: restored ? [] : [{ holdingName: '.mine-utils-replace-Vanilla.zip.42-123-0.tmp', originalName: 'Vanilla.zip' }],
  });
  client.restoreResourceHolding = async (name) => {
    expect(name).toBe('.mine-utils-replace-Vanilla.zip.42-123-0.tmp');
    restored = true;
  };
  renderPage(client);
  await userEvent.click(await screen.findByRole('button', { name: 'Restore Vanilla.zip' }));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Restore Vanilla.zip' })).not.toBeInTheDocument());
  expect(await screen.findByText('Vanilla')).toBeInTheDocument();
});

it.each(['pack_not_found', 'io', 'pack_symlink'] as const)('reloads after Restore fails with %s', async (code) => {
  const client = makeFakeClient();
  let attempted = false;
  const listing = await client.listResourcePacks();
  client.listResourcePacks = async () => ({
    ...listing,
    packs: attempted ? [VANILLA_PACK] : [],
    holding_entries: attempted ? [] : [{ holdingName: '.mine-utils-replace-Vanilla.zip.42-123-0.tmp', originalName: 'Vanilla.zip' }],
  });
  client.restoreResourceHolding = async () => {
    attempted = true;
    throw new IpcError(code, 'The leftover Replace entry is gone at /resourcepacks/holding');
  };
  renderPage(client);
  await userEvent.click(await screen.findByRole('button', { name: 'Restore Vanilla.zip' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('The leftover Replace entry is gone at /resourcepacks/holding');
  expect(await screen.findByText('Vanilla')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Restore Vanilla.zip' })).not.toBeInTheDocument();
});
