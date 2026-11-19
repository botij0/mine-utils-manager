import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, it, vi } from 'vitest';
import App from './App';
import { IpcError } from './lib/types';
import { gameDir, makeFakeClient } from './test/fake-ipc';
import { datapackListing, pack, world } from './test/fake-packs';

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
  const readConfig = client.getConfig.bind(client);
  client.getConfig = async () => {
    await new Promise((resolve) => setTimeout(resolve, 250));
    return readConfig();
  };
  render(<App client={client} />);
  await screen.findByRole(
    'button',
    { name: /Active game dir\s*\/home\/demo\/\.minecraft/ },
    { timeout: 10_000 },
  );
  await user.click(screen.getByRole('button', { name: 'Discover' }));
  await user.click(screen.getByRole('tab', { name: 'Datapacks' }));
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'pure' } });
  await user.click(await screen.findByRole('button', { name: /^Pure Datapack,/ }, { timeout: 10_000 }));
  await user.click(await screen.findByRole('button', { name: 'Install…' }, { timeout: 10_000 }));
  expect(await screen.findByRole('option', { name: /Original world/ }, { timeout: 10_000 })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  await waitFor(() => {
    expect(screen.getByRole('list', { name: 'Datapack stack' })).toHaveTextContent('a');
    expect(screen.getByRole('button', { name: 'Import zip' })).toBeEnabled();
  }, { timeout: 10_000 });
  const before = { worlds: worlds.mock.calls.length, packs: packs.mock.calls.length, datapacks: datapacks.mock.calls.length, targets: targets.mock.calls.length };
  await user.click(screen.getByRole('button', { name: 'Import zip' }));
  await waitFor(() => {
    expect(screen.getByRole('list', { name: 'Datapack stack' })).toHaveTextContent('new');
    expect(worlds).toHaveBeenCalledTimes(before.worlds);
    expect(datapacks).toHaveBeenCalledTimes(before.datapacks + 1);
  }, { timeout: 10_000 });
  await user.click(screen.getByRole('button', { name: 'Discover' }));
  await waitFor(() => {
    expect(screen.getByRole('option', { name: /Updated world/ })).toBeInTheDocument();
    expect(targets).toHaveBeenCalledTimes(before.targets + 1);
    expect(worlds).toHaveBeenCalledTimes(before.worlds + 1);
  }, { timeout: 10_000 });
  await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
  await waitFor(() => {
    expect(screen.getByLabelText('Search packs')).toHaveValue('');
    expect(packs).toHaveBeenCalledTimes(before.packs);
  }, { timeout: 10_000 });
}, 30_000);

it('explains why an install is paused when targets cannot load', async () => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  client.modrinthTargets = async () => { throw new IpcError('io', 'target read failed'); };
  const readGuard = client.getGameGuard.bind(client);
  client.getGameGuard = async () => {
    await new Promise((resolve) => setTimeout(resolve, 2_000));
    return readGuard();
  };
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Discover' }));
  fireEvent.change(screen.getByLabelText('Search Modrinth'), { target: { value: 'fresh' } });
  await user.click(await screen.findByRole('button', { name: /^Fresh Animations,/ }));
  await user.click(await screen.findByRole('button', { name: 'Install…' }));
  await waitFor(() => {
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled();
    expect(screen.getByRole('dialog')).toHaveTextContent('Install targets are unavailable. Return to Discover to retry.');
  }, { timeout: 3_000 });
}, 10_000);

async function packsClient() {
  const client = makeFakeClient({
    gameDirs: [gameDir('/home/demo/.minecraft', true, true)],
    packs: [pack('Alpha')],
  });
  await client.setActiveGameDir('/home/demo/.minecraft');
  client.listWorlds = async () => [world()];
  return client;
}

it('retries a failed pack list on the next Packs activation', async () => {
  const user = userEvent.setup();
  const client = await packsClient();
  const read = client.listResourcePacks.bind(client);
  let fail = true;
  let calls = 0;
  client.listResourcePacks = async () => {
    calls += 1;
    if (fail) throw new IpcError('io', 'pack read failed');
    return read();
  };
  render(<App client={client} />);
  expect(await screen.findByText('pack read failed')).toBeInTheDocument();
  const failedCalls = calls;
  fail = false;
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
  await waitFor(() => expect(calls).toBe(failedCalls + 1));
  expect(await screen.findByRole('button', { name: /Alpha/ })).toBeInTheDocument();
  expect(screen.queryByText('pack read failed')).not.toBeInTheDocument();
});

it('retries a failed resource stack on the next Packs activation', async () => {
  const user = userEvent.setup();
  const client = await packsClient();
  const read = client.getResourceStack.bind(client);
  let fail = true;
  let calls = 0;
  client.getResourceStack = async () => {
    calls += 1;
    if (fail) throw new IpcError('options_txt', 'options.txt is unreadable');
    return read();
  };
  render(<App client={client} />);
  expect(await screen.findByText('options.txt is unreadable')).toBeInTheDocument();
  const failedCalls = calls;
  fail = false;
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
  await waitFor(() => expect(calls).toBe(failedCalls + 1));
  expect(await screen.findByRole('list', { name: 'Stack order' })).toHaveTextContent('vanilla');
  expect(screen.queryByText('options.txt is unreadable')).not.toBeInTheDocument();
});

it('does not scan worlds again when Worlds is activated during a load', async () => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  const rows = [world()];
  let release!: (value: typeof rows) => void;
  const pending = new Promise<typeof rows>((resolve) => {
    release = resolve;
  });
  const listWorlds = vi.spyOn(client, 'listWorlds').mockImplementation(() => pending);
  render(<App client={client} />);
  await screen.findByRole(
    'button',
    { name: /Active game dir\s*\/home\/demo\/\.minecraft/ },
  );
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  expect(
    within(await screen.findByRole('navigation', { name: 'Worlds' })).getByText('Loading worlds…'),
  ).toBeInTheDocument();
  expect(listWorlds).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  await user.click(screen.getByRole('button', { name: 'Resource Packs' }));
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  expect(listWorlds).toHaveBeenCalledTimes(1);
  expect(
    within(screen.getByRole('navigation', { name: 'Worlds' })).getByText('Loading worlds…'),
  ).toBeInTheDocument();
  await act(async () => {
    release(rows);
  });
  expect(await screen.findByRole('button', { name: /Survival 1\.17/ })).toHaveAttribute('aria-current', 'true');
  expect(listWorlds).toHaveBeenCalledTimes(1);
});

it('reloads Discover targets after a Worlds lock re-check', async () => {
  const user = userEvent.setup();
  const client = await clientWithWorld();
  const locked = world({
    world_locked: {
      kind: 'session_lock',
      detail: 'saves/Survival 1_17/session.lock is locked by pid 7',
      lock_path: 'session.lock',
      pid: 7,
      command: null,
    },
  });
  const versions: string[] = [];
  client.listWorlds = async () => [locked];
  client.modrinthTargets = async () => {
    const version = `9.9.${versions.length}`;
    versions.push(version);
    return {
      installed_versions: [version],
      preferred_launcher: version,
      world_version: null,
    };
  };
  render(<App client={client} />);
  await user.click(await screen.findByRole('button', { name: 'Discover' }));
  expect(await screen.findByRole('option', { name: '9.9.0' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Worlds' }));
  await user.click(await screen.findByRole('button', { name: 'Re-check' }));
  await user.click(screen.getByRole('button', { name: 'Discover' }));
  expect(await screen.findByRole('option', { name: '9.9.1' })).toBeInTheDocument();
  expect(screen.queryByRole('option', { name: '9.9.0' })).not.toBeInTheDocument();
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
