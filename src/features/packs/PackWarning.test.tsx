import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { PackWarning } from '../../lib/types';
import { pack } from '../../test/fake-packs';
import PackCard from './PackCard';
import PackDetail from './PackDetail';

const warning = {
  code: 'missing_min_max_format',
  message:
    'pack_format 82 needs min_format and max_format, but pack.mcmeta is missing min_format and max_format.',
} satisfies PackWarning;

describe('pack format warnings', () => {
  it('marks the card with a warning chip while the pack stays readable', () => {
    const flagged = pack('Loose', { kind: 'data', warnings: [warning] });
    render(<PackCard pack={flagged} selected={false} onSelect={vi.fn()} />);
    const card = screen.getByRole('button', { name: /^Loose,/ });
    expect(within(card).getByText('Warning')).toBeInTheDocument();
    expect(card).not.toHaveClass('is-broken');
    expect(within(card).getByText('A synthetic pack')).toBeInTheDocument();
  });

  it('shows the warning text in the detail panel', () => {
    const flagged = pack('Loose', { warnings: [warning] });
    render(<PackDetail pack={flagged} busy={false} writesBlocked={false}
      enabledInStack={false} onSetEnabled={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByText(warning.message)).toBeInTheDocument();
  });

  it('does not block the stack toggle on a warned resource pack', () => {
    const flagged = pack('Loose', { warnings: [warning] });
    render(<PackDetail pack={flagged} busy={false} writesBlocked={false}
      enabledInStack={false} onSetEnabled={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByRole('checkbox', { name: 'Enabled in the active stack' })).toBeEnabled();
  });

  it('shows no warning chip or note for a clean pack', () => {
    const clean = pack('Clean');
    render(<PackCard pack={clean} selected={false} onSelect={vi.fn()} />);
    expect(screen.queryByText('Warning')).not.toBeInTheDocument();
  });
});
