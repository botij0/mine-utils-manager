import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, it, vi } from 'vitest';
import App from './App';
import { IpcError } from './lib/types';
import { gameDir, makeFakeClient } from './test/fake-ipc';
import { datapackListing, world } from './test/fake-packs';

vi.mock('./lib/ipc', async (original) => ({
  ...await original<typeof import('./lib/ipc')>(),
  pickPackPath: async () => '/imports/new.zip',
}));

afterEach(() => { window.location.hash = ''; });

async function clientWithWorld() {
  const client = makeFakeClient([gameDir('/home/demo/.minecraft', true, true)]);
  await client.setActiveGameDir('/home/demo/.minecraft');
  client.listWorlds = async () => [world()];
  client.listDatapacks = async () => datapackListing(['a.zip']);
  return client;
}

it('keeps the manual version in the result sheet after an install', async () => {
  const user = userEvent.setup();
  render(<App client={await clientWithWorld()} />);
  await user.click(await screen.findByRole('button', { name: 'Discover' }));
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'fresh' } });
  await user.click(await screen.findByRole('button', { name: /^Fresh Animations,/ }));
  await user.selectOptions(await screen.findByLabelText(/^Version/), 'v1');
  await user.click(screen.getByRole('button', { name: 'Install…' }));
  await user.click(screen.getByRole('button', { name: 'Install' }));
  const sheet = await screen.findByRole('dialog', { name: 'Install Fresh Animations' });
  await within(sheet).findByText('Installed fresh-1.8 and enabled it.');
  await act(async () => {});
  expect(within(sheet).getByText('1.8')).toBeInTheDocument();
  expect(within(sheet).getByText('fresh-1.8.zip')).toBeInTheDocument();
});

it.each(['activation', 'Retry'])('recovers a failed world list via %s', async (retry) => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  vi.spyOn(client, 'listWorlds').mockRejectedValueOnce(new IpcError('io', 'world read failed'));
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Worlds' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('world read failed');
  if (retry === 'activation') {
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
  } else {
    await user.click(screen.getByRole('button', { name: 'Retry' }));
  }
  expect(await screen.findByRole('button', { name: /Survival 1.17/ })).toHaveAttribute('aria-current', 'true');
  expect(await screen.findByRole('list', { name: 'Datapack stack' })).toHaveTextContent('a');
});

it.each(['activation', 'Retry'])('recovers a failed datapack list via %s', async (retry) => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  vi.spyOn(client, 'listDatapacks').mockRejectedValueOnce(new IpcError('io', 'datapack read failed'));
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Worlds' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('datapack read failed');
  if (retry === 'activation') {
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Worlds' }));
  } else {
    await user.click(screen.getByRole('button', { name: 'Retry' }));
  }
  expect(await screen.findByRole('list', { name: 'Datapack stack' })).toHaveTextContent('a');
});

it('refreshes Discover after one Worlds import without rescanning Worlds or Packs', async () => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  let imported = false;
  client.listWorlds = async () => [world({ level_name: imported ? 'Updated world' : 'Original world' })];
  client.listDatapacks = async () => datapackListing([imported ? 'new.zip' : 'a.zip']);
  client.importDatapack = async () => { imported = true; return { status: 'imported', filename: 'new.zip' }; };
  const worlds = vi.spyOn(client, 'listWorlds');
  const packs = vi.spyOn(client, 'listResourcePacks');
  const datapacks = vi.spyOn(client, 'listDatapacks');
  const targets = vi.spyOn(client, 'modrinthTargets');
  render(<App client={client} />);
  await screen.findByLabelText('Search packs');
  await user.click(screen.getByRole('button', { name: 'Discover' }));
  await user.click(screen.getByRole('tab', { name: 'Datapacks' }));
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
  await user.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
  await user.click(await screen.findByRole('button', { name: 'Install…' }));
  expect(await screen.findByRole('option', { name: /Original world/ })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  await screen.findByRole('list', { name: 'Datapack stack' });
  const before = { worlds: worlds.mock.calls.length, packs: packs.mock.calls.length, datapacks: datapacks.mock.calls.length, targets: targets.mock.calls.length };
  await user.click(screen.getByRole('button', { name: 'Import zip' }));
  expect(await screen.findByRole('list', { name: 'Datapack stack' })).toHaveTextContent('new');
  await act(async () => {});
  expect(worlds).toHaveBeenCalledTimes(before.worlds);
  expect(datapacks).toHaveBeenCalledTimes(before.datapacks + 1);
  await user.click(screen.getByRole('button', { name: 'Discover' }));
  expect(await screen.findByRole('option', { name: /Updated world/ })).toBeInTheDocument();
  await waitFor(() => expect(targets).toHaveBeenCalledTimes(before.targets + 1));
  expect(worlds).toHaveBeenCalledTimes(before.worlds + 1);
  await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
  expect(await screen.findByLabelText('Search packs')).toHaveValue('');
  expect(packs).toHaveBeenCalledTimes(before.packs);
});

it('explains why an install is paused when targets cannot load', async () => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  client.modrinthTargets = async () => { throw new IpcError('io', 'target read failed'); };
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Discover' }));
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'fresh' } });
  await user.click(await screen.findByRole('button', { name: /^Fresh Animations,/ }));
  await user.click(await screen.findByRole('button', { name: 'Install…' }));
  expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled();
  expect(screen.getByRole('dialog')).toHaveTextContent('Install targets are unavailable. Return to Discover to retry.');
});

it('shows loading rather than an empty world list in the install sheet', async () => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  client.listWorlds = () => new Promise(() => {});
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Discover' }));
  await user.click(screen.getByRole('tab', { name: 'Datapacks' }));
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
  await user.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }));
  await user.click(await screen.findByRole('button', { name: 'Install…' }));
  expect(screen.getByRole('dialog')).toHaveTextContent('Loading worlds…');
  expect(screen.getByRole('dialog')).not.toHaveTextContent('No worlds yet.');
  expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled();
});
