import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { PackClient } from '../../lib/ipc';
import type {
  FormatRange,
  FormatVersion,
  ImportChoice,
  ImportOutcome,
  Pack,
  PackListing,
} from '../../lib/types';
import { IpcError } from '../../lib/types';
import PacksPage from './PacksPage';

function version(major: number, minor = 0): FormatVersion {
  return { major, minor };
}

function formatRange(min: number, max: number): FormatRange {
  return { min: version(min), max: version(max) };
}

function pack(
  name: string,
  overrides: Partial<Pack> = {},
): Pack {
  return {
    id: `/home/demo/.minecraft/resourcepacks/${name}`,
    name,
    path: `/home/demo/.minecraft/resourcepacks/${name}.zip`,
    location: 'zip',
    kind: 'resource',
    description: 'A synthetic pack',
    meta: {
      pack_format: version(34),
      min_format: null,
      max_format: null,
      supported_formats: null,
      description: 'A synthetic pack',
    },
    format: version(34),
    format_range: formatRange(34, 42),
    game_label: '1.21 to 1.21.3',
    icon: null,
    error: null,
    bytes_read: 512,
    ...overrides,
  };
}

const AUTUMN = pack('Autumn', {
  kind: 'data',
  description: 'Seasonal recipes for a server',
  format_range: formatRange(48, 57),
  game_label: '1.21 to 1.21.3',
});
const VANILLA = pack('Vanilla', {
  description: 'Small UI tweaks',
  format_range: formatRange(34, 69),
  game_label: '1.21 to 1.21.10',
});
const BROKEN = pack('Broken', {
  kind: 'unknown',
  format_range: null,
  format: null,
  game_label: null,
  error: {
    code: 'pack_broken_archive',
    message: 'Broken is not a readable pack: invalid Zip archive',
  },
});

interface FakeClient extends PackClient {
  current: Pack[];
  choices: ImportChoice[];
  imports: string[];
}

function makeFakeClient(initial: Pack[]): FakeClient {
  let current = [...initial];
  const choices: ImportChoice[] = [];
  const imports: string[] = [];
  const client: FakeClient = {
    get current() {
      return current;
    },
    choices,
    imports,
    async listResourcePacks(): Promise<PackListing> {
      return {
        packs: [...current],
        count: current.length,
        bytes_read: current.reduce((total, item) => total + item.bytes_read, 0),
        elapsed_ms: 4,
      };
    },
    async importResourcePack(
      path: string,
      choice: ImportChoice = 'default',
    ): Promise<ImportOutcome> {
      imports.push(path);
      choices.push(choice);
      const base = path.split('/').filter(Boolean).at(-1) ?? path;
      if (base.toLowerCase().includes('duplicate')) {
        if (choice === 'skip') return { status: 'skipped' };
        if (choice !== 'replace') {
          return {
            status: 'duplicate',
            name: base,
            sha256: 'abc123',
            existing: current[0]?.path ?? '/packs/existing.zip',
          };
        }
      }
      if (base.toLowerCase().includes('broken')) {
        throw new IpcError(
          'pack_broken_archive',
          `${base} is not a readable pack: invalid Zip archive`,
        );
      }
      const name = base.replace(/\.zip$/i, '');
      const imported = pack(name);
      current = [...current, imported];
      return { status: 'imported', pack: imported };
    },
    async deleteResourcePack(path: string) {
      current = current.filter((item) => item.path !== path);
    },
  };
  return client;
}

function renderPage(client: FakeClient) {
  return render(<PacksPage client={client} />);
}

describe('PacksPage', () => {
  it('lists packs with descriptions, chips, icons, and an isolated error card', async () => {
    const icon = { sha256: 'icon', url: 'data:image/png;base64,abc' };
    const client = makeFakeClient([{ ...VANILLA, icon }, AUTUMN, BROKEN]);
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
    renderPage(makeFakeClient([VANILLA, AUTUMN]));

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
    renderPage(makeFakeClient([VANILLA, AUTUMN]));

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
    const client = makeFakeClient([VANILLA]);
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
    const client = makeFakeClient([VANILLA]);
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
    expect(client.current).toHaveLength(1);
    expect(screen.getAllByText('Vanilla')).toHaveLength(1);
  });

  it('replaces on a duplicate prompt', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([VANILLA]);
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
    const client = makeFakeClient([VANILLA, AUTUMN]);
    renderPage(client);

    await user.click(await screen.findByRole('button', { name: /^Autumn,/ }));
    const detail = screen.getByRole('complementary', { name: 'Pack details' });
    await user.click(within(detail).getByRole('button', { name: 'Delete pack' }));
    await user.click(within(detail).getByRole('button', { name: 'Move to trash' }));

    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /^Autumn,/ })).not.toBeInTheDocument(),
    );
    expect(screen.getByText('Vanilla')).toBeInTheDocument();
    expect(client.current).toHaveLength(1);
  });

  it('shows an import failure inside the import sheet', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([VANILLA]);
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
    const client = makeFakeClient([VANILLA]);
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
    const client = makeFakeClient([VANILLA]);
    const picked: boolean[] = [];
    const pickPath = async (directory: boolean) => {
      picked.push(directory);
      return directory ? '/drop/PackFolder' : '/drop/Pack.zip';
    };
    render(<PacksPage client={client} pickPath={pickPath} />);

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
      async deleteResourcePack() {
        throw new IpcError('pack_not_found', 'no packs');
      },
    };
    render(<PacksPage client={client} />);

    expect(await screen.findByRole('button', { name: 'Import pack' })).toBeDisabled();
  });
});
