/**
 * Where shell overlays mount.
 *
 * The v2 tokens and every scoped v2 rule live on the product frame, so an
 * overlay that has to escape a clipping ancestor (the sidebar's scroll
 * container, a settings card) still has to land inside that frame. Portaling to
 * `document.body` would render it with no tokens at all.
 */
export const SHELL_FRAME_SELECTOR = ".product-app-frame";

export function resolveShellPortalHost(root: Document): HTMLElement | null {
  return root.querySelector<HTMLElement>(SHELL_FRAME_SELECTOR) ?? root.body ?? null;
}
