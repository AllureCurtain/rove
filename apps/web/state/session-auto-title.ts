/**
 * Frontend auto-title for a still-unnamed session (design F10.2).
 *
 * The server writes this template when a session is created
 * (`apps/api/src/product/store/validation.rs`), and the frontend can only
 * approximate "the user never named this session" by comparing the current title
 * against it — a rename back to the exact template is indistinguishable. That
 * deviation is recorded in the design; the server-side summary title is
 * registered as runtime R10.
 */
export const DEFAULT_SESSION_TITLE = "New session";

/** Design §11.2: the first user message, truncated to 48 code points. */
export const AUTO_TITLE_MAX_CODE_POINTS = 48;

/** True while the session still carries the server's placeholder title. */
export function hasDefaultSessionTitle(title: string | null | undefined): boolean {
  const value = (title ?? "").trim();
  return value.length === 0 || value === DEFAULT_SESSION_TITLE;
}

/**
 * The title a first message gives a still-unnamed session, or `null` when the
 * message holds no usable text. The limit counts code points, so an emoji or any
 * other astral character is never cut in half, and the ellipsis is only added
 * when something was actually dropped. Runs of whitespace collapse to one space
 * because the title is a single line in the rail, the header and the palette.
 */
export function autoSessionTitle(message: string): string | null {
  const compact = message.replace(/\s+/gu, " ").trim();
  if (compact.length === 0) {
    return null;
  }
  const codePoints = Array.from(compact);
  return codePoints.length <= AUTO_TITLE_MAX_CODE_POINTS
    ? compact
    : `${codePoints.slice(0, AUTO_TITLE_MAX_CODE_POINTS).join("")}...`;
}
