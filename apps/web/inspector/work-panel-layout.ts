/**
 * Shared three-column width budget for the work panel.
 *
 * Ported from PI-Desktop `apps/desktop/src/lib/work-panel-resize.ts` — the
 * reference the workbench design's §5.0 budget row is based on. Three rules are
 * carried over deliberately:
 *
 * 1. The conversation column owns a hard floor, and the panel is capped by the
 *    live container width rather than by a fixed pixel ceiling, so a wide window
 *    can spend width on the panel until the conversation reaches its floor.
 * 2. The left rail yields first: when the requested panel width cannot coexist
 *    with the floor, the shell collapses the rail instead of squeezing the chat.
 * 3. A compact panel width stays representable, because the rail-reopen path may
 *    spend the whole right column and must persist that state.
 *
 * Everything here is pure so the budget can be unit tested without a DOM; the
 * inline `min()` in `product-v2.css` applies the same cap to the grid track.
 */

/** Lower bound for a regular panel width. */
export const WORK_PANEL_MIN_WIDTH = 244;
/** Panel width used when the shell has no stored preference. */
export const WORK_PANEL_DEFAULT_WIDTH = 360;
/** Compact width, used while the rail reopens and spends the right column. */
export const WORK_PANEL_COMPACT_MIN_WIDTH = 1;
/** Hard floor for the conversation column (design §3, §5.0). */
export const MAIN_PANE_MIN_WIDTH = 450;
/** Stable conversation width used while the rail reopens. */
export const MAIN_PANE_REOPEN_TARGET_WIDTH = MAIN_PANE_MIN_WIDTH + 10;
export const WORK_PANEL_KEYBOARD_STEP = 16;
export const WORK_PANEL_KEYBOARD_LARGE_STEP = 32;
/**
 * Upper bound for a *stored* preference only. The rendered width is always
 * capped by the live budget, so this only rejects absurd persisted values.
 */
export const WORK_PANEL_STORAGE_MAX_WIDTH = 10000;

/** A requested width below the regular minimum stays compact on purpose. */
export function workPanelMinimumFor(requestedWidth: number): number {
  return requestedWidth < WORK_PANEL_MIN_WIDTH
    ? WORK_PANEL_COMPACT_MIN_WIDTH
    : WORK_PANEL_MIN_WIDTH;
}

/** Clamps a width to the lower bound only; the budget caps the upper side. */
export function clampWorkPanelWidth(
  width: number,
  minimum: number = WORK_PANEL_MIN_WIDTH,
): number {
  const boundedMinimum = Math.max(
    WORK_PANEL_COMPACT_MIN_WIDTH,
    Math.round(Number.isFinite(minimum) ? minimum : WORK_PANEL_MIN_WIDTH),
  );
  if (!Number.isFinite(width)) {
    return boundedMinimum;
  }
  return Math.max(boundedMinimum, Math.round(width));
}


/** Treats a non-finite measurement as "no space" instead of propagating NaN. */
function finiteOr(value: number, fallback = 0): number {
  return Number.isFinite(value) ? value : fallback;
}

export type WorkPanelLayout = {
  /** Width left for the conversation column. */
  mainWidth: number;
  /** Width the panel actually renders at. */
  panelWidth: number;
  /** Largest panel width that still honours the conversation floor. */
  maxPanelWidth: number;
  /** Whether the shell should collapse the rail to keep the floor. */
  shouldCollapseSidebar: boolean;
};

