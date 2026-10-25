import type { StackEntry } from '../../lib/types';

/**
 * Turn a dragged UI list back into load order. Foreign entries keep their
 * indices; the file entries take the remaining slots in their new order. The
 * UI is top-first, so the moved file order is reversed before it is placed.
 */
export function toLoadOrder(
  movedUi: StackEntry[],
  current: StackEntry[],
): string[] {
  const slots = current
    .map((entry, index) => (entry.kind === 'file' ? index : -1))
    .filter((index) => index >= 0);
  const filesInLoadOrder = movedUi
    .slice()
    .reverse()
    .filter((entry) => entry.kind === 'file');
  const order = current.map((entry) => entry.value);
  slots.forEach((slot, index) => {
    order[slot] = filesInLoadOrder[index].value;
  });
  return order;
}
