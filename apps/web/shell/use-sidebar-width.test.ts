import { describe, expect, it } from "vitest";

import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_KEYBOARD_LARGE_STEP,
  SIDEBAR_KEYBOARD_STEP,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  boundSidebarWidth,
  sidebarWidthFromPointer,
  sidebarWidthKeyboardStep,
} from "./use-sidebar-width";

describe("sidebar width preference", () => {  it("keeps the width inside the design bounds", () => {
    expect(boundSidebarWidth(120)).toBe(SIDEBAR_MIN_WIDTH);
    expect(boundSidebarWidth(9999)).toBe(SIDEBAR_MAX_WIDTH);
    expect(boundSidebarWidth(280)).toBe(280);
    expect(boundSidebarWidth(Number.NaN)).toBe(SIDEBAR_DEFAULT_WIDTH);
    expect(boundSidebarWidth(Number.POSITIVE_INFINITY)).toBe(SIDEBAR_DEFAULT_WIDTH);
  });

  it("steps by the design amount and multiplies it with Shift", () => {
    expect(sidebarWidthKeyboardStep({ key: "ArrowRight", shiftKey: false }, 275)).toBe(
      275 + SIDEBAR_KEYBOARD_STEP,
    );
    expect(sidebarWidthKeyboardStep({ key: "ArrowLeft", shiftKey: false }, 275)).toBe(
      275 - SIDEBAR_KEYBOARD_STEP,
    );
    expect(sidebarWidthKeyboardStep({ key: "ArrowRight", shiftKey: true }, 275)).toBe(
      275 + SIDEBAR_KEYBOARD_LARGE_STEP,
    );
    expect(sidebarWidthKeyboardStep({ key: "ArrowDown", shiftKey: false }, 275)).toBe(
      275 - SIDEBAR_KEYBOARD_STEP,
    );
  });

  it("clamps at the bounds instead of overflowing", () => {
    expect(sidebarWidthKeyboardStep({ key: "ArrowLeft", shiftKey: false }, SIDEBAR_MIN_WIDTH)).toBe(
      SIDEBAR_MIN_WIDTH,
    );
    expect(sidebarWidthKeyboardStep({ key: "End", shiftKey: false }, 275)).toBe(
      SIDEBAR_MAX_WIDTH,
    );
    expect(sidebarWidthKeyboardStep({ key: "Home", shiftKey: false }, 275)).toBe(
      SIDEBAR_MIN_WIDTH,
    );
  });

  it("ignores keys that are not resize keys", () => {
    for (const key of ["Enter", "Escape", "a", "PageUp", "Tab"]) {
      expect(sidebarWidthKeyboardStep({ key, shiftKey: false }, 275)).toBeNull();
    }
  });

  // Regression: the pointer position must be measured from the shell, not from
  // the handle. The handle sits on the rail's trailing edge and slides with the
  // width, so measuring from it made the grab itself collapse the rail to the
  // minimum instead of tracking the pointer.
  it("takes the rail width from the pointer position, not the handle's own edge", () => {
    expect(sidebarWidthFromPointer(280, 0)).toBe(280);
    expect(sidebarWidthFromPointer(240, 0)).toBe(240);
    expect(sidebarWidthFromPointer(360, 0)).toBe(360);
  });

  it("honours a container that does not start at the viewport edge", () => {
    expect(sidebarWidthFromPointer(320, 40)).toBe(280);
    expect(sidebarWidthFromPointer(335, 60)).toBe(SIDEBAR_DEFAULT_WIDTH);
  });

  it("bounds the pointer result and survives a non-finite container offset", () => {
    expect(sidebarWidthFromPointer(40, 0)).toBe(SIDEBAR_MIN_WIDTH);
    expect(sidebarWidthFromPointer(1200, 0)).toBe(SIDEBAR_MAX_WIDTH);
    expect(sidebarWidthFromPointer(280, Number.NaN)).toBe(280);
  });
});
