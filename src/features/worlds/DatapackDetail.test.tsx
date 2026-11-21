import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { PackWarning } from '../../lib/types';
import { datapack } from '../../test/fake-packs';
import DatapackDetail from './DatapackDetail';

const warnings = [
  {
    code: 'missing_min_max_format',
    message:
      'pack_format 82 needs min_format and max_format, but pack.mcmeta is missing min_format and max_format.',
  },
  {
    code: 'missing_min_max_format',
    message:
      'pack_format 88 needs min_format and max_format, but pack.mcmeta is missing max_format.',
  },
] satisfies PackWarning[];

describe('datapack detail warnings', () => {
  it('shows each pack warning message', () => {
    render(
      <DatapackDetail
        pack={datapack('Loose', 82, null, { warnings })}
        filename="Loose"
        enabled
        locked={false}
        missing={false}
        warning="Built for 1.21. Pack may not match this world."
        writesBlocked={false}
        busy={false}
        onToggle={vi.fn()}
        onDelete={vi.fn()}
        onRemoveReference={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        'pack_format 82 needs min_format and max_format, but pack.mcmeta is missing min_format and max_format.',
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'pack_format 88 needs min_format and max_format, but pack.mcmeta is missing max_format.',
      ),
    ).toBeInTheDocument();
    expect(screen.getByText('Built for 1.21. Pack may not match this world.')).toBeInTheDocument();
  });
});
