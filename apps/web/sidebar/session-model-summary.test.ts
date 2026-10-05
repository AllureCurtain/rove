import { describe, expect, it, vi } from "vitest";

import {
  createSessionModelSummaryStore,
  type SessionModelSummary,
} from "./session-model-summary";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const READY: SessionModelSummary = {
  status: "ready",
  model: "gpt-5",
  reasoning: "high",
  maxSteps: 12,
};

describe("session model summary store", () => {
  it("requests a session once, no matter how often it is asked for", async () => {
    const loader = vi.fn(() => Promise.resolve(READY));
    const store = createSessionModelSummaryStore(loader);

    store.request("s1");
    store.request("s1");
    store.request("s1");
    await Promise.resolve();
    await Promise.resolve();

    expect(loader).toHaveBeenCalledTimes(1);
    expect(loader).toHaveBeenCalledWith("s1");
    expect(store.read("s1")).toEqual(READY);
  });

  it("reports loading until the answer lands", async () => {
    const pending = deferred<SessionModelSummary>();
    const store = createSessionModelSummaryStore(() => pending.promise);

    store.request("s1");
    expect(store.read("s1")).toEqual({ status: "loading" });

    pending.resolve(READY);
    await pending.promise;
    await Promise.resolve();
    expect(store.read("s1")).toEqual(READY);
  });

  it("remembers a failure instead of retrying it on every hover", async () => {
    const loader = vi.fn(() => Promise.reject(new Error("offline")));
    const store = createSessionModelSummaryStore(loader);

    store.request("s1");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(store.read("s1")).toEqual({ status: "unavailable" });
    store.request("s1");
    store.request("s1");
    expect(loader).toHaveBeenCalledTimes(1);
  });

  it("does not request a session that was never asked for", () => {
    const loader = vi.fn(() => Promise.resolve(READY));
    const store = createSessionModelSummaryStore(loader);
    expect(store.read("s1")).toBeNull();
    expect(loader).not.toHaveBeenCalled();
  });

  it("notifies subscribers about each state change", async () => {
    const pending = deferred<SessionModelSummary>();
    const store = createSessionModelSummaryStore(() => pending.promise);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    store.request("s1");
    expect(listener).toHaveBeenCalledTimes(1);

    pending.resolve(READY);
    await pending.promise;
    await Promise.resolve();
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
    store.request("s2");
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("keeps a slow answer on its own row", async () => {
    const slow = deferred<SessionModelSummary>();
    const store = createSessionModelSummaryStore((sessionId) =>
      sessionId === "slow" ? slow.promise : Promise.resolve(READY),
    );

    store.request("slow");
    store.request("fast");
    await Promise.resolve();
    expect(store.read("fast")).toEqual(READY);

    slow.resolve({ ...READY, model: "slow-model" });
    await slow.promise;
    await Promise.resolve();
    expect(store.read("slow")).toEqual({ ...READY, model: "slow-model" });
    expect(store.read("fast")).toEqual(READY);
  });

  it("bounds the cache so a long sidebar cannot grow it forever", async () => {
    const loader = vi.fn(() => Promise.resolve(READY));
    const store = createSessionModelSummaryStore(loader, 2);

    store.request("s1");
    store.request("s2");
    store.request("s3");
    expect(store.read("s1")).toBeNull();
    expect(store.read("s2")).not.toBeNull();
    expect(store.read("s3")).not.toBeNull();

    // An evicted row is requested again rather than reported as unknown.
    store.request("s1");
    expect(loader).toHaveBeenCalledTimes(4);
    expect(store.read("s1")).toEqual({ status: "loading" });
  });
});
