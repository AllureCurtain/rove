import { afterEach, describe, expect, it, vi } from "vitest";

import { createFrameBatcher } from "./frame-batcher";

/**
 * The batcher's whole contract is "one flush per frame", so the frame clock is a
 * stub the test drives by hand rather than real time.
 */
function stubFrameClock() {
  const callbacks: FrameRequestCallback[] = [];
  let nextHandle = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callbacks.push(callback);
    nextHandle += 1;
    return nextHandle;
  });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  return {
    pending: () => callbacks.length,
    runFrame: () => {
      const scheduled = callbacks.splice(0, callbacks.length);
      for (const callback of scheduled) {
        callback(0);
      }
    },
  };
}

describe("createFrameBatcher", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("coalesces a burst on one key into a single flush", () => {
    const clock = stubFrameClock();
    const flushes: string[][] = [];
    const batcher = createFrameBatcher<string>((values) => flushes.push([...values]));

    batcher.enqueue("run-1", "first");
    batcher.enqueue("run-1", "second");
    batcher.enqueue("run-1", "third");

    expect(flushes).toEqual([]);
    expect(clock.pending()).toBe(1);
    expect(batcher.size).toBe(1);

    clock.runFrame();

    expect(flushes).toEqual([["third"]]);
    expect(batcher.size).toBe(0);
  });

  it("keeps insertion order between different keys", () => {
    const clock = stubFrameClock();
    const flushes: string[][] = [];
    const batcher = createFrameBatcher<string>((values) => flushes.push([...values]));

    batcher.enqueue("a", "a-1");
    batcher.enqueue("b", "b-1");
    batcher.enqueue("a", "a-2");
    clock.runFrame();

    expect(flushes).toEqual([["a-2", "b-1"]]);
  });

  it("folds a burst through the merger", () => {
    const clock = stubFrameClock();
    const flushes: string[][] = [];
    const batcher = createFrameBatcher<readonly string[]>((values) =>
      flushes.push(values.flatMap((value) => [...value])),
    );

    batcher.enqueue("run-1", ["a"], (previous, next) => [...previous, ...next]);
    batcher.enqueue("run-1", ["b"], (previous, next) => [...previous, ...next]);
    batcher.enqueue("run-1", ["c"], (previous, next) => [...previous, ...next]);
    clock.runFrame();

    expect(flushes).toEqual([["a", "b", "c"]]);
  });

  it("flushes now without waiting for the frame", () => {
    const clock = stubFrameClock();
    const flushes: string[][] = [];
    const batcher = createFrameBatcher<string>((values) => flushes.push([...values]));

    batcher.enqueue("run-1", "completed");
    expect(batcher.size).toBe(1);

    batcher.flushNow();

    expect(flushes).toEqual([["completed"]]);
    expect(batcher.size).toBe(0);
    // The scheduled frame was cancelled, so driving the clock again is a no-op.
    clock.runFrame();
    expect(flushes).toEqual([["completed"]]);
  });

  it("does not flush an empty batch", () => {
    const clock = stubFrameClock();
    const flush = vi.fn();
    const batcher = createFrameBatcher<string>(flush);

    clock.runFrame();

    expect(flush).not.toHaveBeenCalled();
    expect(batcher.size).toBe(0);
  });

  it("falls back to a timer where no frame clock exists", () => {
    vi.useFakeTimers();
    vi.stubGlobal("requestAnimationFrame", undefined);
    const flush = vi.fn();
    const batcher = createFrameBatcher<string>(flush);

    batcher.enqueue("run-1", "value");
    vi.advanceTimersByTime(20);

    expect(flush).toHaveBeenCalledWith(["value"]);
  });
});
