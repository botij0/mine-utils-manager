import type { Pack } from './types';

/**
 * The on-disk file name of a pack. `file/<filename>` in options.txt is this,
 * never the display name, which strips `.zip`.
 *
 * The separator depends on the path style: a path with `/` is Unix-style; a
 * bare Windows drive path splits on `\`. Elsewhere a backslash is a legal
 * Unix filename character, so `A\B.zip` stays one name instead of becoming
 * `B.zip`.
 */
export function packFilename(pack: { path: string }): string {
  const path = pack.path;
  if (path.includes('/')) {
    return path.split('/').filter(Boolean).at(-1) ?? path;
  }
  if (/^[A-Za-z]:\\/.test(path)) {
    return path.split('\\').filter(Boolean).at(-1) ?? path;
  }
  return path;
}

export function hasNestedSymlink(pack: Pack): boolean {
  return pack.error?.code === 'pack_symlink' &&
    pack.error.symlink_path !== undefined && pack.error.symlink_path !== pack.path;
}
