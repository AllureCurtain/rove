/**
 * Idle-time route prefetching, ported from open-vetta's
 * `useIdleRoutePrefetch` (`requestIdleCallback` with a timeout, plus a timer
 * fallback where it does not exist, cancelled on unmount, failures swallowed).
 *
 * The point is the first click: in the App Router each destination is its own
 * route payload, so clicking Settings or a workspace pays for the fetch and the
 * parse at click time. Pulling those payloads in during idle time makes the
 * first click free without competing with the boot path for the main thread.
 *
 * The list is deliberately bounded and derived from where the user already is:
 * the settings sections the header gear opens, plus the active workspace as the
 * way back. Prefetching the whole catalog would put the boot path in competition
 * with itself.
 */

/** `requestIdleCallback` timeout: prefetch eventually even on a busy main thread. */
export const ROUTE_PREFETCH_IDLE_TIMEOUT_MS = 8000;
/** Fallback delay where `requestIdleCallback` is unavailable. */
export const ROUTE_PREFETCH_FALLBACK_DELAY_MS = 3000;
/** The settings sections reachable in one click from the shell header. */
export const ROUTE_PREFETCH_SETTINGS_SECTIONS = ["general", "providers"] as const;

/**
 * Prefetching is a production behaviour in both senses.
 *
 * Next only issues the requests in a production build, and in development a
 * prefetch request would only compete with on-demand compilation for the very
 * route being compiled — the dev server gains nothing and the test suite gets
 * slower. Next reaches the same conclusion internally; stating it here keeps our
 * hook silent in dev instead of depending on that.
 */
export function shouldPrefetchRoutes(nodeEnv: string | undefined): boolean {
  return nodeEnv === "production";
}

export function routePrefetchTargets({
  activeWorkspaceId,
}: {
  activeWorkspaceId: string | null;
}): string[] {
  const targets: string[] = ROUTE_PREFETCH_SETTINGS_SECTIONS.map(
    (section) => `/settings/${section}`,
  );
  if (activeWorkspaceId) {
    targets.push(`/w/${activeWorkspaceId}`);
  }
  return [...new Set(targets)];
}
