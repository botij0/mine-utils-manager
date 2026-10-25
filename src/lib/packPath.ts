/**
 * The on-disk file name of a pack. `file/<filename>` in options.txt is this,
 * never the display name, which strips `.zip`. Windows separators are split
 * too, so a path from another platform cannot leak into the stack filename.
 */
export function packFilename(pack: { path: string }): string {
  return pack.path.split(/[\\/]/).filter(Boolean).at(-1) ?? pack.path;
}
