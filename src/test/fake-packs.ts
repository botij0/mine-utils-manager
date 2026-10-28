import type {
  Config,
  DataStack,
  FormatRange,
  FormatVersion,
  GameDirEntry,
  ModrinthKind,
  ModrinthProject,
  ModrinthSearchHit,
  ModrinthTargets,
  ModrinthVersion,
  Pack,
  PackMeta,
  ResourceStack,
  StackEntry,
  World,
} from '../lib/types';
import { IpcError } from '../lib/types';

export function iconDataUrl(label: string, background: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="10" fill="${background}"/><text x="32" y="40" font-family="sans-serif" font-size="26" font-weight="700" text-anchor="middle" fill="#fff">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

export function formatVersion(major: number, minor = 0): FormatVersion {
  return { major, minor };
}

export function formatRange(min: number, max: number): FormatRange {
  return { min: formatVersion(min), max: formatVersion(max) };
}

export function emptyMeta(description = ''): PackMeta {
  return {
    pack_format: null,
    min_format: null,
    max_format: null,
    supported_formats: null,
    description,
  };
}

export function stackEntry(value: string): StackEntry {
  return value.startsWith('file/')
    ? { kind: 'file', value, filename: value.slice('file/'.length) }
    : { kind: 'foreign', value };
}

export function gameDir(
  path: string,
  present = true,
  stored = true,
): GameDirEntry {
  return {
    path,
    source: stored ? 'manual' : 'std',
    present,
    stored,
  };
}

export function pack(name: string, overrides: Partial<Pack> = {}): Pack {
  return {
    id: `/home/demo/.minecraft/resourcepacks/${name}`,
    name,
    path: `/home/demo/.minecraft/resourcepacks/${name}.zip`,
    location: 'zip',
    kind: 'resource',
    description: 'A synthetic pack',
    meta: {
      pack_format: formatVersion(34),
      min_format: null,
      max_format: null,
      supported_formats: null,
      description: 'A synthetic pack',
    },
    format: formatVersion(34),
    format_range: formatRange(34, 42),
    game_label: '1.21 to 1.21.3',
    icon: null,
    error: null,
    bytes_read: 512,
    ...overrides,
  };
}

export function datapack(
  name: string,
  min: number | null = 7,
  max: number | null = 7,
  overrides: Partial<Pack> = {},
): Pack {
  const minFormat = min === null ? null : formatVersion(min);
  const maxFormat = max === null ? null : formatVersion(max);
  return {
    id: `/saves/Survival 1_17/datapacks/${name}`,
    name,
    path: `/saves/Survival 1_17/datapacks/${name}`,
    location: 'zip',
    kind: 'data',
    description: 'A datapack',
    meta: {
      pack_format: null,
      min_format: minFormat,
      max_format: maxFormat,
      supported_formats: null,
      description: 'A datapack',
    },
    format: minFormat,
    format_range: { min: minFormat, max: maxFormat },
    game_label: '1.17 to 1.17.1',
    icon: null,
    error: null,
    bytes_read: 128,
    ...overrides,
  };
}

export function world(overrides: Partial<World> = {}): World {
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

export function stack(overrides: Partial<DataStack> = {}): DataStack {
  return {
    enabled: [
      { kind: 'foreign', value: 'vanilla' },
      { kind: 'file', value: 'file/a.zip', filename: 'a.zip' },
      { kind: 'foreign', value: 'fabric' },
    ],
    disabled: [{ kind: 'file', value: 'file/b.zip', filename: 'b.zip' }],
    running: null,
    world_locked: null,
    stale_locks: [],
    detection_supported: true,
    ...overrides,
  };
}

export function resourceStack(
  values: string[] = ['vanilla', 'fabric'],
  overrides: Partial<ResourceStack> = {},
): ResourceStack {
  return {
    entries: values.map(stackEntry),
    incompatible: [],
    running: null,
    stale_locks: [],
    detection_supported: true,
    ...overrides,
  };
}

export const VANILLA_PACK = pack('Vanilla', {
  description: 'Small UI tweaks',
  format_range: formatRange(34, 69),
  game_label: '1.21 to 1.21.10',
});

export const AUTUMN_PACK = pack('Autumn', {
  kind: 'data',
  description: 'Seasonal recipes for a server',
  format_range: formatRange(48, 57),
  game_label: '1.21 to 1.21.3',
});

export const BROKEN_PACK = pack('Broken', {
  kind: 'unknown',
  format_range: null,
  format: null,
  game_label: null,
  error: {
    code: 'pack_broken_archive',
    message: 'Broken is not a readable pack: invalid Zip archive',
  },
});

export const SYMLINK_PACK = pack('Linked', {
  path: '/home/demo/.minecraft/resourcepacks/Linked',
  location: 'directory',
  kind: 'unknown',
  format_range: null,
  format: null,
  game_label: null,
  error: {
    code: 'pack_symlink',
    message: 'Linked is a symlink; Mine Utils does not follow links',
  },
});

function looksLikeGameDir(path: string): boolean {
  return (
    path.includes('.minecraft') ||
    /\/(saves|versions|resourcepacks)(\/|$)/.test(path)
  );
}

export function withGameDirAdded(config: Config, path: string): Config {
  if (!looksLikeGameDir(path)) {
    throw new IpcError(
      'invalid_game_dir',
      `invalid game dir ${path}: expected one of versions, saves, or resourcepacks`,
    );
  }
  if (config.game_dirs.some((dir) => dir.path === path)) return config;
  return {
    ...config,
    game_dirs: [...config.game_dirs, { path, source: 'manual' }],
  };
}

export function withGameDirRemoved(config: Config, path: string): Config {
  if (!config.game_dirs.some((dir) => dir.path === path)) {
    throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
  }
  return {
    ...config,
    game_dirs: config.game_dirs.filter((dir) => dir.path !== path),
    active_game_dir:
      config.active_game_dir === path ? null : config.active_game_dir,
  };
}

export function withActiveGameDir(
  config: Config,
  entries: GameDirEntry[],
  path: string,
): Config {
  const target = entries.find((candidate) => candidate.path === path);
  if (!target) {
    throw new IpcError('unknown_game_dir', `unknown game dir ${path}`);
  }
  if (!target.present) {
    throw new IpcError(
      'invalid_game_dir',
      `invalid game dir ${path}: expected one of versions, saves, or resourcepacks`,
    );
  }
  return { ...config, active_game_dir: path };
}

export const MOCK_PROJECTS: ModrinthSearchHit[] = [
  {
    project_id: '7q7JnQD0',
    slug: 'fresh-animations',
    title: 'Fresh Animations',
    description: 'Fresh animations for mobs and players.',
    icon_url: iconDataUrl('FA', '#3f7d5a'),
    downloads: 123456,
    follows: 789,
    project_type: 'resourcepack',
    versions: ['1.21.1', '1.21'],
    latest_version: 'v2',
  },
  {
    project_id: 'bbbb',
    slug: 'other-pack',
    title: 'Other Pack',
    description: 'Another resource pack for testing filters.',
    icon_url: null,
    downloads: 10,
    follows: 1,
    project_type: 'resourcepack',
    versions: ['1.21'],
    latest_version: 'o1',
  },
  {
    project_id: 'dp1',
    slug: 'pure-datapack',
    title: 'Pure Datapack',
    description: 'Only datapack files, no mods.',
    icon_url: iconDataUrl('PD', '#5a3f7d'),
    downloads: 500,
    follows: 5,
    project_type: 'datapack',
    versions: ['1.21.1'],
    latest_version: 'd2',
  },
  {
    project_id: 'mm1',
    slug: 'mismatch-pack',
    title: 'Mismatch Pack',
    description: 'Installing this file always fails the hash check in the mock.',
    icon_url: null,
    downloads: 3,
    follows: 0,
    project_type: 'resourcepack',
    versions: ['1.21.1'],
    latest_version: 'm1',
  },
];

export const MOCK_VERSIONS: Record<string, ModrinthVersion[]> = {
  '7q7JnQD0': [
    {
      id: 'v2',
      project_id: '7q7JnQD0',
      name: 'Fresh Animations 1.9',
      version_number: '1.9',
      game_versions: ['1.21.1', '1.21'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/7q7/fresh-1.9.zip',
          filename: 'fresh-1.9.zip',
          size: 1024,
          primary: true,
          hashes: { sha512: 'mock-sha512', sha1: 'mock-sha1' },
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
          hashes: { sha512: 'mock-old', sha1: 'mock-old1' },
        },
      ],
    },
  ],
  bbbb: [
    {
      id: 'o1',
      project_id: 'bbbb',
      name: 'Other Pack 1.0',
      version_number: '1.0',
      game_versions: ['1.21'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/bbb/other-1.0.zip',
          filename: 'other-1.0.zip',
          size: 512,
          primary: true,
          hashes: { sha512: 'mock-other', sha1: 'mock-other1' },
        },
      ],
    },
  ],
  dp1: [
    {
      id: 'd2',
      project_id: 'dp1',
      name: 'Pure Datapack 2.0',
      version_number: '2.0',
      game_versions: ['1.21.1'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/dp1/pure-2.0.zip',
          filename: 'pure-2.0.zip',
          size: 256,
          primary: true,
          hashes: { sha512: 'mock-dp', sha1: 'mock-dp1' },
        },
      ],
    },
  ],
  mm1: [
    {
      id: 'm1',
      project_id: 'mm1',
      name: 'Mismatch Pack 1.0',
      version_number: '1.0',
      game_versions: ['1.21.1'],
      loaders: ['minecraft'],
      files: [
        {
          url: 'https://cdn.modrinth.com/data/mm1/mismatch.zip',
          filename: 'mismatch.zip',
          size: 128,
          primary: true,
          hashes: { sha512: '00aa', sha1: '00' },
        },
      ],
    },
  ],
};

