import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { userEvent } from '@testing-library/user-event';
import DiscoverPage from './DiscoverPage';
import InstallSheet from './InstallSheet';
import ProjectDetail from './ProjectDetail';
import VersionPicker from './VersionPicker';
import { IpcError } from '../../lib/types';
import type { ModrinthInstallOutcome, ModrinthSearchHit } from '../../lib/types';
import { makeFakeClient } from '../../test/fake-ipc';
import { fakeVersions, freshHit } from '../../test/fake-packs';
import { isTrustedIconUrl } from './icon';

function importedOutcome(): ModrinthInstallOutcome {
  return {
    kind: 'resourcepack',
    outcome: {
      status: 'imported',
      pack: {
        id: 'fresh-1.9.zip',
        name: 'fresh-1.9.zip',
        path: '/home/demo/.minecraft/resourcepacks/fresh-1.9.zip',
        location: 'zip',
        kind: 'resource',
        description: 'temp',
        meta: {
          pack_format: null,
          min_format: null,
          max_format: null,
          supported_formats: null,
          description: 'temp',
        },
        format: null,
        format_range: null,
        game_label: null,
        icon: null,
        error: null,
        bytes_read: 0,
      },
    },
  };
}

function searchHit(title: string): ModrinthSearchHit {
  return { ...freshHit(), project_id: title, slug: title, title };
}

function page(offset: number, count: number): ModrinthSearchHit[] {
  return Array.from({ length: count }, (_, index) =>
    searchHit(`Page ${String(offset + index).padStart(2, '0')}`),
  );
}

const sheetProps = {
  open: true,
  kind: 'resourcepack' as const,
  projectTitle: 'Fresh Animations',
  versionLabel: '1.9',
  filename: 'fresh-1.9.zip',
  worlds: [],
  world: null,
  onWorldChange: () => {},
  busy: false,
  writesBlocked: false,
  blockedReason: null,
  hasGameDir: true,
  result: null,
  error: null,
  onClose: () => {},
  onInstall: () => {},
};

