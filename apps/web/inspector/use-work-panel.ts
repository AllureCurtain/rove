"use client";

import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

import {
  WORK_PANEL_DEFAULT_WIDTH,
  clampWorkPanelWidth,
  parseStoredWorkPanelWidth,
  workPanelKeyboardWidth,
  workPanelMinimumFor,
} from "./work-panel-layout";

import {
  activateWorkPanelTab,
  closeWorkPanelTab,
  defaultWorkPanelTabs,
  moveWorkPanelTab,
  openWorkPanelFileTab,
  openWorkPanelTab,
  parseStoredWorkPanelTabs,
  replaceWorkPanelTab,
  sanitizeWorkPanelTabs,
  serializeWorkPanelTabs,
  type WorkPanelTabKind,
  type WorkPanelTabsState,
} from "./work-panel-tabs";

export type WorkPanelTarget =
  | { kind: "approval"; jobId: string; runId: string; callId: string }
  | { kind: "file"; path: string; line: number }
  | null;
interface PanelSelection {
  tabs: WorkPanelTabsState;
  target: WorkPanelTarget;
  width: number;
  collapsed: boolean;
  /**
   * Maximized takes the middle column's width (design §8). It is a *session*
   * mode, not a persisted preference: a refresh restores the stored open flag
   * and tab strip, and the panel comes back at its regular width.
   */
  maximized: boolean;
  returnFocus: HTMLElement | null;
}

/**
 * Panel width as a UI preference (design §3 ownership table: layout values live
 * in localStorage, not in the product preference contract). The stored value is
 * a *request*; the rendered width is always the live budget's answer
 * (`workPanelLayout`), so a wide stored value can never squeeze the chat.
 */
const STORAGE_KEY = "rove.ui-work-panel-width";
/**
 * Whether the panel is open is a UI preference too (design §5.0): the panel is
 * closed by default and a reload restores exactly the state that was left.
 * Only user-initiated toggles write it — the drawer's force-close on a narrow
 * layout does not, or a phone visit would erase the desktop preference.
 */
const STORAGE_OPEN_KEY = "rove.ui-work-panel-open";
/**
 * The tab strip persists per session (design §8: tab order persists; refresh
 * restores exactly). One storage key holds a bounded map of session keys to
 * serialized strips so a long-lived catalog cannot grow the entry without limit.
 */
const STORAGE_TABS_KEY = "rove.ui-work-panel-tabs";
const STORED_TAB_SESSION_LIMIT = 24;

function readStoredPanelOpen(): boolean | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_OPEN_KEY);
    return raw === "1" ? true : raw === "0" ? false : null;
  } catch {
    return null;
  }
}

function writeStoredPanelOpen(open: boolean): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(STORAGE_OPEN_KEY, open ? "1" : "0");
  } catch {
    // Optional preference; the in-memory state still applies.
  }
}

function readStoredTabsMap(): Map<string, string> {
  if (typeof window === "undefined") {
    return new Map();
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_TABS_KEY);
    if (!raw) {
      return new Map();
    }
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const entries = Object.entries(parsed).filter(
      (entry): entry is [string, string] => typeof entry[1] === "string",
    );
    return new Map(entries);
  } catch {
    return new Map();
  }
}

/**
 * The stored key pairs the workspace and session so one workspace's strip never
 * bleeds into another's (design §8: panel selection is session-isolated).
 */
function storedTabsKey(workspaceId: string | null, sessionId: string | null): string | null {
  if (!workspaceId || !sessionId) {
    return null;
  }
  return `${workspaceId}::${sessionId}`;
}

function readStoredPanelTabs(
  workspaceId: string | null,
  sessionId: string | null,
): WorkPanelTabsState | null {
  const storedKey = storedTabsKey(workspaceId, sessionId);
  if (!storedKey) {
    return null;
  }
  return parseStoredWorkPanelTabs(readStoredTabsMap().get(storedKey) ?? null);
}

