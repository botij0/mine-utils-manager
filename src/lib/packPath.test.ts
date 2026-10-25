import { describe, expect, it } from 'vitest';
import { packFilename } from './packPath';

describe('packFilename', () => {
  it('takes the basename of a Unix path', () => {
    expect(packFilename({ path: '/home/demo/.minecraft/resourcepacks/Autumn.zip' })).toBe(
      'Autumn.zip',
    );
  });

  it('takes the basename of a Windows path', () => {
    expect(packFilename({ path: 'C:\\Users\\demo\\resourcepacks\\Autumn.zip' })).toBe(
      'Autumn.zip',
    );
  });

  it('keeps a backslash inside a Unix file name', () => {
    expect(packFilename({ path: '/packs/A\\B.zip' })).toBe('A\\B.zip');
    expect(packFilename({ path: 'A\\B.zip' })).toBe('A\\B.zip');
  });

  it('handles a bare name and trailing separators', () => {
    expect(packFilename({ path: 'Autumn.zip' })).toBe('Autumn.zip');
    expect(packFilename({ path: '/packs/Autumn/' })).toBe('Autumn');
    expect(packFilename({ path: '' })).toBe('');
  });
});
