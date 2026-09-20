"use client";

import { useRef, useState, type RefObject } from "react";

export type WorkPanelTab = "run" | "review" | "approval";
export type WorkPanelTarget =
  | { kind: "approval"; jobId: string; runId: string; callId: string }
  | { kind: "file"; path: string; line: number }
  | null;
interface PanelSelection {
  tab: WorkPanelTab;
  target: WorkPanelTarget;
  width: number;
  collapsed: boolean;
  returnFocus: HTMLElement | null;
}
export const PANEL_MIN_WIDTH = 280;
export const PANEL_MAX_WIDTH = 560;
export function boundPanelWidth(width: number) {
  return Math.min(PANEL_MAX_WIDTH, Math.max(PANEL_MIN_WIDTH, Number.isFinite(width) ? width : 360));
}
const initialSelection = (): PanelSelection => ({
  tab: "run", target: null, width: 360,
  collapsed: typeof window === "undefined" || window.matchMedia("(max-width: 960px)").matches,
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
  function update(patch: Partial<PanelSelection>) {
    // Bound the UI cache; evicting a selection never changes runtime state.
    if (!selections.current.has(key) && selections.current.size >= 64) {
      selections.current.delete(selections.current.keys().next().value!);
    }
    selections.current.set(key, { ...(selections.current.get(key) ?? initialSelection()), ...patch });
    render((value) => value + 1);
  }
  function open(tab = selection.tab, target = selection.target, trigger?: HTMLElement | null) {
    update({ collapsed: false, tab, target,
      returnFocus: trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null) });
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
  return {
    ...selection, open, close,
    setTab: (tab: WorkPanelTab) => update({ tab }),
    setTarget: (target: WorkPanelTarget) => update({ target }),
    setWidth: (width: number) => update({ width: boundPanelWidth(width) }),
    setCollapsed: (collapsed: boolean) => update({ collapsed }),
    toggle: () => selection.collapsed ? open() : close(),
  };
}
export type WorkPanelState = ReturnType<typeof useWorkPanel>;
