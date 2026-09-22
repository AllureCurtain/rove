import { afterEach, describe, expect, it, vi } from "vitest";

import {
  SCROLLBAR_REVEAL_HOLD_MS,
  SCROLLING_ATTRIBUTE,
  installScrollbarReveal,
} from "./scrollbar-reveal";

function fakeElement() {
  const attributes = new Map<string, string>();
  return {
    attributes,
    setAttribute(name: string, value: string) {
      attributes.set(name, value);
    },
    removeAttribute(name: string) {
      attributes.delete(name);
    },
    get marked() {
      return attributes.has(SCROLLING_ATTRIBUTE);
    },
  };
}

function fakeRoot(documentElement: ReturnType<typeof fakeElement> | null) {
  const listeners = new Set<(event: { target: unknown }) => void>();
  return {
    documentElement,
    listeners,
    addEventListener(type: string, listener: (event: { target: unknown }) => void) {
      expect(type).toBe("scroll");
      listeners.add(listener);
    },
    removeEventListener(_type: string, listener: (event: { target: unknown }) => void) {
      listeners.delete(listener);
    },
    emit(target: unknown) {
      for (const listener of listeners) {
        listener({ target });
      }
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("scrollbar reveal", () => {
  it("marks the element that scrolled and clears it after the hold", () => {
    vi.useFakeTimers();
    const root = fakeRoot(fakeElement());
    const scroller = fakeElement();
    const dispose = installScrollbarReveal(root);

    root.emit(scroller);
    expect(scroller.marked).toBe(true);
    expect(scroller.attributes.get(SCROLLING_ATTRIBUTE)).toBe("");

    vi.advanceTimersByTime(SCROLLBAR_REVEAL_HOLD_MS - 1);
    expect(scroller.marked).toBe(true);
    vi.advanceTimersByTime(1);
    expect(scroller.marked).toBe(false);
    dispose();
  });

  it("attributes a document scroll to the root element", () => {
    vi.useFakeTimers();
    const documentElement = fakeElement();
    const root = fakeRoot(documentElement);
    const dispose = installScrollbarReveal(root);

    // `document` cannot carry attributes, so the mark goes on <html>.
    root.emit({ nodeType: 9 });
    expect(documentElement.marked).toBe(true);
    dispose();
    expect(documentElement.marked).toBe(false);
  });

  it("keeps the mark alive while scrolling continues, then holds once from the last event", () => {
    vi.useFakeTimers();
    const root = fakeRoot(fakeElement());
    const scroller = fakeElement();
    const dispose = installScrollbarReveal(root);

    root.emit(scroller);
    vi.advanceTimersByTime(SCROLLBAR_REVEAL_HOLD_MS - 50);
    root.emit(scroller);
    vi.advanceTimersByTime(SCROLLBAR_REVEAL_HOLD_MS - 50);
    expect(scroller.marked).toBe(true);
    vi.advanceTimersByTime(50);
    expect(scroller.marked).toBe(false);
    dispose();
  });

  it("honours a custom hold and ignores a non-markable target without a root element", () => {
    vi.useFakeTimers();
    const root = fakeRoot(fakeElement());
    const scroller = fakeElement();
    const dispose = installScrollbarReveal(root, { holdMs: 40 });

    root.emit(scroller);
    vi.advanceTimersByTime(40);
    expect(scroller.marked).toBe(false);

    const bare = fakeRoot(null);
    const bareDispose = installScrollbarReveal(bare);
    bare.emit(undefined);
    expect(bare.listeners.size).toBe(1);
    bareDispose();
    dispose();
  });

  it("removes its listener and every mark it set on dispose", () => {
    vi.useFakeTimers();
    const root = fakeRoot(fakeElement());
    const scroller = fakeElement();
    const dispose = installScrollbarReveal(root);

    root.emit(scroller);
    expect(scroller.marked).toBe(true);
    dispose();
    expect(scroller.marked).toBe(false);
    expect(root.listeners.size).toBe(0);
    // A late timer must not resurrect the mark after disposal.
    vi.advanceTimersByTime(SCROLLBAR_REVEAL_HOLD_MS * 2);
    expect(scroller.marked).toBe(false);
  });
});
