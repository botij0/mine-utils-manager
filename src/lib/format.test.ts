import { describe, expect, it } from 'vitest';
import {
  formatRangeLabel,
  formatVersionLabel,
  matchesFormat,
  matchesSearch,
  packFormatLabel,
  packGameLabel,
  shortDescription,
} from './format';
import type { FormatVersion, Pack } from './types';

function version(major: number, minor = 0): FormatVersion {
  return { major, minor };
}

function pack(overrides: Partial<Pack> = {}): Pack {
  const formatRange = { min: version(34), max: version(94, 1) };
  return {
    id: 'pack',
    name: 'Pack',
    path: '/packs/Pack.zip',
    location: 'zip',
    kind: 'resource',
    description: 'A pack',
    meta: {
      pack_format: version(34),
      min_format: null,
      max_format: null,
      supported_formats: null,
      description: 'A pack',
    },
    format: formatRange.min,
    format_range: formatRange,
    game_label: '1.21 to 1.21.11',
    icon: null,
    error: null,
    bytes_read: 512,
    ...overrides,
  };
}

describe('format labels', () => {
  it('hides a zero minor and keeps dotted formats', () => {
    expect(formatVersionLabel(version(34))).toBe('34');
    expect(formatVersionLabel(version(75, 0))).toBe('75');
    expect(formatVersionLabel(version(94, 1))).toBe('94.1');
    expect(formatRangeLabel({ min: version(34), max: version(69) })).toBe('34 to 69');
    expect(formatRangeLabel({ min: version(94, 1), max: version(94, 1) })).toBe('94.1');
    expect(formatRangeLabel(null)).toBe('Unknown format');
  });

  it('labels unknown format numbers without inventing a version', () => {
    const unknown = pack({
      format_range: { min: version(999), max: version(999) },
      game_label: null,
    });
    expect(packFormatLabel(unknown)).toBe('Format 999');
    expect(packGameLabel(unknown)).toBe('Untested on 999');

    const empty = pack({ format_range: null, format: null, game_label: null });
    expect(packFormatLabel(empty)).toBe('Unknown format');
    expect(packGameLabel(empty)).toBe('Unknown game version');
  });

  it('uses the backend game label when present', () => {
    expect(packGameLabel(pack())).toBe('1.21 to 1.21.11');
  });

  it('caps long descriptions for cards but not detail text', () => {
    const long = 'x'.repeat(120);
    expect(shortDescription(long).length).toBe(90);
    expect(shortDescription(long).endsWith('…')).toBe(true);
    expect(shortDescription('short', 90)).toBe('short');
    expect(shortDescription(long, 500)).toBe(long);
  });

  it('matches search across name and description', () => {
    const subject = pack({ name: 'Autumn', description: 'Seasonal recipes' });
    expect(matchesSearch(subject, 'aut')).toBe(true);
    expect(matchesSearch(subject, 'RECIPE')).toBe(true);
    expect(matchesSearch(subject, 'winter')).toBe(false);
    expect(matchesSearch(subject, '  ')).toBe(true);
  });

  it('matches format labels exactly', () => {
    const subject = pack();
    expect(matchesFormat(subject, 'all')).toBe(true);
    expect(matchesFormat(subject, '34 to 94.1')).toBe(true);
    expect(matchesFormat(subject, '34 to 69')).toBe(false);
  });
});
