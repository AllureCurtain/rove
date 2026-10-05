import { describe, expect, it } from "vitest";

import {
  MAX_MOUNTED,
  MOUNT_STEP,
  NO_OLDER_HISTORY,
  hiddenMountedCount,
  initialTranscriptWindow,
  mountedRunRange,
  mountsRun,
  olderHistoryControl,
  reduceTranscriptWindow,
  shouldRequestOlderPage,
} from "./transcript-window";

describe("transcript window", () => {
  it("mounts at most the initial slice of loaded runs", () => {
    expect(initialTranscriptWindow(3)).toEqual({ mounted: 3, loaded: 3 });
    expect(initialTranscriptWindow(40).mounted).toBe(12);
  });

  it("grow never mounts past what is loaded", () => {
    const window = initialTranscriptWindow(20);
    const grown = reduceTranscriptWindow(window, { type: "grow" });
    expect(grown.mounted).toBe(20);
  });

  it("grow can eventually mount everything loaded", () => {
    let window = initialTranscriptWindow(400);
    for (let index = 0; index < 30; index += 1) {
      window = reduceTranscriptWindow(window, { type: "grow" });
    }
    expect(window.mounted).toBe(400);
  });

  it("loaded pages raise the loaded layer", () => {
    const window = reduceTranscriptWindow(initialTranscriptWindow(12), {
      type: "loaded",
      count: 16,
    });
    expect(window).toEqual({ mounted: 12, loaded: 28 });
  });

  it("only proposes a server request when mounted reaches loaded with a cursor", () => {
    const partial = { mounted: 12, loaded: 28 };
    expect(shouldRequestOlderPage(partial, 40)).toBe(false);

    const drained = reduceTranscriptWindow(partial, { type: "grow" });
    expect(drained.mounted).toBe(28);
    expect(shouldRequestOlderPage(drained, 40)).toBe(true);
    expect(shouldRequestOlderPage(drained, null)).toBe(false);
    expect(shouldRequestOlderPage(drained, undefined)).toBe(false);
  });

  it("grow steps by the mount step", () => {
    const window = reduceTranscriptWindow(initialTranscriptWindow(100), { type: "grow" });
    expect(window.mounted).toBe(12 + MOUNT_STEP);
  });

  it("mount reaches a named run in one step, clamped to what is loaded", () => {
    // A reveal cannot walk the window one step at a time: the target may be far
    // outside it, and the reader asked for that message, not for a window size.
    const window = reduceTranscriptWindow(initialTranscriptWindow(40), {
      type: "mount",
      count: 35,
    });
    expect(window).toEqual({ mounted: 35, loaded: 40 });
  });

  it("mount never unmounts and never outruns the loaded layer", () => {
    const window = reduceTranscriptWindow({ mounted: 30, loaded: 40 }, {
      type: "mount",
      count: 12,
    });
    expect(window.mounted).toBe(30);

    const clamped = reduceTranscriptWindow({ mounted: 12, loaded: 20 }, {
      type: "mount",
      count: 64,
    });
    expect(clamped.mounted).toBe(20);
  });
});

