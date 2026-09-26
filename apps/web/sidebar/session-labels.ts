import type { SessionRecord } from "../state/product-types";

/**
 * Sidebar presentation helpers shared by the session row and its hover card
 * (design F4). They used to live inside `WorkspaceTree.tsx`; the card needs the
 * same words, so they moved here instead of being copied.
 */

export function sessionStatusLabel(
  status: SessionRecord["status"],
  t: (path: string) => string,
): string {
  switch (status) {
    case "running":
      return t("workspace.running");
    case "needs_attention":
      return t("workspace.needsAttention");
    case "error":
      return t("inspector.statusFailed");
    default:
      return t("inspector.statusQueued");
  }
}

export function shortId(value: string): string {
  return value.length <= 10 ? value : value.slice(0, 10);
}

export function formatDisplayPath(path: string): string {
  return path.startsWith("\\\\?\\") ? path.slice(4) : path;
}
