import type { FormatRange, FormatVersion, Pack, PackKind } from './types';

export function formatVersionLabel(version: FormatVersion): string {
  return version.minor === 0
    ? String(version.major)
    : `${version.major}.${version.minor}`;
}

export function formatRangeLabel(range: FormatRange | null): string {
  if (!range) return 'Unknown format';
  if (range.min.major === range.max.major && range.min.minor === range.max.minor) {
    return formatVersionLabel(range.min);
  }
  return `${formatVersionLabel(range.min)} to ${formatVersionLabel(range.max)}`;
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
  const seen = new Set<string>();
  for (const pack of packs) {
    if (pack.format_range) seen.add(formatRangeLabel(pack.format_range));
  }
  return [...seen].sort((left, right) => {
    const first = packs.find(
      (pack) => pack.format_range && formatRangeLabel(pack.format_range) === left,
    );
    const second = packs.find(
      (pack) => pack.format_range && formatRangeLabel(pack.format_range) === right,
    );
    return (
      (first?.format_range?.min.major ?? 0) - (second?.format_range?.min.major ?? 0) ||
      (first?.format_range?.min.minor ?? 0) - (second?.format_range?.min.minor ?? 0)
    );
  });
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
