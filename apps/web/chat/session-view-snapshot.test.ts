import { beforeEach, describe, expect, it, vi } from "vitest";

import type { TranscriptRunGroup } from "../lib/rove-state";
import {
  SESSION_VIEW_SNAPSHOT_CAPACITY,
  captureActiveSessionView,
  createSessionViewSnapshotStore,
  registerSessionViewCapture,
  restoreSessionView,
  transcriptFingerprint,
  type SessionViewCapture,
} from "./session-view-snapshot";

function group(
  runOrdinal: number | null,
  runId: string | null,
  eventSeqs: Array<number | null>,
): TranscriptRunGroup {
  return {
    id: `run:${runId ?? "current"}`,
    runId,
    runOrdinal,
    inherited: false,
    sourceSessionId: null,
    items: eventSeqs.map((eventSeq, index) => ({
      kind: "message" as const,
      entry: {
        id: `entry-${index}`,
        kind: "message" as const,
        entityId: `message-${index}`,
        runId,
        runOrdinal,
        eventSeq,
        inherited: false,
        sourceSessionId: null,
      },
      message: {
        id: `message-${index}`,
        role: "assistant" as const,
        content: "",
        status: "final" as const,
      },
    })),
  };
}

function capture(overrides: Partial<SessionViewCapture> = {}): SessionViewCapture {
  return {
    scrollTop: 420,
    followPinned: false,
    window: { mounted: 12, loaded: 40 },
    disclosure: new Map([["activity-1", { open: true, userControlled: true }]]),
    ...overrides,
  };
}

describe("transcript fingerprint", () => {
  it("summarizes each run by identity and last event sequence", () => {
    expect(transcriptFingerprint([group(1, "run-1", [1, 4, 2])])).toBe("1:4");
    expect(
      transcriptFingerprint([group(1, "run-1", [4]), group(2, "run-2", [9])]),
    ).toBe("1:4|2:9");
  });

  it("changes when a run advances", () => {
    const before = transcriptFingerprint([group(1, "run-1", [4])]);
    const after = transcriptFingerprint([group(1, "run-1", [4, 5])]);
    expect(after).not.toBe(before);
  });

  it("changes when a run is appended", () => {
    const before = transcriptFingerprint([group(1, "run-1", [4])]);
    const after = transcriptFingerprint([
      group(1, "run-1", [4]),
      group(2, "run-2", [1]),
    ]);
    expect(after).not.toBe(before);
  });

  it("falls back to the run id when the ordinal is unknown", () => {
    expect(transcriptFingerprint([group(null, "run-7", [3])])).toBe("run-7:3");
    expect(transcriptFingerprint([group(null, null, [3])])).toBe("current:3");
  });

  it("ignores a missing event sequence instead of producing NaN", () => {
    expect(transcriptFingerprint([group(1, "run-1", [null, 2])])).toBe("1:2");
    expect(transcriptFingerprint([group(1, "run-1", [null])])).toBe("1:0");
  });
});

describe("restoreSessionView", () => {
  it("reinstate the captured view when the run set is unchanged", () => {
    const store = createSessionViewSnapshotStore();
    store.record("session-a", capture(), "1:4");
    const snapshot = store.peek("session-a")!;

    const restored = restoreSessionView(snapshot, "1:4");
    expect(restored.stale).toBe(false);
    expect(restored.scrollTop).toBe(420);
    expect(restored.followPinned).toBe(false);
    expect(restored.window).toEqual({ mounted: 12, loaded: 40 });
    expect(restored.disclosure.get("activity-1")).toEqual({
      open: true,
      userControlled: true,
    });
  });

  it("keeps disclosure but drops the viewport when the session advanced", () => {
    const snapshot = { ...capture(), fingerprint: "1:4" };

    const restored = restoreSessionView(snapshot, "1:9");
    expect(restored.stale).toBe(true);
    expect(restored.scrollTop).toBeNull();
    expect(restored.window).toBeNull();
    expect(restored.followPinned).toBe(false);
    expect(restored.disclosure.size).toBe(1);
  });

  it("hands out a copy of the disclosure map", () => {
    const snapshot = { ...capture(), fingerprint: "1:4" };
    const restored = restoreSessionView(snapshot, "1:4");
    restored.disclosure.set("activity-2", { open: false, userControlled: false });
    expect(snapshot.disclosure.has("activity-2")).toBe(false);
  });
});

describe("session view snapshot store", () => {
  it("consumes a snapshot at most once", () => {
    const store = createSessionViewSnapshotStore();
    store.record("session-a", capture(), "1:4");
    expect(store.peek("session-a")).not.toBeNull();
    expect(store.take("session-a")).not.toBeNull();
    expect(store.take("session-a")).toBeNull();
    expect(store.peek("session-a")).toBeNull();
  });

  it("evicts the least recently captured session past capacity", () => {
    const store = createSessionViewSnapshotStore(2);
    store.record("session-a", capture(), "1:1");
    store.record("session-b", capture(), "1:2");
    store.record("session-c", capture(), "1:3");

    expect(store.keys()).toEqual(["session-b", "session-c"]);
    expect(store.peek("session-a")).toBeNull();
  });

  it("re-recording a session moves it to the most recent position", () => {
    const store = createSessionViewSnapshotStore(2);
    store.record("session-a", capture(), "1:1");
    store.record("session-b", capture(), "1:2");
    store.record("session-a", capture({ scrollTop: 10 }), "1:3");
    store.record("session-c", capture(), "1:4");

    expect(store.keys()).toEqual(["session-a", "session-c"]);
    expect(store.peek("session-a")?.scrollTop).toBe(10);
  });

  it("copies the disclosure map at record time", () => {
    const store = createSessionViewSnapshotStore();
    const source = capture();
    store.record("session-a", source, "1:1");
    source.disclosure.set("activity-9", { open: true, userControlled: false });
    expect(store.peek("session-a")?.disclosure.has("activity-9")).toBe(false);
  });

  it("defaults to the documented capacity", () => {
    const store = createSessionViewSnapshotStore();
    for (let index = 0; index <= SESSION_VIEW_SNAPSHOT_CAPACITY; index += 1) {
      store.record(`session-${index}`, capture(), `1:${index}`);
    }
    expect(store.size()).toBe(SESSION_VIEW_SNAPSHOT_CAPACITY);
    expect(store.keys()[0]).toBe("session-1");
  });
});

describe("session view capture registry", () => {
  beforeEach(() => registerSessionViewCapture(null));

  it("calls the registered capture and tolerates none", () => {
    const capture = vi.fn();
    registerSessionViewCapture(capture);
    captureActiveSessionView();
    expect(capture).toHaveBeenCalledTimes(1);

    registerSessionViewCapture(null);
    captureActiveSessionView();
    expect(capture).toHaveBeenCalledTimes(1);
  });
});
