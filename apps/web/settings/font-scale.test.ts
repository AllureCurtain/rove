import { describe, expect, it } from "vitest";

import {
  FONT_SCALE_DEFAULT,
  FONT_SCALE_MAX,
  FONT_SCALE_MIN,
  alignFontScale,
  clampFontScale,
  stepFontScale,
} from "./font-scale";

describe("font scale", () => {
  it("clamps out-of-range values", () => {
    expect(clampFontScale(0.5)).toBe(FONT_SCALE_MIN);
    expect(clampFontScale(2)).toBe(FONT_SCALE_MAX);
    expect(clampFontScale(Number.NaN)).toBe(FONT_SCALE_DEFAULT);
  });

  it("aligns stored values to the offered step", () => {
    expect(alignFontScale(1.02)).toBe(1);
    expect(alignFontScale(1.03)).toBe(1.05);
    expect(alignFontScale(0.85)).toBe(FONT_SCALE_MIN);
    expect(alignFontScale(1.4)).toBe(FONT_SCALE_MAX);
  });

  it("steps without leaving the range", () => {
    expect(stepFontScale(1, "up")).toBeCloseTo(1.05, 5);
    expect(stepFontScale(1, "down")).toBeCloseTo(0.95, 5);
    expect(stepFontScale(FONT_SCALE_MAX, "up")).toBe(FONT_SCALE_MAX);
    expect(stepFontScale(FONT_SCALE_MIN, "down")).toBe(FONT_SCALE_MIN);
  });
});
