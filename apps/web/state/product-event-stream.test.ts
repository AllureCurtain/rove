import { describe, expect, it, vi } from "vitest";

import { AuthorizedStreamErrorEvent, PRODUCT_EVENT_KINDS } from "../lib/rove-client";
import {
  decideProductStatusPoll,
  PRODUCT_STATUS_FALLBACK_POLL_MS,
  PRODUCT_STATUS_SAFETY_NET_POLL_MS,
  subscribeToProductEvents,
  type ProductEventStreamSource,
} from "./product-event-stream";

/** An EventSource stand-in: no DOM, no network, no timers. */
class FakeEventStream implements ProductEventStreamSource {
  readonly close = vi.fn();
  private readonly listeners = new Map<string, EventListener[]>();

  addEventListener(type: string, listener: EventListener): void {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  removeEventListener(type: string, listener: EventListener): void {
    this.listeners.set(
      type,
      (this.listeners.get(type) ?? []).filter((item) => item !== listener),
    );
  }

  emit(type: string, event?: Event): void {
    for (const listener of [...(this.listeners.get(type) ?? [])]) {
      listener(event ?? new Event(type));
    }
  }
}

function subscribe(stream: FakeEventStream) {
  const refresh = vi.fn();
  const availability: boolean[] = [];
  const detach = subscribeToProductEvents(stream, {
    refresh,
    onAvailabilityChange: (available) => availability.push(available),
  });
  return { refresh, availability, detach };
}

describe("product event stream subscription", () => {
  it("refreshes the catalog for every kind the server can emit", () => {
    const stream = new FakeEventStream();
    const { refresh } = subscribe(stream);

    for (const kind of PRODUCT_EVENT_KINDS) {
      stream.emit(kind);
    }

    expect(refresh).toHaveBeenCalledTimes(PRODUCT_EVENT_KINDS.length);
  });

  it("tracks availability from open, error, and delivery", () => {
    const stream = new FakeEventStream();
    const { availability } = subscribe(stream);

    stream.emit("open");
    stream.emit("open");
    stream.emit("error");
    // A frame proves the connection is up even when `open` was missed.
    stream.emit("session.status_changed");

    expect(availability).toEqual([true, false, true]);
  });

  it("re-reads the catalog when the server reports an expired cursor", () => {
    const stream = new FakeEventStream();
    const { refresh, availability } = subscribe(stream);

    stream.emit("open");
    stream.emit("error", new AuthorizedStreamErrorEvent(409));

    expect(refresh).toHaveBeenCalledTimes(1);
    expect(availability).toEqual([true, false]);
  });

  it("does not re-read the catalog for a transport error", () => {
    const stream = new FakeEventStream();
    const { refresh } = subscribe(stream);

    stream.emit("open");
    stream.emit("error", new AuthorizedStreamErrorEvent(null));

    expect(refresh).not.toHaveBeenCalled();
  });

  it("stops refreshing and closes the stream on detach", () => {
    const stream = new FakeEventStream();
    const { refresh, availability, detach } = subscribe(stream);

    stream.emit("open");
    stream.emit("session.created");
    detach();
    stream.emit("session.created");
    stream.emit("open");

    expect(refresh).toHaveBeenCalledTimes(1);
    expect(stream.close).toHaveBeenCalledTimes(1);
    expect(availability).toEqual([true, false]);
  });
});

describe("product status poll cadence", () => {
  it("keeps the two cadences at the contract values", () => {
    expect(PRODUCT_STATUS_FALLBACK_POLL_MS).toBe(2_500);
    expect(PRODUCT_STATUS_SAFETY_NET_POLL_MS).toBe(30_000);
  });

  it("demotes polling to the 30s safety net while the stream is available", () => {
    expect(
      decideProductStatusPoll({ streamAvailable: true, hasBusySession: false }),
    ).toEqual({ mode: "safety-net", intervalMs: 30_000 });
    // Unconditional: a live stream must not be able to switch the net off.
    expect(
      decideProductStatusPoll({ streamAvailable: true, hasBusySession: true }),
    ).toEqual({ mode: "safety-net", intervalMs: 30_000 });
  });

  it("restores the 2.5s fallback once the stream is unavailable", () => {
    expect(
      decideProductStatusPoll({ streamAvailable: false, hasBusySession: true }),
    ).toEqual({ mode: "fallback", intervalMs: 2_500 });
  });

  it("does not poll without a stream and without a moving session", () => {
    expect(
      decideProductStatusPoll({ streamAvailable: false, hasBusySession: false }),
    ).toEqual({ mode: "idle" });
  });
});