function copyHit(hit: ModrinthSearchHit): ModrinthSearchHit {
  return { ...hit, versions: [...hit.versions] };
}

export function findMockProject(projectId: string): ModrinthSearchHit | undefined {
  return MOCK_PROJECTS.find(
    (item) => item.project_id === projectId || item.slug === projectId,
  );
}

export function fakeHits(kind: ModrinthKind): ModrinthSearchHit[] {
  const hit =
    kind === 'datapack'
      ? (findMockProject('pure-datapack') ?? MOCK_PROJECTS[0])
      : (findMockProject('fresh-animations') ?? MOCK_PROJECTS[0]);
  return [copyHit(hit)];
}

export function freshHit(): ModrinthSearchHit {
  return copyHit(findMockProject('fresh-animations') ?? MOCK_PROJECTS[0]);
}

export function mismatchHit(): ModrinthSearchHit {
  return copyHit(findMockProject('mismatch-pack') ?? MOCK_PROJECTS[0]);
}

export function versionsForProject(
  projectId: string,
): ModrinthVersion[] | undefined {
  const hit = findMockProject(projectId);
  return hit ? MOCK_VERSIONS[hit.project_id] : undefined;
}

export function fakeVersions(projectId: string): ModrinthVersion[] {
  return versionsForProject(projectId) ?? MOCK_VERSIONS['7q7JnQD0'] ?? [];
}