describe('discover dialogs', () => {
  it('labels only the compatible version, not the default newest', () => {
    render(
      <VersionPicker
        versions={fakeVersions('7q7JnQD0')}
        selectedId="v2"
        onSelect={() => {}}
        compatibleId="v1"
        targetLabel="1.20.1"
      />,
    );
    const options = screen.getAllByRole('option');
    expect(options[0]).not.toHaveTextContent('compatible');
    expect(options[1]).toHaveTextContent('compatible');
  });

  it('shows no compatible label when nothing matches the target', () => {
    render(
      <VersionPicker
        versions={fakeVersions('7q7JnQD0')}
        selectedId="v2"
        onSelect={() => {}}
        compatibleId={null}
        targetLabel="1.19.4"
      />,
    );
    for (const option of screen.getAllByRole('option')) {
      expect(option).not.toHaveTextContent('compatible');
    }
  });

  it('claims a preselected compatible version only when one exists', () => {
    const { rerender } = render(
      <VersionPicker
        versions={fakeVersions('7q7JnQD0')}
        selectedId="v2"
        onSelect={() => {}}
        compatibleId="v2"
        targetLabel="1.21.1"
      />,
    );
    expect(screen.getByText(/compatible with 1\.21\.1 preselected/)).toBeInTheDocument();
    rerender(
      <VersionPicker
        versions={fakeVersions('7q7JnQD0')}
        selectedId="v2"
        onSelect={() => {}}
        compatibleId={null}
        targetLabel="1.19.4"
      />,
    );
    expect(screen.queryByText(/preselected/)).toBeNull();
    expect(
      screen.getByText(/No published version lists 1\.19\.4/),
    ).toBeInTheDocument();
  });

  it('shows the guard banner while the first check is still unknown', async () => {
    const { makeFakeClient } = await import('../../test/fake-ipc');
    const { default: DiscoverPage } = await import('./DiscoverPage');
    const client = makeFakeClient([]);
    const { rerender } = render(
      <DiscoverPage client={client} gameRunning={undefined} gameGuardUnknown hasGameDir />,
    );
    expect(screen.getByText(/Game guard unavailable/)).toBeInTheDocument();
    rerender(
      <DiscoverPage client={client} gameRunning={undefined} gameGuardUnknown={false} hasGameDir />,
    );
    expect(screen.queryByText(/Game guard unavailable/)).toBeNull();
  });

  it('renders one dialog with aria-modal and closes on Escape', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const { container } = render(
      <ProjectDetail
        hit={freshHit()}
        project={null}
        versions={fakeVersions('7q7JnQD0')}
        loading={false}
        error={null}
        selectedVersionId="v2"
        onSelectVersion={() => {}}
        compatibleId="v2"
        targetLabel="1.21.1"
        onInstall={() => {}}
        onClose={onClose}
      />,
    );
    const dialogs = container.querySelectorAll('[role="dialog"]');
    expect(dialogs).toHaveLength(1);
    expect(dialogs[0]).toHaveAttribute('aria-modal', 'true');
    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('surfaces the provenance warning without failing the install', () => {
    const result: ModrinthInstallOutcome = {
      ...importedOutcome(),
      provenance_warning: 'installed, but provenance was not recorded: disk full',
    };
    render(<InstallSheet {...sheetProps} result={result} />);
    expect(screen.getByText(/Installed fresh-1\.9\.zip and enabled it\./)).toBeInTheDocument();
    expect(screen.getByText(/provenance was not recorded/)).toBeInTheDocument();
  });

  it('explains a missing game dir and disables install', () => {
    render(<InstallSheet {...sheetProps} hasGameDir={false} writesBlocked blockedReason={null} />);
    expect(screen.getByText(/No game dir is active/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled();
  });

  it('renders the wire resourcepack kind with the install text', () => {
    // The backend serializes the outcome kind as `resourcepack` (one word);
    // a `resource_pack` payload must not silently render as a datapack.
    render(<InstallSheet {...sheetProps} result={importedOutcome()} />);
    expect(screen.getByText(/Installed fresh-1\.9\.zip and enabled it\./)).toBeInTheDocument();
  });
});

describe('discover Load more', () => {
  it('appends the next page and disables the action while it loads', async () => {
    const client = makeFakeClient([]);
    const calls: Array<number | undefined> = [];
    let release!: (hits: ModrinthSearchHit[]) => void;
    client.searchModrinth = async (_text, _kind, _gameVersion, _sort, offset = 0) => {
      calls.push(offset);
      if (offset === 0) return page(0, 20);
      return new Promise<ModrinthSearchHit[]>((resolve) => {
        release = resolve;
      });
    };

    render(<DiscoverPage client={client} gameRunning={null} hasGameDir />);
    fireEvent.change(screen.getByLabelText('Search Modrinth'), {
      target: { value: 'fresh' },
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Load more' }));
    expect(screen.getByText('Page 00')).toBeInTheDocument();

    const loading = await screen.findByRole('button', { name: 'Loading more…' });
    expect(loading).toBeDisabled();

    release(page(20, 5));
    await waitFor(() => expect(screen.getByText('Page 24')).toBeInTheDocument());

    expect(screen.getByText('Page 00')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull();
    expect(calls).toEqual([0, 20]);
  });

  it('keeps the loaded cards and offers Retry after a failed next page', async () => {
    const client = makeFakeClient([]);
    client.searchModrinth = async (_text, _kind, _gameVersion, _sort, offset = 0) => {
      if (offset === 0) return page(0, 20);
      throw new IpcError('modrinth_status', 'Modrinth request failed with status 500');
    };

    render(<DiscoverPage client={client} gameRunning={null} hasGameDir />);
    fireEvent.change(screen.getByLabelText('Search Modrinth'), {
      target: { value: 'fresh' },
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Load more' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Modrinth request failed with status 500',
    );
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.getByText('Page 00')).toBeInTheDocument();
  });
});

describe('isTrustedIconUrl', () => {
  it('trusts only the cdn https host', () => {
    expect(isTrustedIconUrl('https://cdn.modrinth.com/icons/fresh.webp')).toBe(true);
    expect(isTrustedIconUrl('https://cdn.modrinth.com:443/icons/fresh.webp')).toBe(true);
    expect(isTrustedIconUrl('data:image/svg+xml;utf8,abc')).toBe(true);
    expect(isTrustedIconUrl('http://cdn.modrinth.com/icons/fresh.webp')).toBe(false);
    expect(isTrustedIconUrl('https://cdn.modrinth.com:8443/icons/fresh.webp')).toBe(false);
    expect(isTrustedIconUrl('https://evil.example/icon.png')).toBe(false);
    expect(isTrustedIconUrl(null)).toBe(false);
    expect(isTrustedIconUrl('not a url')).toBe(false);
  });
});
