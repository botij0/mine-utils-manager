import type { FormatRange, FormatVersion, Pack, PackKind } from './types';

/** The minor the backend uses for a max written as a whole number, which
 *  accepts every minor of that major. */
export const ANY_MINOR = 0x7fffffff;

export function formatVersionLabel(version: FormatVersion): string {
  return version.minor === 0 || version.minor === ANY_MINOR
    ? String(version.major)
    : `${version.major}.${version.minor}`;
}

export function formatRangeLabel(range: FormatRange | null): string {
  if (!range || (!range.min && !range.max)) return 'Unknown format';
  const { min, max } = range;
  if (min && max) {
    const low = formatVersionLabel(min);
    const high = formatVersionLabel(max);
    return low === high ? low : `${low} to ${high}`;
  }
  if (min) return `${formatVersionLabel(min)} and up`;
  if (max) return `up to ${formatVersionLabel(max)}`;
  return 'Unknown format';
}

/** "Format 34 to 42" or "Unknown format". Never invents a game version. */
export function packFormatLabel(pack: Pack): string {
  if (!pack.format_range) return 'Unknown format';
  return `Format ${formatRangeLabel(pack.format_range)}`;
}

/** The game range from the backend table, or the raw format as a fallback. */
export function packGameLabel(pack: Pack): string {
  if (pack.game_label) return pack.game_label;
  if (!pack.format_range) return 'Unknown game version';
  return `Untested on ${formatRangeLabel(pack.format_range)}`;
}

export function packKindLabel(kind: PackKind): string {
  switch (kind) {
    case 'resource':
      return 'Resource';
    case 'data':
      return 'Data';
    case 'both':
      return 'Resource and data';
    default:
      return 'Unknown';
  }
}

export function kindFilterLabel(kind: PackKind | 'all'): string {
  return kind === 'all' ? 'All kinds' : packKindLabel(kind);
}

/** Unique format ranges, sorted by their lowest format number. */
export function formatFilterOptions(packs: Pack[]): string[] {
  const lowEndByLabel = new Map<string, { major: number; minor: number }>();
  for (const pack of packs) {
    if (!pack.format_range) continue;
    const label = formatRangeLabel(pack.format_range);
    if (lowEndByLabel.has(label)) continue;
    const bound = pack.format_range.min ?? pack.format_range.max;
    lowEndByLabel.set(label, { major: bound?.major ?? 0, minor: bound?.minor ?? 0 });
  }
  return [...lowEndByLabel.entries()]
    .sort(([, left], [, right]) => left.major - right.major || left.minor - right.minor)
    .map(([label]) => label);
}

export function matchesSearch(pack: Pack, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return (
    pack.name.toLowerCase().includes(needle) ||
    pack.description.toLowerCase().includes(needle)
  );
}

export function matchesKind(pack: Pack, kind: PackKind | 'all'): boolean {
  return kind === 'all' || pack.kind === kind;
}

export function matchesFormat(pack: Pack, format: string): boolean {
  return format === 'all' || formatRangeLabel(pack.format_range) === format;
}

/** Shorten a long description for the card, leaving the detail panel full text. */
export function shortDescription(description: string, limit = 90): string {
  if (description.length <= limit) return description;
  return `${description.slice(0, limit - 1).trimEnd()}…`;
}