export function workPanelLayout({
  containerWidth,
  sidebarWidth,
  sidebarCollapsed,
  requestedPanelWidth,
  maximized = false,
}: {
  containerWidth: number;
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  requestedPanelWidth: number;
  maximized?: boolean;
}): WorkPanelLayout {
  const width = Math.max(0, Math.round(finiteOr(containerWidth)));
  const leftWidth = sidebarCollapsed
    ? 0
    : Math.max(0, Math.round(finiteOr(sidebarWidth)));

  // Maximized: the conversation is not rendered, so the panel takes the whole
  // client area beside the rail and the floor does not apply.
  if (maximized) {
    const fullWidth = Math.max(0, width - leftWidth);
    return {
      mainWidth: 0,
      panelWidth: fullWidth,
      maxPanelWidth: fullWidth,
      shouldCollapseSidebar: false,
    };
  }

  const requested = clampWorkPanelWidth(
    finiteOr(requestedPanelWidth, WORK_PANEL_DEFAULT_WIDTH),
    workPanelMinimumFor(finiteOr(requestedPanelWidth, WORK_PANEL_DEFAULT_WIDTH)),
  );
  const maxPanelWidth = Math.max(0, width - leftWidth - MAIN_PANE_MIN_WIDTH);
  const panelWidth = Math.min(requested, maxPanelWidth);

  return {
    mainWidth: Math.max(0, width - leftWidth - panelWidth),
    panelWidth,
    maxPanelWidth,
    shouldCollapseSidebar:
      !sidebarCollapsed && width - leftWidth - requested <= MAIN_PANE_MIN_WIDTH,
  };
}

/**
 * Width used while manually reopening the rail: the panel gives up space first
 * so the conversation keeps its current width, falling back to the reopen
 * target when that would break the floor.
 */
export function workPanelWidthForSidebarReopen({
  containerWidth,
  sidebarWidth,
  currentPanelWidth,
}: {
  containerWidth: number;
  sidebarWidth: number;
  currentPanelWidth: number;
}): number {
  const width = Math.max(0, Math.round(containerWidth));
  const leftWidth = Math.max(0, Math.round(sidebarWidth));
  const panelWidth = clampWorkPanelWidth(
    currentPanelWidth,
    workPanelMinimumFor(currentPanelWidth),
  );
  const remainingAfterRail = width - leftWidth;
  const preservedMainWidth = remainingAfterRail - panelWidth;
  const targetMainWidth =
    preservedMainWidth >= MAIN_PANE_MIN_WIDTH
      ? preservedMainWidth
      : MAIN_PANE_REOPEN_TARGET_WIDTH;
  return Math.max(
    WORK_PANEL_COMPACT_MIN_WIDTH,
    Math.min(panelWidth, remainingAfterRail - targetMainWidth),
  );
}

/**
 * Keyboard resize step for the panel separator. The separator sits on the
 * panel's leading edge, so ArrowLeft widens the panel and ArrowRight narrows it;
 * End goes to the live budget maximum, not to a fixed ceiling (design §3.3).
 * Returns null for keys the shell should leave alone.
 */
export function workPanelKeyboardWidth(
  event: { key: string; shiftKey: boolean },
  current: number,
  maxPanelWidth: number,
): number | null {
  const step = event.shiftKey
    ? WORK_PANEL_KEYBOARD_LARGE_STEP
    : WORK_PANEL_KEYBOARD_STEP;
  const minimum = workPanelMinimumFor(current);
  const maximum = Math.max(minimum, Math.round(maxPanelWidth));
  switch (event.key) {
    case "ArrowLeft":
      return clampWorkPanelWidth(current + step, minimum);
    case "ArrowRight":
      return clampWorkPanelWidth(
        current - step < minimum ? minimum : current - step,
        minimum,
      );
    case "Home":
      return minimum;
    case "End":
      return maximum;
    default:
      return null;
  }
}

/** Parses a stored preference; `null` means "no usable preference". */
export function parseStoredWorkPanelWidth(input: unknown): number | null {
  if (typeof input !== "string" || input.trim() === "") {
    return null;
  }
  const parsed = Number.parseInt(input, 10);
  if (!Number.isSafeInteger(parsed)) {
    return null;
  }
  if (parsed < WORK_PANEL_COMPACT_MIN_WIDTH) {
    return null;
  }
  return Math.min(parsed, WORK_PANEL_STORAGE_MAX_WIDTH);
}
