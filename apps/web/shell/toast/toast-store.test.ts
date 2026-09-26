import { describe, expect, it } from "vitest";

import {
  EMPTY_TOAST_STATE,
  TOAST_DEDUPE_WINDOW_MS,
  TOAST_DURATION_MS,
  TOAST_FAILURE_HISTORY_LIMIT,
  TOAST_MAX_VISIBLE,
  createToastEntry,
  readToastMotion,
  reduceToasts,
  toastRemainingMs,
  toastRole,
  type ToastEvent,
  type ToastInput,
  type ToastState,
} from "./toast-store";

const T0 = 1_000_000;

function notify(
  state: ToastState,
  input: ToastInput,
  id: string,
  now = T0,
): ToastState {
  return reduceToasts(state, { type: "notify", toast: createToastEntry(input, id, now) });
}

function run(events: ToastEvent[], state: ToastState = EMPTY_TOAST_STATE): ToastState {
  return events.reduce(reduceToasts, state);
}

describe("toast queue", () => {
  it("keeps at most three toasts and drops the oldest", () => {
    let state = EMPTY_TOAST_STATE;
    for (const id of ["a", "b", "c", "d"]) {
      state = notify(state, { kind: "info", message: id }, id);
    }
    expect(state.visible.map((toast) => toast.id)).toEqual(["b", "c", "d"]);
    expect(state.visible.length).toBe(TOAST_MAX_VISIBLE);
  });

  it("gives success four seconds and errors eight", () => {
    const state = run([
      { type: "notify", toast: createToastEntry({ kind: "success", message: "saved" }, "a", T0) },
      { type: "notify", toast: createToastEntry({ kind: "error", message: "failed" }, "b", T0) },
    ]);
    expect(state.visible.map((toast) => toast.expiresAt)).toEqual([
      T0 + TOAST_DURATION_MS.success,
      T0 + TOAST_DURATION_MS.error,
    ]);
    expect(toastRemainingMs(state.visible[0]!, T0 + 1_000)).toBe(
      TOAST_DURATION_MS.success - 1_000,
    );
  });

  it("expires a toast at its deadline and ignores a timer that fires early", () => {
    const state = notify(EMPTY_TOAST_STATE, { kind: "success", message: "saved" }, "a");
    expect(run([{ type: "expire", id: "a", now: T0 + 500 }], state)).toBe(state);
    expect(
      run([{ type: "expire", id: "a", now: T0 + TOAST_DURATION_MS.success }], state).visible,
    ).toEqual([]);
  });

  it("dismisses on demand", () => {
    const state = run([
      { type: "notify", toast: createToastEntry({ kind: "info", message: "one" }, "a", T0) },
      { type: "notify", toast: createToastEntry({ kind: "info", message: "two" }, "b", T0) },
      { type: "dismiss", id: "a" },
    ]);
    expect(state.visible.map((toast) => toast.id)).toEqual(["b"]);
  });

  it("says the same thing once inside the dedupe window", () => {
    let state = notify(
      EMPTY_TOAST_STATE,
      { kind: "error", message: "Session A failed", dedupeKey: "session-a" },
      "a",
    );
    state = notify(
      state,
      { kind: "error", message: "Session A failed", dedupeKey: "session-a" },
      "b",
      T0 + TOAST_DEDUPE_WINDOW_MS - 1,
    );
    expect(state.visible.map((toast) => toast.id)).toEqual(["a"]);

    // After the window the news is worth repeating.
    state = notify(
      state,
      { kind: "error", message: "Session A failed", dedupeKey: "session-a" },
      "c",
      T0 + TOAST_DEDUPE_WINDOW_MS,
    );
    expect(state.visible.map((toast) => toast.id)).toEqual(["a", "c"]);
  });

  it("dedupes across a toast the reader already dismissed", () => {
    let state = notify(
      EMPTY_TOAST_STATE,
      { kind: "error", message: "Session A failed", dedupeKey: "session-a" },
      "a",
    );
    state = reduceToasts(state, { type: "dismiss", id: "a" });
    state = notify(
      state,
      { kind: "error", message: "Session A failed", dedupeKey: "session-a" },
      "b",
      T0 + 1_000,
    );
    expect(state.visible).toEqual([]);
  });

  it("keeps notifications without a key independent", () => {
    let state = notify(EMPTY_TOAST_STATE, { kind: "info", message: "one" }, "a");
    state = notify(state, { kind: "info", message: "one" }, "b", T0 + 1);
    expect(state.visible.map((toast) => toast.id)).toEqual(["a", "b"]);
  });

  it("remembers errors for the inbox while other kinds stay transient", () => {
    let state = notify(EMPTY_TOAST_STATE, { kind: "error", message: "failed" }, "a");
    state = notify(state, { kind: "success", message: "saved" }, "b", T0 + 1);
    expect(state.failures.map((toast) => toast.id)).toEqual(["a"]);

    // Dismissing a toast does not erase the failure record.
    state = reduceToasts(state, { type: "dismiss", id: "a" });
    expect(state.failures.map((toast) => toast.id)).toEqual(["a"]);
  });

  it("bounds the failure history", () => {
    let state = EMPTY_TOAST_STATE;
    for (let index = 0; index < TOAST_FAILURE_HISTORY_LIMIT + 5; index += 1) {
      state = notify(
        state,
        { kind: "error", message: `failure ${index}` },
        `f${index}`,
        // Outside the dedupe window, and with no key there is nothing to dedupe.
        T0 + index,
      );
    }
    expect(state.failures.length).toBe(TOAST_FAILURE_HISTORY_LIMIT);
    expect(state.failures.at(-1)?.id).toBe(`f${TOAST_FAILURE_HISTORY_LIMIT + 4}`);
  });

  it("pauses every timer while the stack is hovered and shifts the deadlines", () => {
    let state = notify(EMPTY_TOAST_STATE, { kind: "success", message: "saved" }, "a");
    const deadline = state.visible[0]!.expiresAt;

    state = reduceToasts(state, { type: "pause", now: T0 + 1_000 });
    // A timer that still fires during the hover cannot expire the toast.
    state = reduceToasts(state, { type: "expire", id: "a", now: T0 + TOAST_DURATION_MS.success });
    expect(state.visible.map((toast) => toast.id)).toEqual(["a"]);

    state = reduceToasts(state, { type: "resume", now: T0 + 3_000 });
    expect(state.visible[0]!.expiresAt).toBe(deadline + 2_000);
    expect(state.pausedAt).toBeNull();
  });

  it("ignores a repeated pause while already paused", () => {
    const paused = reduceToasts(EMPTY_TOAST_STATE, { type: "pause", now: T0 });
    expect(reduceToasts(paused, { type: "pause", now: T0 + 500 })).toBe(paused);
    expect(reduceToasts(paused, { type: "resume", now: T0 + 500 }).pausedAt).toBeNull();
  });
});

describe("toast presentation", () => {
  it("maps errors to alert and everything else to status", () => {
    expect(toastRole("error")).toBe("alert");
    expect(toastRole("success")).toBe("status");
    expect(toastRole("info")).toBe("status");
  });

  it("turns animation off when the reader asked for reduced motion", () => {
    const asked: string[] = [];
    expect(
      readToastMotion((query) => {
        asked.push(query);
        return { matches: true };
      }),
    ).toBe("reduced");
    expect(asked).toEqual(["(prefers-reduced-motion: reduce)"]);
    expect(readToastMotion(() => ({ matches: false }))).toBe("full");
    expect(readToastMotion(undefined)).toBe("full");
  });
});
