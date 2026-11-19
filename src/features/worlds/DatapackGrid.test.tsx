import { act, within } from '@testing-library/react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { describe, expect, it } from 'vitest';
import { datapack } from '../../test/fake-packs';
import DatapackGrid, { type DatapackRow } from './DatapackGrid';

function brokenRow(filename: string, message: string): DatapackRow {
  return {
    pack: datapack(filename, null, null, {
      error: { code: 'pack_broken_archive', message },
    }),
    entry: { kind: 'file', value: `file/${filename}`, filename },
    enabled: true,
    locked: false,
    missing: false,
    warning: null,
    filename,
    canMoveUp: false,
    canMoveDown: false,
  };
}

describe('DatapackGrid', () => {
  it('mounts the broken summary empty, then fills the status', async () => {
    const container = document.createElement('div');
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      flushSync(() => {
        root.render(
          <DatapackGrid
            rows={[brokenRow('broken.zip', 'broken.zip is not readable')]}
            loading={false}
            selectedFilename={null}
            onSelect={() => {}}
            writesBlocked={false}
            onToggle={() => {}}
            onMove={() => {}}
          />,
        );
      });
      const status = within(container).getByRole('status');
      expect(status.textContent).toBe('');
      await act(async () => {});
      expect(status.textContent).toBe('1 broken datapack');
    } finally {
      act(() => {
        root.unmount();
      });
      container.remove();
    }
  });
});