describe("bounded mounted window", () => {
  it("renders at most the cap however far the reader walks up", () => {
    const total = 300;
    let window = initialTranscriptWindow(total);
    const rendered: number[] = [];
    for (let step = 0; step < 40; step += 1) {
      window = reduceTranscriptWindow(window, { type: "grow" });
      const range = mountedRunRange(window, total);
      rendered.push(range.end - range.start);
    }
    // The walk revealed every loaded run; the DOM never held more than the cap.
    expect(window.mounted).toBe(total);
    expect(Math.max(...rendered)).toBe(MAX_MOUNTED);
    expect(hiddenMountedCount(window)).toBe(0);
  });

  it("releases the newest runs instead of refusing to grow", () => {
    const total = 400;
    const before = { mounted: MAX_MOUNTED + MOUNT_STEP, loaded: total };
    const grown = reduceTranscriptWindow(before, { type: "grow" });
    const beforeRange = mountedRunRange(before, total);
    const afterRange = mountedRunRange(grown, total);
    expect(afterRange.end - afterRange.start).toBe(MAX_MOUNTED);
    // Both edges moved up: MOUNT_STEP older runs arrived and MOUNT_STEP newest
    // runs were released, which is what keeps the DOM bounded.
    expect(beforeRange.start - afterRange.start).toBe(MOUNT_STEP);
    expect(beforeRange.end - afterRange.end).toBe(MOUNT_STEP);
    // The run at the top of the old window is still rendered, so a step never
    // takes away the place the reader was reading.
    expect(afterRange.start).toBeLessThanOrEqual(beforeRange.start);
    expect(beforeRange.start).toBeLessThan(afterRange.end);
  });

  it("never renders more runs than the timeline holds", () => {
    expect(mountedRunRange({ mounted: 120, loaded: 10 }, 10)).toEqual({
      start: 0,
      end: 10,
    });
    expect(mountedRunRange(initialTranscriptWindow(0), 0)).toEqual({
      start: 0,
      end: 0,
    });
  });

  it("reveals a run the walk released, and leaves an on-screen one alone", () => {
    const total = 400;
    // The reader has walked 300 runs back. A window of the cap still shows a run
    // 250 runs back from the tail, but no longer one 200 runs back.
    const walked = { mounted: 300, loaded: total };
    expect(mountsRun(walked, 250)).toBe(true);
    expect(mountsRun(walked, 200)).toBe(false);

    const revealed = reduceTranscriptWindow(walked, { type: "mount", count: 200 });
    expect(mountsRun(revealed, 200)).toBe(true);
    const target = total - 200;
    const range = mountedRunRange(revealed, total);
    expect(target).toBeGreaterThanOrEqual(range.start);
    expect(target).toBeLessThan(range.end);

    // A target that is already on screen does not move the window: a reveal is
    // not a reason to release runs the reader is still looking at.
    expect(reduceTranscriptWindow(revealed, { type: "mount", count: 200 })).toBe(
      revealed,
    );
  });

  it("returns to the live tail only when the window had left it", () => {
    const walked = { mounted: 300, loaded: 400 };
    const back = reduceTranscriptWindow(walked, { type: "tail" });
    expect(back.mounted).toBe(MAX_MOUNTED);
    expect(mountedRunRange(back, 400).end).toBe(400);
    // Idempotent, and it allocates nothing when there is nothing to do: the
    // follow state asks for it every time follow is re-engaged.
    expect(reduceTranscriptWindow(back, { type: "tail" })).toBe(back);
  });

  it("keeps the older-history decisions reading the reveal budget", () => {
    // Everything loaded has been revealed, so the server page is the only thing
    // left — even though the DOM is holding one window of what was revealed.
    const walked = { mounted: 300, loaded: 300 };
    const range = mountedRunRange(walked, 300);
    expect(range.end - range.start).toBe(MAX_MOUNTED);
    expect(shouldRequestOlderPage(walked, 45)).toBe(true);
    expect(olderHistoryControl(walked, { hasMore: true, cursor: 45 })).toBe(
      "server",
    );

    // A page that arrived and has not been revealed yet still grows locally
    // first, with a cursor waiting.
    const paged = { mounted: 300, loaded: 364 };
    expect(hiddenMountedCount(paged)).toBe(64);
    expect(olderHistoryControl(paged, { hasMore: true, cursor: 45 })).toBe("grow");
  });
});

describe("older history control", () => {
  const drained = { mounted: 64, loaded: 64 };

  it("grows the local window first, even with a server cursor", () => {
    expect(
      olderHistoryControl({ mounted: 12, loaded: 64 }, { hasMore: true, cursor: 237 }),
    ).toBe("grow");
  });

  it("requests a server page once the window is drained and a cursor exists", () => {
    expect(olderHistoryControl(drained, { hasMore: true, cursor: 237 })).toBe(
      "server",
    );
  });

  it("offers nothing when the server published no more history", () => {
    expect(olderHistoryControl(drained, { hasMore: false, cursor: null })).toBe(
      "none",
    );
    // A legacy response has no pagination fields at all.
    expect(olderHistoryControl(drained, NO_OLDER_HISTORY)).toBe("none");
    // `has_more` without a cursor can never be fetched.
    expect(olderHistoryControl(drained, { hasMore: true, cursor: null })).toBe(
      "none",
    );
  });
});
