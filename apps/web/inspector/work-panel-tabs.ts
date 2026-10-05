/**
 * Work panel tab model.
 *
 * Tabs are a *dynamic* set (open, close, reorder, replace, per-session
 * contexts) identified by id, not a fixed set of always-present tabs, and there
 * is no plugin-view machinery: every kind here is host-owned. Singleton kinds
 * use their kind as the id; a `file` tab carries the workspace-relative path it
 * views, so every opened file is its own tab (design §8). The pure state
 * helpers keep the tab logic testable without a DOM.
 */

export const WORK_PANEL_TAB_KINDS = [
  /** Launcher page created by the `+` action. */
  "new",
  "pending",
  "status",
  /** Directory browser; opening a file spawns a `file` tab. */
  "files",
  /** One file's viewer. `path` carries the workspace-relative path. */
  "file",
  /** Aggregated diff — the evidence surface and the default tab. */
  "changes",
  /** Code-review findings for the session. */
  "review",
  /** Subagent transcripts. */
  "subagents",
] as const;

export type WorkPanelTabKind = (typeof WORK_PANEL_TAB_KINDS)[number];

export type WorkPanelTab = {
  /** Stable identity: the kind for singleton views, `file:<path>` for files. */
  id: string;
  kind: WorkPanelTabKind;
  /** File tabs only: the workspace-relative path this tab views. */
  path?: string;
};

export type WorkPanelTabsState = {
  tabs: WorkPanelTab[];
  activeId: string | null;
};

/** Kinds the launcher offers; `new` and `file` are never offered. */
export const WORK_PANEL_LAUNCHER_KINDS = [
  "pending",
  "status",
  "files",
  "changes",
  "review",
  "subagents",
] as const satisfies readonly WorkPanelTabKind[];

/**
 * A tab's id is its identity: singleton kinds are unique by kind, while a file
 * tab is unique by the path it views, so reopening the same file reuses its tab.
 */
export function workPanelTabId(kind: WorkPanelTabKind, path?: string): string {
  return kind === "file" && path ? `file:${path}` : kind;
}

/**
 * Default context for a session: the aggregated diff, because the panel's
 * first job is to show evidence of what the run changed (design §8, §2.2).
 */
export function defaultWorkPanelTabs(): WorkPanelTabsState {
  return {
    tabs: [{ id: "changes", kind: "changes" }],
    activeId: "changes",
  };
}

export function isWorkPanelTabKind(value: unknown): value is WorkPanelTabKind {
  return (
    typeof value === "string" &&
    (WORK_PANEL_TAB_KINDS as readonly string[]).includes(value)
  );
}

/** Drops unknown kinds, malformed tabs, and duplicates, then repairs the active tab. */
export function sanitizeWorkPanelTabs(state: WorkPanelTabsState): WorkPanelTabsState {
  const tabs: WorkPanelTab[] = [];
  for (const tab of state.tabs) {
    if (!tab || !isWorkPanelTabKind(tab.kind)) {
      continue;
    }
    const path = tab.kind === "file" && typeof tab.path === "string" && tab.path !== ""
      ? tab.path
      : undefined;
    if (tab.kind === "file" && !path) {
      continue;
    }
    const id = workPanelTabId(tab.kind, path);
    if (tabs.some((existing) => existing.id === id)) {
      continue;
    }
    tabs.push({ id, kind: tab.kind, ...(path ? { path } : {}) });
  }
  const activeId =
    state.activeId && tabs.some((tab) => tab.id === state.activeId)
      ? state.activeId
      : (tabs.at(-1)?.id ?? null);
  return { tabs, activeId };
}

/** Opens a kind, reusing the tab when it is already open. */
export function openWorkPanelTab(
  state: WorkPanelTabsState,
  kind: WorkPanelTabKind,
): WorkPanelTabsState {
  const id = workPanelTabId(kind);
  if (state.tabs.some((tab) => tab.id === id)) {
    return { ...state, activeId: id };
  }
  return { tabs: [...state.tabs, { id, kind }], activeId: id };
}

/**
 * Opens a file's viewer tab, reusing it when the file is already open so the
 * strip never stacks duplicates for one path.
 */
export function openWorkPanelFileTab(
  state: WorkPanelTabsState,
  path: string,
): WorkPanelTabsState {
  const id = workPanelTabId("file", path);
  const existing = state.tabs.find((tab) => tab.id === id);
  if (existing) {
    return { ...state, activeId: id };
  }
  return {
    tabs: [...state.tabs, { id, kind: "file", path }],
    activeId: id,
  };
}

