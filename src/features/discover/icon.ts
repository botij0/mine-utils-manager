/**
 * Remote icons render without proxying: only `https://cdn.modrinth.com` on
 * the default port is trusted. `data:image/` URLs never leave the page
 * (the offline mock uses them), so they are allowed; everything else
 * renders the local placeholder instead of hitting an unexpected host.
 */
export function isTrustedIconUrl(url: string | null | undefined): url is string {
  if (!url) return false;
  if (url.startsWith('data:image/')) return true;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  return (
    parsed.protocol === 'https:' &&
    parsed.hostname === 'cdn.modrinth.com' &&
    (parsed.port === '' || parsed.port === '443')
  );
}
