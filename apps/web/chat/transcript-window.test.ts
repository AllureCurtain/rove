import { describe, expect, it } from "vitest";

import {
  MOUNT_STEP,
  initialTranscriptWindow,
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
});
