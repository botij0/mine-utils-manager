import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import App from './App';
import type { IpcClient } from './lib/ipc';
import { makeFakeClient } from './test/fake-ipc';

describe('App', () => {
  it('shows the startup warning and the shell', async () => {
    const client = makeFakeClient([]);
    const withWarning: IpcClient = {
      ...client,
      getStartupWarning: async () => 'The config file could not be read.',
    };

    render(<App client={withWarning} />);

    expect(await screen.findByRole('status')).toHaveTextContent(
      'The config file could not be read.',
    );
    expect(screen.getByText('Mine Utils')).toBeInTheDocument();
  });
});