export function activateWorkPanelTab(
  state: WorkPanelTabsState,
  id: string,
): WorkPanelTabsState {
  return state.tabs.some((tab) => tab.id === id)
    ? { ...state, activeId: id }
    : state;
}

/**
 * Closes a tab and activates its successor (the next tab, else the previous),
 * so the strip never leaves focus on a closed tab.
 */
export function closeWorkPanelTab(
  state: WorkPanelTabsState,
  id: string,
): WorkPanelTabsState {
  const index = state.tabs.findIndex((tab) => tab.id === id);
  if (index === -1) {
    return state;
  }
  const tabs = state.tabs.filter((tab) => tab.id !== id);
  if (state.activeId !== id) {
    return { tabs, activeId: state.activeId };
  }
  return { tabs, activeId: tabs[Math.min(index, tabs.length - 1)]?.id ?? null };
}

/** Replaces a launcher tab with the chosen kind, reusing an open tab. */
export function replaceWorkPanelTab(
  state: WorkPanelTabsState,
  sourceId: string,
  kind: WorkPanelTabKind,
): WorkPanelTabsState {
  const sourceIndex = state.tabs.findIndex((tab) => tab.id === sourceId);
  if (sourceIndex < 0) {
    return openWorkPanelTab(state, kind);
  }
  const id = workPanelTabId(kind);
  if (sourceId === id) {
    return { ...state, activeId: id };
  }
  const tabs = [...state.tabs];
  tabs[sourceIndex] = { id, kind };
  const deduped = tabs.filter(
    (tab, position) =>
      tab.id !== id || tabs.findIndex((other) => other.id === id) === position,
  );
  return { tabs: deduped, activeId: id };
}

/**
 * Reorders a tab to an absolute index (design §8: drag reorder and
 * Alt+Left/Alt+Right). Out-of-range indices clamp; a no-op returns `state`.
 */
export function moveWorkPanelTab(
  state: WorkPanelTabsState,
  id: string,
  toIndex: number,
): WorkPanelTabsState {
  const fromIndex = state.tabs.findIndex((tab) => tab.id === id);
  if (fromIndex === -1) {
    return state;
  }
  const target = Math.max(0, Math.min(state.tabs.length - 1, Math.round(toIndex)));
  if (target === fromIndex) {
    return state;
  }
  const tabs = [...state.tabs];
  const [moved] = tabs.splice(fromIndex, 1);
  tabs.splice(target, 0, moved!);
  return { ...state, tabs };
}

/**
 * Serializes the strip for storage (design §8: tab order persists and a refresh
 * restores exactly what was left). The shape is deliberately small — id, kind,
 * and the file path — so a stale payload can never rehydrate foreign state.
 */
export function serializeWorkPanelTabs(state: WorkPanelTabsState): string {
  return JSON.stringify({
    tabs: state.tabs.map((tab) => ({
      id: tab.id,
      kind: tab.kind,
      ...(tab.path ? { path: tab.path } : {}),
    })),
    activeId: state.activeId,
  });
}

/** Parses a stored strip; `null` means "no usable preference". */
export function parseStoredWorkPanelTabs(input: unknown): WorkPanelTabsState | null {
  if (typeof input !== "string" || input.trim() === "") {
    return null;
  }
  try {
    const parsed = JSON.parse(input) as { tabs?: unknown; activeId?: unknown };
    if (!Array.isArray(parsed?.tabs)) {
      return null;
    }
    const candidate: WorkPanelTabsState = {
      tabs: parsed.tabs.flatMap((tab): WorkPanelTab[] => {
        if (!tab || typeof tab !== "object") {
          return [];
        }
        const record = tab as Record<string, unknown>;
        if (!isWorkPanelTabKind(record.kind)) {
          return [];
        }
        const path =
          typeof record.path === "string" && record.path !== ""
            ? record.path
            : undefined;
        return [
          {
            id: workPanelTabId(record.kind, path),
            kind: record.kind,
            ...(path ? { path } : {}),
          },
        ];
      }),
      activeId: typeof parsed.activeId === "string" ? parsed.activeId : null,
    };
    return sanitizeWorkPanelTabs(candidate);
  } catch {
    return null;
  }
}

/** Step for the tab strip's arrow keys; wraps around. */
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
