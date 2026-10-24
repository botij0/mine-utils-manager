import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import type { Config } from '../../lib/types';
import { defaultConfig } from '../../lib/types';
import SettingsPage from './SettingsPage';
import { gameDir, makeFakeClient } from '../../test/fake-ipc';
import type { FakeClient } from '../../test/fake-ipc';

function Harness({ client }: { client: FakeClient }) {
  const [config, setConfig] = useState<Config>(defaultConfig());
  return (
    <SettingsPage
      client={client}
      config={config}
      onConfigChanged={setConfig}
    />
  );
}

describe('SettingsPage', () => {
  it('keeps an added dir when the theme changes', async () => {
    const user = userEvent.setup();
    const client = makeFakeClient([
      gameDir('/home/demo/.minecraft', true, false),
    ]);
    render(<Harness client={client} />);

    await user.type(screen.getByLabelText('Game dir path'), '/tmp/saves/one');
    await user.click(screen.getByRole('button', { name: 'Add' }));
    await screen.findByText('/tmp/saves/one');

    await user.click(screen.getByRole('radio', { name: 'Dark' }));

    expect(client.config.theme).toBe('dark');
    expect(client.config.game_dirs.map((dir) => dir.path)).toContain(
      '/tmp/saves/one',
    );
  });
});
