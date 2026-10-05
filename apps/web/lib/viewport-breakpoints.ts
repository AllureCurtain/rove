/**
 * Responsive breakpoints shared by the shell's JavaScript and CSS.
 *
 * The media queries in `styles/product-v2.css` mirror these values; when one
 * side changes, change both. The e2e layout suite pins the behaviour on either
 * side of the boundary (three columns stay one row at 1024x768, the panel opens
 * as a dialog at 375x812).
 */

/**
 * At or below this width the rail and the work panel become overlays: the rail
 * is a drawer and the panel opens as a modal dialog. Above it they are columns
 * that share the width budget (design §5.0).
 */
export const DRAWER_MAX_WIDTH = 960;

export const DRAWER_MEDIA_QUERY = `(max-width: ${DRAWER_MAX_WIDTH}px)`;

/** True when the viewport is in the drawer layout; false during SSR. */
export function matchesDrawerLayout(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia(DRAWER_MEDIA_QUERY).matches;
}
