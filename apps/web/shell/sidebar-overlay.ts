/**
 * Narrow-screen sidebar overlay policy, ported from open-vetta's root layout
 * (`openOverlay` / `scheduleOverlayClose`).
 *
 * On a narrow screen the rail stops being a column and becomes a floating
 * overlay: a pointer can summon it by touching the left edge, and it hides again
 * shortly after the pointer leaves so crossing a small gap does not flicker.
 *
 * Two things are deliberately different from the reference here:
 *
 * 1. A hover summons a **non-modal peek**: no `role="dialog"`, no `aria-modal`,
 *    no focus trap and no focus move. The reference's overlay is the one
 *    sidebar; rove's narrow rail is also the modal drawer used by the
 *    click/keyboard path, and a modal dialog that appears under the pointer and
 *    pulls focus into itself would hijack the conversation. The deliberate paths
 *    (the header button, or any keyboard user) keep the modal behaviour.
 * 2. The hover edge only exists for a fine pointer (`hover: hover` and
 *    `pointer: fine`), so a touch device never grows an invisible strip that
 *    swallows taps at the left edge of the transcript.
 */

/** Grace period before a peek hides, so a gap crossing is not a flicker. */
export const SIDEBAR_OVERLAY_CLOSE_DELAY_MS = 120;

/** The rail is peekable only on a narrow screen, and only while it is shut. */
export function shouldShowSidebarHoverZone({
  narrow,
  open,
}: {
  narrow: boolean;
  open: boolean;
}): boolean {
  return narrow && !open;
}

/**
 * Focus returns to the conversation heading only for a deliberate open. A peek
 * that moved focus would pull a keyboard user out of whatever they were doing
 * for a purely pointer-driven gesture.
 */
export function shouldFocusSessionHeadingAfterSelection({
  deliberate,
}: {
  deliberate: boolean;
}): boolean {
  return deliberate;
}
