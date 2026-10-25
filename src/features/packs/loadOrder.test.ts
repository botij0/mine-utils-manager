import { describe, expect, it } from 'vitest';
import type { StackEntry } from '../../lib/types';
import { toLoadOrder } from './loadOrder';

function entry(value: string): StackEntry {
  return value.startsWith('file/')
    ? { kind: 'file', value, filename: value.slice('file/'.length) }
    : { kind: 'foreign', value };
}

function stack(values: string[]): StackEntry[] {
  return values.map(entry);
}

/** UI is top-first: the last load-order entry comes first. */
function ui(values: string[]): StackEntry[] {
  return [...values].reverse().map(entry);
}

function stringify(values: StackEntry[]): string[] {
  return values.map((value) => value.value);
}

describe('toLoadOrder', () => {
  it('keeps foreign entries at their indices with files interleaved', () => {
    const current = stack(['vanilla', 'file/a.zip', 'fabric', 'file/b.zip']);
    // Drag b above a in the UI. Foreign slots stay put.
    const moved = [...ui(['vanilla', 'file/a.zip', 'fabric', 'file/b.zip'])];
    const b = moved.findIndex((item) => item.value === 'file/b.zip');
    const a = moved.findIndex((item) => item.value === 'file/a.zip');
    const [dragged] = moved.splice(b, 1);
    moved.splice(a, 0, dragged);

    expect(toLoadOrder(moved, current)).toEqual([
      'vanilla',
      'file/b.zip',
      'fabric',
      'file/a.zip',
    ]);
  });

  it('handles a drop onto a foreign slot without moving the foreign entry', () => {
    const current = stack(['file/a.zip', 'vanilla', 'file/b.zip']);
    const moved = [...ui(['file/a.zip', 'vanilla', 'file/b.zip'])];
    // UI order is [b, vanilla, a]; drag b down onto vanilla's slot.
    const [dragged] = moved.splice(0, 1);
    moved.splice(1, 0, dragged);

    expect(toLoadOrder(moved, current)).toEqual([
      'file/a.zip',
      'vanilla',
      'file/b.zip',
    ]);
  });

  it('keeps duplicate filenames distinct by position', () => {
    const current = stack(['vanilla', 'file/a.zip', 'file/a.zip', 'file/b.zip']);
    const moved = [...ui(['vanilla', 'file/a.zip', 'file/a.zip', 'file/b.zip'])];
    const [dragged] = moved.splice(0, 1);
    moved.push(dragged);

    expect(toLoadOrder(moved, current)).toEqual([
      'vanilla',
      'file/b.zip',
      'file/a.zip',
      'file/a.zip',
    ]);
  });

  it('returns an empty order for an empty stack', () => {
    expect(toLoadOrder([], [])).toEqual([]);
  });

  it('keeps a singleton stack unchanged', () => {
    const current = stack(['file/only.zip']);
    expect(toLoadOrder([...current], current)).toEqual(['file/only.zip']);
  });

  it('falls back to the current order when a move loses an entry', () => {
    const current = stack(['vanilla', 'file/a.zip', 'file/b.zip']);
    expect(toLoadOrder(ui(['vanilla', 'file/a.zip']), current)).toEqual(
      stringify(current),
    );
  });
});