function writeStoredPanelTabs(
  workspaceId: string | null,
  sessionId: string | null,
  tabs: WorkPanelTabsState,
): void {
  const storedKey = storedTabsKey(workspaceId, sessionId);
  if (!storedKey || typeof window === "undefined") {
    return;
  }
  try {
    const map = readStoredTabsMap();
    // Refresh the entry's insertion order, then bound the map so a long-lived
    // catalog cannot grow the stored preference without limit.
    map.delete(storedKey);
    map.set(storedKey, serializeWorkPanelTabs(tabs));
    while (map.size > STORED_TAB_SESSION_LIMIT) {
      map.delete(map.keys().next().value!);
    }
    window.localStorage.setItem(STORAGE_TABS_KEY, JSON.stringify(Object.fromEntries(map)));
  } catch {
    // Optional preference; the in-memory strip still applies.
  }
}

function readStoredPanelWidth(): number {
  if (typeof window === "undefined") {
    return WORK_PANEL_DEFAULT_WIDTH;
  }
  try {
    return (
      parseStoredWorkPanelWidth(window.localStorage.getItem(STORAGE_KEY)) ??
      WORK_PANEL_DEFAULT_WIDTH
    );
  } catch {
    // A blocked storage quota must not break resizing.
    return WORK_PANEL_DEFAULT_WIDTH;
  }
}

function writeStoredPanelWidth(width: number): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(STORAGE_KEY, String(width));
  } catch {
    // Ignore; the in-memory width still applies for this session.
  }
}

const initialSelection = (): PanelSelection => ({
  tabs: defaultWorkPanelTabs(),
  target: null,
  width: WORK_PANEL_DEFAULT_WIDTH,
  // Closed by default (design §5.0); a stored "open" restores after mount.
  collapsed: true,
  maximized: false,
  returnFocus: null,
});

