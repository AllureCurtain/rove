import type { WorkspaceRecord } from "../state/product-types";

/**
 * §7.3: project rows reorder by drag. The manual order is a per-machine
 * reading preference — the same rule session pins already follow — so it lives
 * in `localStorage`, not the product store. Entries are workspace ids; unknown
 * or stale ids drop out without fanfare, and workspaces the order does not
 * mention keep their server order after the remembered ones.
 */
export const PROJECT_ORDER_KEY = "rove.ui-project-order";

export function readProjectOrder(): string[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(PROJECT_ORDER_KEY);
    if (!raw) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter((id): id is string => typeof id === "string");
  } catch {
    return [];
  }
}

export function writeProjectOrder(ids: readonly string[]): void {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(PROJECT_ORDER_KEY, JSON.stringify(ids));
}

/**
 * Apply a remembered order: pinned workspaces always lead (the server-side
 * pin outranks a manual slot), then remembered ids in order, then the rest in
 * their catalog order.
 */
export function orderWorkspaces(
  workspaces: readonly WorkspaceRecord[],
  remembered: readonly string[],
): WorkspaceRecord[] {
  const position = new Map(remembered.map((id, index) => [id, index]));
  return [...workspaces].sort((left, right) => {
    if (left.pinned !== right.pinned) {
      return left.pinned ? -1 : 1;
    }
    const leftOrder = position.get(left.id);
    const rightOrder = position.get(right.id);
    if (leftOrder !== undefined && rightOrder !== undefined) {
      return leftOrder - rightOrder;
    }
    if (leftOrder !== undefined) {
      return -1;
    }
    if (rightOrder !== undefined) {
      return 1;
    }
    return 0;
  });
}

/**
 * The order ids the drop produces: `dragged` moves directly before
 * `targetId` (`after` puts it directly behind). The returned list covers every
 * workspace, so persisting it verbatim keeps the new relative order stable.
 */
export function reorderProjectOrder(
  currentIds: readonly string[],
  dragged: string,
  targetId: string,
  after: boolean,
): string[] {
  const next = currentIds.filter((id) => id !== dragged);
  const targetIndex = next.indexOf(targetId);
  if (targetIndex === -1) {
    return [...next, dragged];
  }
  next.splice(after ? targetIndex + 1 : targetIndex, 0, dragged);
  return next;
}
