import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { userEvent } from '@testing-library/user-event';
import InstallSheet from './InstallSheet';
import ProjectDetail from './ProjectDetail';
import VersionPicker from './VersionPicker';
import type {
  ModrinthInstallOutcome,
  ModrinthSearchHit,
  ModrinthVersion,
} from '../../lib/types';
import { isTrustedIconUrl } from './icon';

function hit(): ModrinthSearchHit {
  return {
    project_id: '7q7JnQD0',
    slug: 'fresh-animations',
    title: 'Fresh Animations',
    description: 'Fresh animations.',
    icon_url: 'https://cdn.modrinth.com/icons/fresh.webp',
    downloads: 100,
    follows: 10,
    project_type: 'resourcepack',
    versions: ['1.21.1'],
    latest_version: 'v2',
  };
}

function versions(): ModrinthVersion[] {
  return [
    {
      id: 'v2',
      project_id: '7q7JnQD0',
      name: 'Fresh Animations 1.9',
      version_number: '1.9',
      game_versions: ['1.21.1'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/7q7/fresh-1.9.zip',
          filename: 'fresh-1.9.zip',
          size: 1024,
          primary: true,
          hashes: { sha512: 'aa', sha1: 'a' },
        },
      ],
    },
    {
      id: 'v1',
      project_id: '7q7JnQD0',
      name: 'Fresh Animations 1.8',
      version_number: '1.8',
      game_versions: ['1.20.1'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/7q7/fresh-1.8.zip',
          filename: 'fresh-1.8.zip',
          size: 900,
          primary: true,
          hashes: { sha512: 'bb', sha1: 'b' },
        },
      ],
    },
  ];
}

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
        versions={versions()}
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
        versions={versions()}
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

  it('renders one dialog with aria-modal and closes on Escape', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const { container } = render(
      <ProjectDetail
        hit={hit()}
        project={null}
        versions={versions()}
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
