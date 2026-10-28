export function packInitials(name: string): string {
  return name.replace(/[^A-Za-z0-9]/g, '').slice(0, 2).toUpperCase() || 'PK';
}

export function worldInitials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  return trimmed.slice(0, 2).toUpperCase();
}

export function hideBrokenImage(event: { currentTarget: HTMLImageElement }): void {
  // The asset-scheme issue can blank an <img> without firing an error the
  // page can see. When an error does fire, hide the image so the gradient
  // tile underneath shows instead of a broken glyph.
  event.currentTarget.style.display = 'none';
}

export function formatCount(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}k`;
  return `${count}`;
}
