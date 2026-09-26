/**
 * Conversation reading band width, ported from PI-Desktop's D439
 * (`packages/shared/src/chat-content-width.ts`).
 *
 * The transcript and the composer share one *preferred* max width. The live band
 * is `min(available, preferred)`, so a squeezed conversation column compresses
 * the band without rewriting the user's preference — the reason the reference
 * stores a preference rather than a measured width.
 *
 * Two deliberate differences from the reference:
 *
 * 1. Bounds. The reference allows 560–(pane width) with a 760 default. rove's
 *    design §3.3 documents a 680–840 measure, so the *upper* bound stays 840
 *    (a wider line hurts readability regardless of how wide the window is) while
 *    the *lower* bound takes the reference's 560, because rove's conversation
 *    column can legitimately be narrower than the comfort band and a control
 *    that cannot move is not a control.
 * 2. `available` is measured from the transcript's content box (its own padding
 *    removed) instead of from the pane minus a hard-coded gutter. The reference
 *    can hard-code 24px because its surface is unpadded; rove's transcript and
 *    composer padding lives in CSS, and duplicating it here is exactly the
 *    CSS/JS divergence this shell already had to fix once.
 */

/** Lower bound, from the reference's drag floor. */
export const READING_WIDTH_MIN = 560;
/** Upper bound, from rove's design §3.3 reading measure. */
export const READING_WIDTH_MAX = 840;
/** Default and reset target: the design's full measure. */
export const READING_WIDTH_DEFAULT = 840;
export const READING_WIDTH_KEYBOARD_STEP = 16;
export const READING_WIDTH_KEYBOARD_LARGE_STEP = 32;

export type ReadingWidthSide = "left" | "right";

/** Snap a stored value to the drag floor; `undefined` means "unusable". */
export function normalizeReadingWidth(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }
  return Math.max(READING_WIDTH_MIN, Math.round(value));
}

/** Persistable preference; absent or invalid values fall back to the default. */
export function resolveReadingWidth(value: unknown): number {
  return normalizeReadingWidth(value) ?? READING_WIDTH_DEFAULT;
}

/**
 * Clamp a candidate against the space the band can actually use. Below the
 * floor the pane wins: a narrow column draws narrow instead of overflowing or
 * ignoring the preference.
 */
export function clampReadingWidth(width: number, available: number): number {
  const usable = Number.isFinite(available) ? Math.max(0, Math.round(available)) : 0;
  if (usable <= 0) {
    return READING_WIDTH_MIN;
  }
  const floor = Math.min(READING_WIDTH_MIN, usable);
  const ceiling = Math.max(floor, Math.min(READING_WIDTH_MAX, usable));
  if (!Number.isFinite(width)) {
    return ceiling;
  }
  return Math.min(ceiling, Math.max(floor, Math.round(width)));
}

/** The width the band renders at: `min(available, preferred)`. */
export function effectiveReadingWidth(
  preferred: number,
  available: number,
): number {
  return clampReadingWidth(resolveReadingWidth(preferred), available);
}

/**
 * Both handles move the same centered band, so one pointer pixel changes the
 * width by two: dragging the left handle left (or the right handle right) widens
 * the band by the same amount on both sides. Ported verbatim from the reference.
 */
export function readingWidthFromDrag({
  side,
  startWidth,
  startClientX,
  clientX,
  available,
}: {
  side: ReadingWidthSide;
  startWidth: number;
  startClientX: number;
  clientX: number;
  available: number;
}): number {
  const delta = clientX - startClientX;
  const next =
    side === "left" ? startWidth - 2 * delta : startWidth + 2 * delta;
  return clampReadingWidth(next, available);
}

/**
 * Keyboard contract from the reference: arrows step toward or away from the
 * centre depending on which edge the handle owns, Home returns to the default
 * measure, End spends the whole pane. Returns null for keys the panel or shell
 * should keep.
 */
export function readingWidthFromKeyboard(
  event: { key: string; shiftKey: boolean },
  current: number,
  { side, available }: { side: ReadingWidthSide; available: number },
): number | null {
  const step = event.shiftKey
    ? READING_WIDTH_KEYBOARD_LARGE_STEP
    : READING_WIDTH_KEYBOARD_STEP;
  const towardWider =
    (side === "left" && event.key === "ArrowLeft") ||
    (side === "right" && event.key === "ArrowRight");
  switch (event.key) {
    case "ArrowLeft":
    case "ArrowRight":
      return clampReadingWidth(current + (towardWider ? step : -step), available);
    case "Home":
      return clampReadingWidth(READING_WIDTH_DEFAULT, available);
    case "End":
      return clampReadingWidth(Number.POSITIVE_INFINITY, available);
    default:
      return null;
  }
}

/** Parses a stored preference; `null` means "no usable preference". */
export function parseStoredReadingWidth(input: unknown): number | null {
  if (typeof input !== "string" || input.trim() === "") {
    return null;
  }
  const parsed = Number.parseInt(input, 10);
  if (!Number.isSafeInteger(parsed)) {
    return null;
  }
  return normalizeReadingWidth(parsed) ?? null;
}
