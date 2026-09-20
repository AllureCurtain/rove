import { describe, expect, it, vi } from "vitest";

import { createComposerDraftStore } from "./composer-draft-store";

const first = { workspaceId: "workspace-1", productSessionId: "session-1" };
const second = { ...first, productSessionId: "session-2" };

function deferred() {
  let resolve!: (accepted: boolean) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<boolean>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

describe("composer draft store", () => {
  it("isolates both identity fields, preserves whitespace, and has no shared singleton", () => {
    const store = createComposerDraftStore();
    const otherWorkspace = { ...first, workspaceId: "workspace-2" };
    store.setText(first, "  original\n草稿  ");
    store.setText(second, "second");
    store.setText(otherWorkspace, "other workspace");
    expect(store.getSnapshot({ ...first }).text).toBe("  original\n草稿  ");
    expect(store.getSnapshot(second).text).toBe("second");
    expect(store.getSnapshot(otherWorkspace).text).toBe("other workspace");
    expect(createComposerDraftStore().getSnapshot(first).text).toBe("");
    // Delimiters inside identifiers must not alias another pair.
    const left = { workspaceId: "a\u0000b", productSessionId: "c" };
    const right = { workspaceId: "a", productSessionId: "b\u0000c" };
    store.setText(left, "left");
    store.setText(right, "right");
    expect(store.getSnapshot(left).text).toBe("left");
  });

  it("publishes stable immutable snapshots and advances versions on every edit", () => {
    const store = createComposerDraftStore();
    const initial = store.getSnapshot(first);
    expect(store.getSnapshot(first)).toBe(initial);
    store.setText(first, "A");
    const a = store.getSnapshot(first);
    store.setText(first, "A");
    const sameText = store.getSnapshot(first);
    store.setText(first, "");
    expect(Object.isFrozen(a)).toBe(true);
    expect(a.text).toBe("A");
    expect(initial.text).toBe("");
    expect(a.version).toBeGreaterThan(initial.version);
    expect(sameText.version).toBeGreaterThan(a.version);
    expect(store.getSnapshot(first).version).toBeGreaterThan(sameText.version);
  });

  it("does not evict unsent drafts when more identities are visited", () => {
    const store = createComposerDraftStore();
    store.setText(first, "keep me");
    for (let index = 0; index < 1000; index += 1) {
      store.setText({ ...first, productSessionId: `extra-${index}` }, "draft");
    }
    expect(store.getSnapshot(first).text).toBe("keep me");
  });

  it("clears only the original identity after acceptance and sends trimmed text", async () => {
    const store = createComposerDraftStore();
    const pending = deferred();
    const send = vi.fn(() => pending.promise);
    const mutableIdentity = { ...first };
    store.setText(first, "  first\nmessage  ");
    const version = store.getSnapshot(first).version;
    const result = store.submit(mutableIdentity, send);
    mutableIdentity.productSessionId = second.productSessionId;
    store.setText(second, "new session text");
    pending.resolve(true);
    expect(await result).toBe(true);
    expect(send).toHaveBeenCalledExactlyOnceWith("first\nmessage");
    expect(store.getSnapshot(first)).toMatchObject({ text: "", submitting: false });
    expect(store.getSnapshot(first).version).toBeGreaterThan(version);
    expect(store.getSnapshot(second).text).toBe("new session text");
  });

  it.each(["new text", "A"])("never clears a newer version, including ABA: %s", async (replacement) => {
    const store = createComposerDraftStore();
    const pending = deferred();
    store.setText(first, "A");
    const original = store.getSnapshot(first);
    const result = store.submit(first, () => pending.promise);
    store.setText(first, "B");
    store.setText(first, replacement);
    const newer = store.getSnapshot(first);
    pending.resolve(true);
    await result;
    expect(store.getSnapshot(first)).toMatchObject({
      text: replacement, version: newer.version, submitting: false,
    });
    expect(newer.version).toBeGreaterThan(original.version);
  });

  it("locks synchronously across duplicate events, unsubscribe/remount, and edits", async () => {
    const store = createComposerDraftStore();
    const pending = deferred();
    const send = vi.fn(() => pending.promise);
    store.setText(first, "first");
    const unsubscribe = store.subscribe(vi.fn());
    const result = store.submit(first, send);
    expect(store.getSnapshot(first).submitting).toBe(true);
    expect(await store.submit(first, send)).toBe(false);
    unsubscribe(); // Navigation unmounts the composer, not the layout store.
    const remountedListener = vi.fn();
    const unsubscribeRemount = store.subscribe(remountedListener);
    store.setText(first, "edited while pending");
    expect(await store.submit({ ...first }, send)).toBe(false);
    // Another identity is not locked by this request.
    store.setText(second, "second");
    expect(await store.submit(second, () => true)).toBe(true);
    pending.resolve(true);
    await result;
    expect(send).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot(first)).toMatchObject({
      text: "edited while pending", submitting: false,
    });
    expect(remountedListener).toHaveBeenCalled();
    unsubscribeRemount();
    // No queued resend; another explicit action is required.
    expect(await store.submit(first, () => true)).toBe(true);
  });

  it("preserves text/version on false, releases the lock, and allows explicit retry", async () => {
    const store = createComposerDraftStore();
    store.setText(first, "  retained  ");
    const original = store.getSnapshot(first);
    expect(await store.submit(first, () => false)).toBe(false);
    expect(store.getSnapshot(first)).toEqual(original);
    expect(await store.submit(first, () => true)).toBe(true);
    expect(store.getSnapshot(first).text).toBe("");
  });

  it.each(["throw", "reject"])("handles unexpected %s without retaining details or resending", async (kind) => {
    const store = createComposerDraftStore();
    store.setText(first, "keep draft");
    const version = store.getSnapshot(first).version;
    const send = vi.fn(() => {
      if (kind === "throw") throw new Error("private details");
      return Promise.reject(new Error("private details"));
    });
    expect(await store.submit(first, send)).toBe(false);
    expect(send).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot(first)).toEqual({
      text: "keep draft", version, submitting: false, sendFailed: true,
    });
    expect(store.getSnapshot(second).sendFailed).toBe(false);
    expect(await store.submit(first, () => true)).toBe(true);
    expect(store.getSnapshot(first).sendFailed).toBe(false);
  });

  it("does not publish a stale failure onto a newer draft", async () => {
    const store = createComposerDraftStore();
    const pending = deferred();
    store.setText(first, "old");
    const result = store.submit(first, () => pending.promise);
    store.setText(first, "new");
    pending.reject(new Error("unexpected"));
    await result;
    expect(store.getSnapshot(first)).toMatchObject({
      text: "new", submitting: false, sendFailed: false,
    });
  });

  it("does not send whitespace-only drafts", async () => {
    const store = createComposerDraftStore();
    store.setText(first, " \n\t ");
    const original = store.getSnapshot(first);
    const send = vi.fn(() => true);
    expect(await store.submit(first, send)).toBe(false);
    expect(send).not.toHaveBeenCalled();
    expect(store.getSnapshot(first)).toBe(original);
  });
});
