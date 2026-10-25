import { useMemo } from 'react';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from '@dnd-kit/core';
import type { DragEndEvent } from '@dnd-kit/core';
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import type {
  GameRunning,
  Pack,
  ResourceStack,
  StackEntry as StackEntryData,
} from '../../lib/types';
import GameRunningBanner from './GameRunningBanner';
import StackEntryCard from './StackEntry';

interface ActiveStackProps {
  stack: ResourceStack;
  packs: Pack[];
  busy: boolean;
  /** The live game guard state; the app polls it, so the banner can appear
   *  while the page is open. */
  running: GameRunning | null;
  onToggle: (filename: string, enabled: boolean) => void;
  onReorder: (order: string[]) => void;
  onRecheck: () => void;
}

function fileKey(entry: StackEntryData): string {
  return entry.kind === 'file' ? entry.filename : entry.value;
}

/**
 * Turn a dragged UI list back into load order. Foreign entries keep their
 * indices; the file entries take the remaining slots in their new order. The
 * UI shows the reverse of the array, so the top card is the highest priority.
 */
function toLoadOrder(
  movedUi: StackEntryData[],
  current: StackEntryData[],
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

function packByFilename(packs: Pack[]): Map<string, Pack> {
  const byName = new Map<string, Pack>();
  for (const pack of packs) {
    const filename = pack.path.split('/').filter(Boolean).at(-1) ?? pack.path;
    byName.set(filename, pack);
  }
  return byName;
}

/** The enabled stack, top first: the top card loads last and wins. */
export default function ActiveStack({
  stack,
  packs,
  busy,
  running,
  onToggle,
  onReorder,
  onRecheck,
}: ActiveStackProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  // Top first: the reverse of the load order array.
  const uiEntries = useMemo(() => [...stack.entries].reverse(), [stack.entries]);
  const ids = useMemo(
    () => uiEntries.map((entry, index) => `entry-${index}-${fileKey(entry)}`),
    [uiEntries],
  );
  const byFilename = useMemo(() => packByFilename(packs), [packs]);
  const blocked = busy || running !== null;

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const from = ids.indexOf(String(active.id));
    const to = ids.indexOf(String(over.id));
    if (from < 0 || to < 0) return;
    onReorder(toLoadOrder(arrayMove(uiEntries, from, to), stack.entries));
  };

  const handleMove = (index: number, direction: -1 | 1) => {
    let target = index + direction;
    while (target >= 0 && target < uiEntries.length) {
      if (uiEntries[target].kind === 'file') break;
      target += direction;
    }
    if (target < 0 || target >= uiEntries.length) return;
    onReorder(toLoadOrder(arrayMove(uiEntries, index, target), stack.entries));
  };

  return (
    <section className="active-stack" aria-label="Active stack">
      <div className="active-stack-head">
        <h2>Active stack</h2>
        <p className="active-stack-hint">
          Top card loads last and wins. Locked entries cannot move.
        </p>
      </div>

      <GameRunningBanner
        running={running}
        staleLocks={stack.stale_locks}
        supported={stack.detection_supported}
        onRecheck={onRecheck}
      />

      {uiEntries.length === 0 ? (
        <p className="active-stack-empty">No packs are enabled.</p>
      ) : (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          onDragEnd={handleDragEnd}
        >
          <SortableContext items={ids} strategy={verticalListSortingStrategy}>
            <ul className="stack-list" aria-label="Stack order">
              {uiEntries.map((entry, index) => (
                <StackEntryCard
                  key={ids[index]}
                  id={ids[index]}
                  entry={entry}
                  pack={entry.kind === 'file' ? (byFilename.get(entry.filename) ?? null) : null}
                  disabled={blocked}
                  canMoveUp={!blocked && uiEntries.slice(0, index).some((item) => item.kind === 'file')}
                  canMoveDown={!blocked && uiEntries.slice(index + 1).some((item) => item.kind === 'file')}
                  onToggle={(enabled) =>
                    entry.kind === 'file' && onToggle(entry.filename, enabled)
                  }
                  onMove={(direction) => handleMove(index, direction)}
                />
              ))}
            </ul>
          </SortableContext>
        </DndContext>
      )}
    </section>
  );
}
