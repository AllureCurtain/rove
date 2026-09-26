/**
 * Work panel tab model.
 *
 * Ported from PI-Desktop `apps/desktop/src/lib/work-panel-tabs.ts`, minus the
 * plugin-view machinery rove does not have: the reference panel is a *dynamic*
 * tab strip (open, close, replace, per-session contexts) rather than a fixed set
 * of always-present tabs. Every kind here is host-owned, so a tab is identified
 * by its kind.
 *
 * The pure state helpers keep the tab logic testable without a DOM.
 */

export const WORK_PANEL_TAB_KINDS = [
  /** Launcher page created by the `+` action. */
  "new",
  "pending",
  "status",
  "files",
  "changes",
  "review",
] as const;

export type WorkPanelTabKind = (typeof WORK_PANEL_TAB_KINDS)[number];

export type WorkPanelTab = {
  kind: WorkPanelTabKind;
};

export type WorkPanelTabsState = {
  tabs: WorkPanelTab[];
  activeKind: WorkPanelTabKind | null;
};

/** Kinds the launcher offers; `new` itself is never offered. */
export const WORK_PANEL_LAUNCHER_KINDS = [
  "pending",
  "status",
  "files",
  "changes",
  "review",
] as const satisfies readonly WorkPanelTabKind[];

/**
 * Default context for a session: the run status, because the panel's first job
 * is to show what the current turn is doing.
 */
export function defaultWorkPanelTabs(): WorkPanelTabsState {
  return { tabs: [{ kind: "status" }], activeKind: "status" };
}

export function isWorkPanelTabKind(value: unknown): value is WorkPanelTabKind {
  return (
    typeof value === "string" &&
    (WORK_PANEL_TAB_KINDS as readonly string[]).includes(value)
  );
}

/** Drops unknown kinds and duplicates, then repairs the active kind. */
export function sanitizeWorkPanelTabs(state: WorkPanelTabsState): WorkPanelTabsState {
  const tabs: WorkPanelTab[] = [];
  for (const tab of state.tabs) {
    if (!tab || !isWorkPanelTabKind(tab.kind)) {
      continue;
    }
    if (tabs.some((existing) => existing.kind === tab.kind)) {
      continue;
    }
    tabs.push({ kind: tab.kind });
  }
  const activeKind =
    state.activeKind && tabs.some((tab) => tab.kind === state.activeKind)
      ? state.activeKind
      : (tabs.at(-1)?.kind ?? null);
  return { tabs, activeKind };
}

/** Opens a kind, reusing the tab when it is already open. */
export function openWorkPanelTab(
  state: WorkPanelTabsState,
  kind: WorkPanelTabKind,
): WorkPanelTabsState {
  if (state.tabs.some((tab) => tab.kind === kind)) {
    return { ...state, activeKind: kind };
  }
  return { tabs: [...state.tabs, { kind }], activeKind: kind };
}

export function activateWorkPanelTab(
  state: WorkPanelTabsState,
  kind: WorkPanelTabKind,
): WorkPanelTabsState {
  return state.tabs.some((tab) => tab.kind === kind)
    ? { ...state, activeKind: kind }
    : state;
}

/**
 * Closes a tab and activates its successor (the next tab, else the previous),
 * matching the reference implementation's close-then-focus behaviour.
 */
export function closeWorkPanelTab(
  state: WorkPanelTabsState,
  kind: WorkPanelTabKind,
): WorkPanelTabsState {
  const index = state.tabs.findIndex((tab) => tab.kind === kind);
  if (index === -1) {
    return state;
  }
  const tabs = state.tabs.filter((tab) => tab.kind !== kind);
  if (state.activeKind !== kind) {
    return { tabs, activeKind: state.activeKind };
  }
  return { tabs, activeKind: tabs[Math.min(index, tabs.length - 1)]?.kind ?? null };
}

/** Replaces a launcher tab with the chosen kind, reusing an open tab. */
export function replaceWorkPanelTab(
  state: WorkPanelTabsState,
  sourceKind: WorkPanelTabKind,
  kind: WorkPanelTabKind,
): WorkPanelTabsState {
  const sourceIndex = state.tabs.findIndex((tab) => tab.kind === sourceKind);
  if (sourceIndex < 0) {
    return openWorkPanelTab(state, kind);
  }
  if (sourceKind === kind) {
    return { ...state, activeKind: kind };
  }
  const tabs = [...state.tabs];
  tabs[sourceIndex] = { kind };
  const deduped = tabs.filter(
    (tab, position) =>
      tab.kind !== kind || tabs.findIndex((other) => other.kind === kind) === position,
  );
  return { tabs: deduped, activeKind: kind };
}

/** Step for the tab strip's arrow keys; wraps around like the reference. */
export function workPanelTabStep(
  event: { key: string },
  index: number,
  count: number,
): number | null {
  if (count <= 0 || index < 0) {
    return null;
  }
  switch (event.key) {
    case "ArrowLeft":
      return (index - 1 + count) % count;
    case "ArrowRight":
      return (index + 1) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}
