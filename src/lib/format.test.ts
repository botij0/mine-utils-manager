import { describe, expect, it } from 'vitest';
import {
  formatFilterOptions,
  formatRangeLabel,
  formatVersionLabel,
  matchesFormat,
  matchesSearch,
  packFormatLabel,
  packGameLabel,
  shortDescription,
} from './format';
import type { Pack } from './types';
import { formatVersion as version, pack as makePack } from '../test/fake-packs';

const FORMAT_RANGE = {
  min: version(34),
  max: version(94, 1),
};

function formatPack(overrides: Partial<Pack> = {}): Pack {
  return makePack('Pack', {
    id: 'pack',
    path: '/packs/Pack.zip',
    description: 'A pack',
    meta: {
      pack_format: version(34),
      min_format: null,
      max_format: null,
      supported_formats: null,
      description: 'A pack',
    },
    format: FORMAT_RANGE.min,
    format_range: FORMAT_RANGE,
    game_label: '1.21 to 1.21.11',
    ...overrides,
  });
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

  it('hides the any-minor marker of a max written as a whole number', () => {
    const anyMinor = 2147483647;
    expect(formatVersionLabel(version(94, anyMinor))).toBe('94');
    expect(formatRangeLabel({ min: version(88), max: version(94, anyMinor) })).toBe('88 to 94');
    expect(formatRangeLabel({ min: version(94), max: version(94, anyMinor) })).toBe('94');
  });

  it('labels a lone min or max as open instead of dropping it', () => {
    expect(formatRangeLabel({ min: version(48), max: null })).toBe('48 and up');
    expect(formatRangeLabel({ min: null, max: version(42) })).toBe('up to 42');
    expect(formatRangeLabel({ min: null, max: null })).toBe('Unknown format');
    const open = formatPack({
      format_range: { min: version(48), max: null },
      game_label: null,
    });
    expect(packFormatLabel(open)).toBe('Format 48 and up');
    expect(packGameLabel(open)).toBe('Untested on 48 and up');
  });

  it('labels unknown format numbers without inventing a version', () => {
    const unknown = formatPack({
      format_range: { min: version(999), max: version(999) },
      game_label: null,
    });
    expect(packFormatLabel(unknown)).toBe('Format 999');
    expect(packGameLabel(unknown)).toBe('Untested on 999');

    const empty = formatPack({ format_range: null, format: null, game_label: null });
    expect(packFormatLabel(empty)).toBe('Unknown format');
    expect(packGameLabel(empty)).toBe('Unknown game version');
  });

  it('uses the backend game label when present', () => {
    expect(packGameLabel(formatPack())).toBe('1.21 to 1.21.11');
  });

  it('caps long descriptions for cards but not detail text', () => {
    const long = 'x'.repeat(120);
    expect(shortDescription(long).length).toBe(90);
    expect(shortDescription(long).endsWith('…')).toBe(true);
    expect(shortDescription('short', 90)).toBe('short');
    expect(shortDescription(long, 500)).toBe(long);
  });

  it('matches search across name and description', () => {
    const subject = formatPack({ name: 'Autumn', description: 'Seasonal recipes' });
    expect(matchesSearch(subject, 'aut')).toBe(true);
    expect(matchesSearch(subject, 'RECIPE')).toBe(true);
    expect(matchesSearch(subject, 'winter')).toBe(false);
    expect(matchesSearch(subject, '  ')).toBe(true);
  });

  it('matches format labels exactly', () => {
    const subject = formatPack();
    expect(matchesFormat(subject, 'all')).toBe(true);
    expect(matchesFormat(subject, '34 to 94.1')).toBe(true);
    expect(matchesFormat(subject, '34 to 69')).toBe(false);
  });

  it('orders unique format options by lowest format number', () => {
    const subjects = [
      formatPack({ format_range: { min: version(48), max: null } }),
      formatPack({ format_range: { min: version(34), max: null } }),
      formatPack({ format_range: { min: version(34), max: version(69) } }),
      formatPack({ format_range: { min: version(34), max: version(69) } }),
      formatPack({ format_range: null, format: null }),
      formatPack({ format_range: { min: null, max: version(42) } }),
    ];
    expect(formatFilterOptions(subjects)).toEqual([
      '34 and up',
      '34 to 69',
      'up to 42',
      '48 and up',
    ]);
  });
});