export function projectFromHit(
  hit: ModrinthSearchHit,
  body = hit.description,
  versions = versionsForProject(hit.project_id) ?? [],
): ModrinthProject {
  return {
    id: hit.project_id,
    slug: hit.slug,
    title: hit.title,
    description: hit.description,
    body,
    icon_url: hit.icon_url,
    downloads: hit.downloads,
    follows: hit.follows,
    project_type: hit.project_type,
    game_versions: [...hit.versions],
    versions: versions.map((version) => version.id),
  };
}

export function fakeProject(projectId: string): ModrinthProject {
  const hit = findMockProject(projectId) ?? {
    project_id: projectId,
    slug: projectId,
    title: projectId,
    description: '',
    icon_url: null,
    downloads: 0,
    follows: 0,
    project_type: 'resourcepack',
    versions: [] as string[],
    latest_version: null,
  };
  return projectFromHit(hit, hit.description, fakeVersions(hit.project_id));
}

export function modrinthTargets(
  worldVersion: string | null = '1.21.1',
): ModrinthTargets {
  return {
    installed_versions: ['1.21', '1.21.1'],
    preferred_launcher: '1.21.1',
    world_version: worldVersion,
  };
}

export function assertDownloadHash(filename: string): void {
  if (filename.toLowerCase().includes('mismatch')) {
    throw new IpcError(
      'download_hash_mismatch',
      'download failed: SHA-512 mismatch (expected 00aa, got ffbb)',
    );
  }
}
