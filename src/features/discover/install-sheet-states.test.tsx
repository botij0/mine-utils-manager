import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { userEvent } from '@testing-library/user-event';
import DiscoverPage from './DiscoverPage';
import { IpcError } from '../../lib/types';
import type { ModrinthTargets, World } from '../../lib/types';
import { makeFakeClient } from '../../test/fake-ipc';
import { modrinthTargets, world } from '../../test/fake-packs';

const READY_TARGETS: ModrinthTargets = modrinthTargets();

function sheetClient() {
  return makeFakeClient([]);
}

async function openResourceSheet(client: ReturnType<typeof sheetClient>) {
  render(
    <DiscoverPage client={client} gameRunning={null} hasGameDir gameDir="/home/demo/.minecraft" />,
  );
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'fresh' } });
  fireEvent.click(await screen.findByRole('button', { name: /^Fresh Animations,/ }));
  fireEvent.click(await screen.findByRole('button', { name: 'Install…' }));
  return screen.getByRole('dialog', { name: 'Install Fresh Animations' });
}

describe('install sheet target states', () => {
  it('says targets are loading and does not call that a failure', async () => {
    const client = sheetClient();
    client.modrinthTargets = () => new Promise<ModrinthTargets>(() => {});
    const dialog = await openResourceSheet(client);
    expect(dialog).toHaveTextContent('Loading install targets…');
    expect(dialog).not.toHaveTextContent('Install targets are unavailable');
    expect(dialog).not.toHaveTextContent('Return to Discover');
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeDisabled();
  });

  it('offers an in-place Retry when targets fail to load', async () => {
    const user = userEvent.setup();
    const client = sheetClient();
    const calls: string[] = [];
    let fail = true;
    client.modrinthTargets = async () => {
      calls.push(fail ? 'failed' : 'ready');
      if (fail) throw new IpcError('io', 'target read failed');
      return READY_TARGETS;
    };
    const dialog = await openResourceSheet(client);
    expect(dialog).toHaveTextContent('Install targets are unavailable.');
    expect(dialog).not.toHaveTextContent('Return to Discover');
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeDisabled();
    fail = false;
    await user.click(within(dialog).getByRole('button', { name: 'Retry' }));
    await waitFor(() => {
      expect(dialog).not.toHaveTextContent('Install targets are unavailable');
      expect(within(dialog).getByRole('button', { name: 'Install' })).toBeEnabled();
    });
    expect(calls).toEqual(['failed', 'ready']);
  });

  it('shows the install action once targets have loaded', async () => {
    const client = sheetClient();
    client.modrinthTargets = async () => READY_TARGETS;
    const dialog = await openResourceSheet(client);
    expect(dialog).not.toHaveTextContent('Loading install targets');
    expect(dialog).not.toHaveTextContent('unavailable');
    expect(dialog).not.toHaveTextContent('Return to Discover');
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeEnabled();
  });
});

describe('install sheet worlds after a folder change', () => {
  it('says worlds are loading instead of showing an empty list', async () => {
    const client = sheetClient();
    const first: World[] = [world()];
    let calls = 0;
    client.listWorlds = () => {
      calls += 1;
      if (calls === 1) return Promise.resolve(first);
      return new Promise<World[]>(() => {});
    };
    const view = render(
      <DiscoverPage client={client} gameRunning={null} hasGameDir gameDir="/dir-a/.minecraft" />,
    );
    fireEvent.click(screen.getByRole('tab', { name: 'Datapacks' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
    fireEvent.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Install…' }));
    expect(await screen.findByRole('option', { name: /Survival 1.17/ })).toBeInTheDocument();
    view.rerender(
      <DiscoverPage client={client} gameRunning={null} hasGameDir gameDir="/dir-b/.minecraft" />,
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent('Loading worlds…');
    expect(dialog).not.toHaveTextContent('No worlds yet');
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeDisabled();
  });

  it('says worlds are loading when the previous folder had none', async () => {
    const client = sheetClient();
    let calls = 0;
    client.listWorlds = () => {
      calls += 1;
      if (calls === 1) return Promise.resolve([]);
      return new Promise<World[]>(() => {});
    };
    const view = render(
      <DiscoverPage client={client} gameRunning={null} hasGameDir gameDir="/dir-a/.minecraft" />,
    );
    fireEvent.click(screen.getByRole('tab', { name: 'Datapacks' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
    fireEvent.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Install…' }));
    expect(await screen.findByText(/No worlds yet/)).toBeInTheDocument();
    view.rerender(
      <DiscoverPage client={client} gameRunning={null} hasGameDir gameDir="/dir-b/.minecraft" />,
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent('Loading worlds…');
    expect(dialog).not.toHaveTextContent('No worlds yet');
  });

  it('does not describe a failed world read as an empty list', async () => {
    const user = userEvent.setup();
    const client = sheetClient();
    let fail = true;
    client.listWorlds = async () => {
      if (fail) throw new IpcError('io', 'world read failed');
      return [world()];
    };
    render(
      <DiscoverPage client={client} gameRunning={null} hasGameDir gameDir="/dir-a/.minecraft" />,
    );
    fireEvent.click(screen.getByRole('tab', { name: 'Datapacks' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
    fireEvent.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Install…' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('world read failed');
    expect(dialog).not.toHaveTextContent('No worlds yet');
    fail = false;
    await user.click(within(dialog).getByRole('button', { name: 'Retry' }));
    expect(await within(dialog).findByRole('option', { name: /Survival 1.17/ })).toBeInTheDocument();
    expect(dialog).not.toHaveTextContent('world read failed');
    expect(dialog).not.toHaveTextContent('No worlds yet');
  });

  it('says worlds are loading when the folder changes while Discover is hidden', async () => {
    const client = sheetClient();
    let calls = 0;
    client.listWorlds = () => {
      calls += 1;
      if (calls === 1) return Promise.resolve([]);
      return new Promise<World[]>(() => {});
    };
    const view = render(
      <DiscoverPage
        client={client}
        active
        gameRunning={null}
        hasGameDir
        gameDir="/dir-a/.minecraft"
      />,
    );
    fireEvent.click(screen.getByRole('tab', { name: 'Datapacks' }));
    fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
    fireEvent.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Install…' }));
    expect(await screen.findByText(/No worlds yet/)).toBeInTheDocument();
    view.rerender(
      <DiscoverPage
        client={client}
        active={false}
        gameRunning={null}
        hasGameDir
        gameDir="/dir-b/.minecraft"
      />,
    );
    view.rerender(
      <DiscoverPage
        client={client}
        active
        gameRunning={null}
        hasGameDir
        gameDir="/dir-b/.minecraft"
      />,
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent('Loading worlds…');
    expect(dialog).not.toHaveTextContent('No worlds yet');
  });
});
