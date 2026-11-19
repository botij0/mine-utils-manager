import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, it, vi } from 'vitest';
import App from './App';
import { gameDir, makeFakeClient } from './test/fake-ipc';
import { datapackListing, world } from './test/fake-packs';

afterEach(() => { window.location.hash = ''; });

const OPTIONS_BACKUP = {
  file_name: '20261025T090000.000000000-options.txt',
  bytes: 4096,
  display: '2026-10-25 09:00:00 UTC',
};

const LEVEL_BACKUP = {
  file_name: '20261025T090000.000000000-Survival-level.dat',
  world: 'Survival',
  bytes: 2048,
  display: '2026-10-25 09:00:00 UTC',
};

async function restoreClient() {
  const client = makeFakeClient({
    gameDirs: [gameDir('/home/demo/.minecraft', true, true)],
    backups: [OPTIONS_BACKUP],
    levelBackups: [LEVEL_BACKUP],
  });
  await client.setActiveGameDir('/home/demo/.minecraft');
  client.listWorlds = async () => [world()];
  client.listDatapacks = async () => datapackListing(['a.zip']);
  return client;
}

it('reloads Packs after an options backup is restored in Settings', async () => {
  const user = userEvent.setup();
  const client = await restoreClient();
  const listPacks = vi.spyOn(client, 'listResourcePacks');
  render(<App client={client} />);
  await screen.findByLabelText('Search packs');
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  const options = await screen.findByRole('region', { name: 'Options backups' });
  const restore = () => within(options).getAllByRole('button', { name: /^Restore / })[0];
  await waitFor(() => expect(restore()).toBeEnabled());
  const before = listPacks.mock.calls.length;
  await user.click(restore());
  await screen.findByText(/^Restored 20261025T090000\.000000000-options\.txt\./);
  await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
  await waitFor(() => expect(listPacks).toHaveBeenCalledTimes(before + 1));
});

it('reloads Worlds after a level backup is restored in Settings', async () => {
  const user = userEvent.setup();
  const client = await restoreClient();
  const listDatapacks = vi.spyOn(client, 'listDatapacks');
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Worlds' }));
  await screen.findByRole('list', { name: 'Datapack stack' });
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  const group = await screen.findByRole('region', { name: 'Survival backups' });
  await waitFor(() => expect(within(group).getByRole('button', { name: /^Restore / })).toBeEnabled());
  const before = listDatapacks.mock.calls.length;
  await user.click(within(group).getByRole('button', { name: /^Restore / }));
  await screen.findByText(/^Restored .* for Survival\./);
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  await waitFor(() => expect(listDatapacks).toHaveBeenCalledTimes(before + 1));
});
