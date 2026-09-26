/**
 * Reader font scale for the conversation surface.
 *
 * Stored locally, applied as one CSS variable on the product frame so every
 * inheriting text scales together. The range is narrower than a whole-UI
 * zoom because the reading column caps at 840px: past 1.4 the line length
 * breaks before the type does.
 */

export const FONT_SCALE_MIN = 0.85;
export const FONT_SCALE_MAX = 1.4;
export const FONT_SCALE_STEP = 0.05;
export const FONT_SCALE_DEFAULT = 1;

export function clampFontScale(value: number): number {
  if (!Number.isFinite(value)) {
    return FONT_SCALE_DEFAULT;
  }
  return Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, value));
}

/** Snap to the nearest step so stored values stay on the offered scale. */
export function alignFontScale(value: number): number {
  const clamped = clampFontScale(value);
  const steps = Math.round((clamped - FONT_SCALE_MIN) / FONT_SCALE_STEP);
  const aligned = FONT_SCALE_MIN + steps * FONT_SCALE_STEP;
  return clampFontScale(Number(aligned.toFixed(2)));
}

export function stepFontScale(value: number, direction: "up" | "down"): number {
  const base = alignFontScale(value);
  const stepped = direction === "up" ? base + FONT_SCALE_STEP : base - FONT_SCALE_STEP;
  return clampFontScale(Number(stepped.toFixed(2)));
}
