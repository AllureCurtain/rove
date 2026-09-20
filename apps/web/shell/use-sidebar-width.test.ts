import { describe, expect, it } from "vitest";

import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_KEYBOARD_LARGE_STEP,
  SIDEBAR_KEYBOARD_STEP,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  boundSidebarWidth,
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
    expect(sidebarWidthKeyboardStep({ key: "ArrowRight", shiftKey: false }, 240)).toBe(
      240 + SIDEBAR_KEYBOARD_STEP,
    );
    expect(sidebarWidthKeyboardStep({ key: "ArrowLeft", shiftKey: false }, 240)).toBe(
      240 - SIDEBAR_KEYBOARD_STEP,
    );
    expect(sidebarWidthKeyboardStep({ key: "ArrowRight", shiftKey: true }, 240)).toBe(
      240 + SIDEBAR_KEYBOARD_LARGE_STEP,
    );
    expect(sidebarWidthKeyboardStep({ key: "ArrowDown", shiftKey: false }, 240)).toBe(
      240 - SIDEBAR_KEYBOARD_STEP,
    );
  });

  it("clamps at the bounds instead of overflowing", () => {
    expect(sidebarWidthKeyboardStep({ key: "ArrowLeft", shiftKey: false }, SIDEBAR_MIN_WIDTH)).toBe(
      SIDEBAR_MIN_WIDTH,
    );
    expect(sidebarWidthKeyboardStep({ key: "End", shiftKey: false }, 240)).toBe(
      SIDEBAR_MAX_WIDTH,
    );
    expect(sidebarWidthKeyboardStep({ key: "Home", shiftKey: false }, 240)).toBe(
      SIDEBAR_MIN_WIDTH,
    );
  });

  it("ignores keys that are not resize keys", () => {
    for (const key of ["Enter", "Escape", "a", "PageUp", "Tab"]) {
      expect(sidebarWidthKeyboardStep({ key, shiftKey: false }, 240)).toBeNull();
    }
  });
});
