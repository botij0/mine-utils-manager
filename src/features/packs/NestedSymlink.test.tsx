import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { PackIssue } from '../../lib/types';
import { pack } from '../../test/fake-packs';
import PackDetail from './PackDetail';
import DatapackDetail from '../worlds/DatapackDetail';
import PacksPage from './PacksPage';
import { makeFakeClient } from '../../test/fake-ipc';

const path = '/packs/Linked';
const error = {
  code: 'pack_symlink',
  message: '/packs/Linked/data/link is a symlink; pack sources must be real files and dirs',
  symlink_path: '/packs/Linked/data/link',
} satisfies PackIssue;
const nested = pack('Linked', {
  path, location: 'directory', kind: 'unknown', error,
});

describe('nested symlink pack details', () => {
  it('removes an enabled nested-folder reference without offering to enable it through Undo', async () => {
    const client = makeFakeClient({ packs: [nested], resource: ['vanilla', 'file/Linked'] });
    const readStack = client.getResourceStack.bind(client);
    client.getResourceStack = async () => {
      await new Promise((resolve) => setTimeout(resolve, 300));
      return readStack();
    };
    render(<PacksPage client={client} gameRunning={null} />);
    await userEvent.click(await screen.findByRole('button', { name: /^Linked,/ }));
    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'Enabled in the active stack' })).toBeEnabled(),
    );
    await userEvent.click(screen.getByRole('checkbox', { name: 'Enabled in the active stack' }));
    await waitFor(() => {
      expect(screen.getByText('Linked disabled')).toBeInTheDocument();
      expect(screen.getByRole('checkbox', { name: 'Enabled in the active stack' })).toBeDisabled();
      expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
    });
    expect((await client.getResourceStack()).entries).toEqual([{ kind: 'foreign', value: 'vanilla' }]);
  });

  it('lets an enabled resource folder be disabled and trashed', async () => {
    const toggle = vi.fn();
    const remove = vi.fn().mockResolvedValue(undefined);
    render(<PackDetail pack={nested} busy={false} writesBlocked={false}
      enabledInStack onSetEnabled={toggle} onDelete={remove} />);
    expect(screen.getByText(/Remove the nested link/)).toHaveTextContent(error.symlink_path);
    expect(screen.getByText(/whole pack folder/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Enabled in the active stack' }));
    expect(toggle).toHaveBeenCalledWith(false);
    await userEvent.click(screen.getByRole('button', { name: 'Delete pack' }));
    await userEvent.click(screen.getByRole('button', { name: 'Move to trash' }));
    expect(remove).toHaveBeenCalledWith(nested);
  });

  it('does not enable an unreadable resource folder', () => {
    render(<PackDetail pack={nested} busy={false} writesBlocked={false}
      enabledInStack={false} onSetEnabled={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByRole('checkbox', { name: 'Enabled in the active stack' })).toBeDisabled();
  });

  it.each([true, false])('explains datapack folder deletion when enabled is %s', async (enabled) => {
    const toggle = vi.fn();
    const remove = vi.fn();
    render(<DatapackDetail pack={nested} filename="Linked" enabled={enabled}
      locked={false} missing={false} warning={null} writesBlocked={false} busy={false}
      onToggle={toggle} onDelete={remove} onRemoveReference={vi.fn()} />);
    expect(screen.getByText(/Remove the nested link/)).toHaveTextContent(error.symlink_path);
    expect(screen.getByText(/whole pack folder/)).toBeInTheDocument();
    if (enabled) {
      await userEvent.click(screen.getByRole('button', { name: 'Disable' }));
      expect(toggle).toHaveBeenCalledWith('Linked', false);
    } else {
      expect(screen.getByRole('button', { name: 'Enable' })).toBeDisabled();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(remove).toHaveBeenCalledWith(path);
  });

  it('keeps the game guard on nested-folder actions', () => {
    render(<PackDetail pack={nested} busy={false} writesBlocked
      enabledInStack onSetEnabled={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByRole('checkbox', { name: 'Enabled in the active stack' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Delete pack' })).toBeDisabled();
  });
});
