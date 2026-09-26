/**
 * Coalesces high-frequency updates until the next paint opportunity, ported from
 * PI-Desktop's `lib/frame-batcher.ts`.
 *
 * A key keeps only its latest value, so a burst for one target costs a single
 * flush per frame while insertion order between different targets is preserved.
 * The merger is where a burst is folded back into one value — appending to a list,
 * accumulating a counter — and `flushNow()` exists so a terminal value never
 * waits behind the frame.
 */

type FrameHandle = number;

export interface FrameBatcher<T> {
  enqueue(key: string, value: T, merge?: (previous: T, next: T) => T): void;
  flushNow(): void;
  readonly size: number;
}

export function createFrameBatcher<T>(
  flush: (values: readonly T[]) => void,
): FrameBatcher<T> {
  const pending = new Map<string, T>();
  let handle: FrameHandle | null = null;
  let usesAnimationFrame = false;

  const cancelScheduledFlush = () => {
    if (handle === null) {
      return;
    }
    if (usesAnimationFrame && typeof globalThis.cancelAnimationFrame === "function") {
      globalThis.cancelAnimationFrame(handle);
    } else {
      globalThis.clearTimeout(handle);
    }
    handle = null;
  };

  const run = () => {
    handle = null;
    if (pending.size === 0) {
      return;
    }
    const values = [...pending.values()];
    pending.clear();
    flush(values);
  };

  const schedule = () => {
    if (handle !== null) {
      return;
    }
    if (typeof globalThis.requestAnimationFrame === "function") {
      usesAnimationFrame = true;
      handle = globalThis.requestAnimationFrame(run);
      return;
    }
    // No frame clock (a test double, an old engine): a 16ms timer is the closest
    // equivalent, since the point is to coalesce, not to be exact.
    usesAnimationFrame = false;
    handle = globalThis.setTimeout(run, 16) as unknown as FrameHandle;
  };

  return {
    enqueue(key: string, value: T, merge?: (previous: T, next: T) => T) {
      const existing = pending.get(key);
      pending.set(key, existing !== undefined && merge ? merge(existing, value) : value);
      schedule();
    },
    flushNow() {
      cancelScheduledFlush();
      run();
    },
    get size() {
      return pending.size;
    },
  };
}