/** UI-only, in-memory selection. Authority stays in the focused run projection. */
export function useWorkPanel(
  workspaceId: string | null,
  sessionId: string | null,
  fallbackRef?: RefObject<HTMLElement | null>,
) {
  const key = JSON.stringify([workspaceId, sessionId]);
  const selections = useRef(new Map<string, PanelSelection>());
  const [, render] = useState(0);
  const selection = selections.current.get(key) ?? initialSelection();

  // Restore after mount so the server and the first client render agree. The
  // stored open flag and stored strip seed a session the shell has not visited
  // yet; a selection that already exists keeps the state the user set this
  // visit and only takes the persisted width.
  useEffect(() => {
    const storedWidth = readStoredPanelWidth();
    const storedOpen = readStoredPanelOpen();
    const current = selections.current.get(key);
    if (current) {
      if (storedWidth !== WORK_PANEL_DEFAULT_WIDTH && current.width !== storedWidth) {
        selections.current.set(key, { ...current, width: storedWidth });
        render((value) => value + 1);
      }
      return;
    }
    const storedTabs = readStoredPanelTabs(workspaceId, sessionId);
    if (storedWidth === WORK_PANEL_DEFAULT_WIDTH && storedOpen === null && storedTabs === null) {
      return;
    }
    selections.current.set(key, {
      ...initialSelection(),
      width: storedWidth,
      collapsed: storedOpen === null ? true : !storedOpen,
      tabs: storedTabs ?? defaultWorkPanelTabs(),
    });
    render((value) => value + 1);
  }, [key, workspaceId, sessionId]);

  function update(patch: Partial<PanelSelection>) {
    // Bound the UI cache; evicting a selection never changes runtime state.
    if (!selections.current.has(key) && selections.current.size >= 64) {
      selections.current.delete(selections.current.keys().next().value!);
    }
    const next = { ...(selections.current.get(key) ?? initialSelection()), ...patch };
    selections.current.set(key, next);
    if (patch.tabs) {
      writeStoredPanelTabs(workspaceId, sessionId, next.tabs);
    }
    render((value) => value + 1);
  }
  const setTabs = (tabs: WorkPanelTabsState) =>
    update({ tabs: sanitizeWorkPanelTabs(tabs) });
  function open(kind?: WorkPanelTabKind, target = selection.target, trigger?: HTMLElement | null) {
    update({
      collapsed: false,
      // No kind means "reopen to the strip exactly as it was left"; a file id
      // is already a real tab, so it is activated, never re-created as a kind.
      tabs: kind ? openWorkPanelTab(selection.tabs, kind) : selection.tabs,
      target,
      returnFocus: trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
    });
    writeStoredPanelOpen(true);
  }
  /**
   * A file opens as its own tab (design §8: files per opened file), and the
   * panel opens with it so a finding deep-link never lands behind a closed panel.
   */
  function openFile(path: string, line?: number | null, trigger?: HTMLElement | null) {
    update({
      collapsed: false,
      tabs: openWorkPanelFileTab(selection.tabs, path),
      target: { kind: "file", path, line: line ?? 1 },
      returnFocus: trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
    });
    writeStoredPanelOpen(true);
  }
  function close() {
    update({ collapsed: true });
    writeStoredPanelOpen(false);
    const trigger = selection.returnFocus;
    // The control that opened the panel can unmount while the panel is open
    // (a settled approval card drops its detail button). Fall back to a stable
    // shell control instead of dropping focus onto the document body.
    const fallback = fallbackRef?.current ?? null;
    const target = trigger?.isConnected ? trigger : fallback;
    requestAnimationFrame(() => { if (target?.isConnected) target.focus(); });
  }
  const setWidth = useCallback(
    (width: number) => {
      // Only the lower bound is enforced here; the live budget caps the render.
      const bounded = clampWorkPanelWidth(width, workPanelMinimumFor(width));
      selections.current.set(key, {
        ...(selections.current.get(key) ?? initialSelection()),
        width: bounded,
      });
      writeStoredPanelWidth(bounded);
      render((value) => value + 1);
    },
    [key],
  );
  return {
    ...selection,
    tabs: selection.tabs.tabs,
    activeId: selection.tabs.activeId,
    maximized: selection.maximized,
    open,
    openFile,
    close,
    openTab: (kind: WorkPanelTabKind) =>
      setTabs(openWorkPanelTab(selection.tabs, kind)),
    activateTab: (id: string) =>
      setTabs(activateWorkPanelTab(selection.tabs, id)),
    closeTab: (id: string) =>
      setTabs(closeWorkPanelTab(selection.tabs, id)),
    replaceTab: (from: string, kind: WorkPanelTabKind) =>
      setTabs(replaceWorkPanelTab(selection.tabs, from, kind)),
    moveTab: (id: string, toIndex: number) =>
      setTabs(moveWorkPanelTab(selection.tabs, id, toIndex)),
    setTarget: (target: WorkPanelTarget) => update({ target }),
    setWidth,
    setMaximized: (maximized: boolean) => update({ maximized }),
    toggleMaximize: () => update({ maximized: !selection.maximized }),
    resizeByKeyboard: (
      event: { key: string; shiftKey: boolean; preventDefault: () => void },
      maxPanelWidth: number,
    ) => {
      const next = workPanelKeyboardWidth(event, selection.width, maxPanelWidth);
      if (next === null || next === selection.width) {
        return;
      }
      event.preventDefault();
      setWidth(next);
    },
    setCollapsed: (collapsed: boolean) => update({ collapsed }),
    /**
     * Re-apply the persisted open preference after a layout-forced close (the
     * narrow drawer hides the panel without touching the preference, so coming
     * back to a wide layout restores what the user actually left).
     */
    restoreOpenPreference: () =>
      update({ collapsed: !(readStoredPanelOpen() ?? false) }),
    toggle: () => selection.collapsed ? open() : close(),
  };
}
export type WorkPanelState = ReturnType<typeof useWorkPanel>;
