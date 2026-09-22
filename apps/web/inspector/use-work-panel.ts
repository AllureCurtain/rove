"use client";

import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

import {
  WORK_PANEL_DEFAULT_WIDTH,
  clampWorkPanelWidth,
  parseStoredWorkPanelWidth,
  workPanelKeyboardWidth,
  workPanelMinimumFor,
} from "./work-panel-layout";

import { matchesDrawerLayout } from "../lib/viewport-breakpoints";
import {
  activateWorkPanelTab,
  closeWorkPanelTab,
  defaultWorkPanelTabs,
  openWorkPanelTab,
  replaceWorkPanelTab,
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
  returnFocus: HTMLElement | null;
}

/**
 * Panel width as a UI preference (design §3 ownership table: layout values live
 * in localStorage, not in the product preference contract). The stored value is
 * a *request*; the rendered width is always the live budget's answer
 * (`workPanelLayout`), so a wide stored value can never squeeze the chat.
 */
const STORAGE_KEY = "rove.ui-work-panel-width";

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
  collapsed: matchesDrawerLayout(),
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

  // Restore after mount so the server and the first client render agree.
  useEffect(() => {
    const stored = readStoredPanelWidth();
    if (stored === WORK_PANEL_DEFAULT_WIDTH) {
      return;
    }
    const current = selections.current.get(key);
    if (current && current.width === stored) {
      return;
    }
    selections.current.set(key, { ...(current ?? initialSelection()), width: stored });
    render((value) => value + 1);
  }, [key]);

  function update(patch: Partial<PanelSelection>) {
    // Bound the UI cache; evicting a selection never changes runtime state.
    if (!selections.current.has(key) && selections.current.size >= 64) {
      selections.current.delete(selections.current.keys().next().value!);
    }
    selections.current.set(key, { ...(selections.current.get(key) ?? initialSelection()), ...patch });
    render((value) => value + 1);
  }
  function open(kind = selection.tabs.activeKind ?? "status", target = selection.target, trigger?: HTMLElement | null) {
    update({
      collapsed: false,
      tabs: openWorkPanelTab(selection.tabs, kind),
      target,
      returnFocus: trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
    });
  }
  function close() {
    update({ collapsed: true });
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
    activeKind: selection.tabs.activeKind,
    open,
    close,
    openTab: (kind: WorkPanelTabKind) =>
      update({ tabs: openWorkPanelTab(selection.tabs, kind) }),
    activateTab: (kind: WorkPanelTabKind) =>
      update({ tabs: activateWorkPanelTab(selection.tabs, kind) }),
    closeTab: (kind: WorkPanelTabKind) =>
      update({ tabs: closeWorkPanelTab(selection.tabs, kind) }),
    replaceTab: (from: WorkPanelTabKind, kind: WorkPanelTabKind) =>
      update({ tabs: replaceWorkPanelTab(selection.tabs, from, kind) }),
    setTarget: (target: WorkPanelTarget) => update({ target }),
    setWidth,
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
    toggle: () => selection.collapsed ? open() : close(),
  };
}
export type WorkPanelState = ReturnType<typeof useWorkPanel>;
