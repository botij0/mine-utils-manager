import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { pack } from '../../test/fake-packs';
import PackCard from './PackCard';

const unread = {
  code: 'pack_missing_meta' as const,
  message: 'The pack has no pack.mcmeta',
};

describe('broken pack cards', () => {
  it('calls a broken zip an archive', () => {
    render(
      <PackCard
        pack={pack('BrokenZip', { location: 'zip', error: unread })}
        selected={false}
        onSelect={vi.fn()}
      />,
    );
    const card = screen.getByRole('button', { name: 'BrokenZip, error card' });
    expect(card).toHaveTextContent('Cannot read this archive');
    expect(card).not.toHaveTextContent('Cannot read this folder');
  });

  it('calls a broken folder a folder', () => {
    render(
      <PackCard
        pack={pack('BrokenFolder', { location: 'directory', error: unread })}
        selected={false}
        onSelect={vi.fn()}
      />,
    );
    const card = screen.getByRole('button', { name: 'BrokenFolder, error card' });
    expect(card).toHaveTextContent('Cannot read this folder');
    expect(card).not.toHaveTextContent('archive');
  });
});
