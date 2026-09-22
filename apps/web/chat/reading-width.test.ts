import { describe, expect, it } from "vitest";

import {
  READING_WIDTH_DEFAULT,
  READING_WIDTH_KEYBOARD_LARGE_STEP,
  READING_WIDTH_KEYBOARD_STEP,
  READING_WIDTH_MAX,
  READING_WIDTH_MIN,
  clampReadingWidth,
  effectiveReadingWidth,
  parseStoredReadingWidth,
  readingWidthFromDrag,
  readingWidthFromKeyboard,
  resolveReadingWidth,
} from "./reading-width";

describe("reading band width preference", () => {
  it("falls back to the design measure for absent or invalid values", () => {
    expect(resolveReadingWidth(undefined)).toBe(READING_WIDTH_DEFAULT);
    expect(resolveReadingWidth(null)).toBe(READING_WIDTH_DEFAULT);
    expect(resolveReadingWidth("900")).toBe(READING_WIDTH_DEFAULT);
    expect(resolveReadingWidth(Number.NaN)).toBe(READING_WIDTH_DEFAULT);
    expect(resolveReadingWidth(720)).toBe(720);
  });

  it("snaps a stored value up to the drag floor", () => {
    expect(resolveReadingWidth(400)).toBe(READING_WIDTH_MIN);
    expect(parseStoredReadingWidth("120")).toBe(READING_WIDTH_MIN);
    expect(parseStoredReadingWidth("abc")).toBeNull();
    expect(parseStoredReadingWidth("")).toBeNull();
    expect(parseStoredReadingWidth(640)).toBeNull();
  });

  it("caps the band at the pane instead of the preference", () => {
    expect(clampReadingWidth(840, 800)).toBe(800);
    expect(clampReadingWidth(840, 1400)).toBe(READING_WIDTH_MAX);
    // A pane narrower than the floor wins: the band draws narrow, silently.
    expect(clampReadingWidth(840, 480)).toBe(480);
    expect(clampReadingWidth(840, 0)).toBe(READING_WIDTH_MIN);
    expect(clampReadingWidth(840, Number.NaN)).toBe(READING_WIDTH_MIN);
  });

  it("renders min(available, preferred) and never exceeds the design cap", () => {
    expect(effectiveReadingWidth(840, 808)).toBe(808);
    expect(effectiveReadingWidth(840, 900)).toBe(840);
    for (const available of [0, 500, 680, 840, 1600, 4000]) {
      const width = effectiveReadingWidth(READING_WIDTH_DEFAULT, available);
      expect(width).toBeLessThanOrEqual(READING_WIDTH_MAX);
      if (available >= READING_WIDTH_MIN) {
        expect(width).toBeGreaterThanOrEqual(READING_WIDTH_MIN);
      }
    }
  });

  // Ported from the reference: both handles move one centered band, so a 1px
  // pointer move is a 2px width change, and the two sides mirror each other.
  it("changes the band width by twice the pointer distance, per side", () => {
    const base = {
      startWidth: 760,
      startClientX: 500,
      available: 1400,
    };
    expect(
      readingWidthFromDrag({ ...base, side: "right", clientX: 540 }),
    ).toBe(840);
    expect(
      readingWidthFromDrag({ ...base, side: "left", clientX: 460 }),
    ).toBe(840);
    expect(
      readingWidthFromDrag({ ...base, side: "right", clientX: 460 }),
    ).toBe(680);
    expect(
      readingWidthFromDrag({ ...base, side: "left", clientX: 540 }),
    ).toBe(680);
  });

  it("clamps a drag against the live pane", () => {
    expect(
      readingWidthFromDrag({
        side: "right",
        startWidth: 700,
        startClientX: 0,
        clientX: 2000,
        available: 820,
      }),
    ).toBe(820);
    expect(
      readingWidthFromDrag({
        side: "right",
        startWidth: 700,
        startClientX: 0,
        clientX: -2000,
        available: 820,
      }),
    ).toBe(READING_WIDTH_MIN);
  });

  it("keys step toward the centre or away from it depending on the edge", () => {
    const right = { side: "right", available: 1400 } as const;
    const left = { side: "left", available: 1400 } as const;
    expect(readingWidthFromKeyboard({ key: "ArrowRight", shiftKey: false }, 760, right)).toBe(
      760 + READING_WIDTH_KEYBOARD_STEP,
    );
    expect(readingWidthFromKeyboard({ key: "ArrowLeft", shiftKey: false }, 760, right)).toBe(
      760 - READING_WIDTH_KEYBOARD_STEP,
    );
    expect(readingWidthFromKeyboard({ key: "ArrowLeft", shiftKey: false }, 760, left)).toBe(
      760 + READING_WIDTH_KEYBOARD_STEP,
    );
    expect(readingWidthFromKeyboard({ key: "ArrowRight", shiftKey: true }, 760, left)).toBe(
      760 - READING_WIDTH_KEYBOARD_LARGE_STEP,
    );
  });

  it("Home returns to the default measure and End spends the pane", () => {
    expect(readingWidthFromKeyboard({ key: "Home", shiftKey: false }, 600, { side: "left", available: 1400 })).toBe(
      READING_WIDTH_DEFAULT,
    );
    expect(readingWidthFromKeyboard({ key: "End", shiftKey: false }, 700, { side: "left", available: 1400 })).toBe(
      READING_WIDTH_MAX,
    );
    expect(readingWidthFromKeyboard({ key: "End", shiftKey: false }, 700, { side: "left", available: 700 })).toBe(700);
  });

  it("ignores keys it does not own", () => {
    for (const key of ["Escape", "Enter", "Tab", "PageUp", "a"]) {
      expect(
        readingWidthFromKeyboard({ key, shiftKey: false }, 760, {
          side: "left",
          available: 1400,
        }),
      ).toBeNull();
    }
  });
});
