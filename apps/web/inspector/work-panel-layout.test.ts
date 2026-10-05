import { describe, expect, test } from "vitest";

import {
  MAIN_PANE_MIN_WIDTH,
  WORK_PANEL_COMPACT_MIN_WIDTH,
  WORK_PANEL_DEFAULT_WIDTH,
  WORK_PANEL_MIN_WIDTH,
  WORK_PANEL_STORAGE_MAX_WIDTH,
  clampWorkPanelWidth,
  parseStoredWorkPanelWidth,
  workPanelKeyboardWidth,
  workPanelLayout,
  workPanelMinimumFor,
  workPanelWidthForSidebarReopen,
} from "./work-panel-layout";

describe("workPanelLayout", () => {
  test("a wide window keeps the requested width and never exceeds the budget", () => {
    const layout = workPanelLayout({
      containerWidth: 1440,
      sidebarWidth: 240,
      sidebarCollapsed: false,
      requestedPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
    });

    expect(layout.panelWidth).toBe(WORK_PANEL_DEFAULT_WIDTH);
    expect(layout.mainWidth).toBe(1440 - 240 - WORK_PANEL_DEFAULT_WIDTH);
    expect(layout.maxPanelWidth).toBe(1440 - 240 - MAIN_PANE_MIN_WIDTH);
    expect(layout.shouldCollapseSidebar).toBe(false);
  });

  test("a narrow window caps the panel and asks the rail to yield first", () => {
    // 1024 − 240 − 360 = 424 < 450: the conversation floor would break, so the
    // rail is asked to collapse rather than the chat being squeezed.
    const withRail = workPanelLayout({
      containerWidth: 1024,
      sidebarWidth: 240,
      sidebarCollapsed: false,
      requestedPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
    });
    expect(withRail.maxPanelWidth).toBe(1024 - 240 - MAIN_PANE_MIN_WIDTH);
    expect(withRail.panelWidth).toBe(1024 - 240 - MAIN_PANE_MIN_WIDTH);
    expect(withRail.mainWidth).toBe(MAIN_PANE_MIN_WIDTH);
    expect(withRail.shouldCollapseSidebar).toBe(true);

    // After the rail yields, the panel gets its request back and the
    // conversation keeps more than the floor.
    const withoutRail = workPanelLayout({
      containerWidth: 1024,
      sidebarWidth: 240,
      sidebarCollapsed: true,
      requestedPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
    });
    expect(withoutRail.panelWidth).toBe(WORK_PANEL_DEFAULT_WIDTH);
    expect(withoutRail.mainWidth).toBe(1024 - WORK_PANEL_DEFAULT_WIDTH);
    expect(withoutRail.shouldCollapseSidebar).toBe(false);
  });

  test("maximized hands the conversation width to the panel", () => {
    const layout = workPanelLayout({
      containerWidth: 1440,
      sidebarWidth: 240,
      sidebarCollapsed: false,
      requestedPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
      maximized: true,
    });

    expect(layout.mainWidth).toBe(0);
    expect(layout.panelWidth).toBe(1200);
    expect(layout.shouldCollapseSidebar).toBe(false);
  });

  test("a collapsed rail never asks to collapse again", () => {
    const layout = workPanelLayout({
      containerWidth: 900,
      sidebarWidth: 240,
      sidebarCollapsed: true,
      requestedPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
    });

    expect(layout.shouldCollapseSidebar).toBe(false);
    expect(layout.panelWidth).toBe(
      Math.min(WORK_PANEL_DEFAULT_WIDTH, 900 - MAIN_PANE_MIN_WIDTH),
    );
  });

  test("a non-finite measurement degrades to no space instead of NaN", () => {
    const layout = workPanelLayout({
      containerWidth: Number.NaN,
      sidebarWidth: Number.NaN,
      sidebarCollapsed: false,
      requestedPanelWidth: Number.NaN,
    });

    expect(Number.isFinite(layout.panelWidth)).toBe(true);
    expect(layout.panelWidth).toBe(0);
    expect(layout.mainWidth).toBe(0);
    expect(layout.shouldCollapseSidebar).toBe(true);
  });
});

describe("clampWorkPanelWidth", () => {
  test("keeps compact widths compact and enforces the regular minimum", () => {
    expect(clampWorkPanelWidth(10, workPanelMinimumFor(10))).toBe(10);
    expect(
      clampWorkPanelWidth(10, workPanelMinimumFor(WORK_PANEL_MIN_WIDTH)),
    ).toBe(WORK_PANEL_MIN_WIDTH);
    expect(clampWorkPanelWidth(WORK_PANEL_DEFAULT_WIDTH)).toBe(
      WORK_PANEL_DEFAULT_WIDTH,
    );
    // No fixed upper clamp: the live budget does that job.
    expect(clampWorkPanelWidth(4000)).toBe(4000);
  });
});

describe("workPanelWidthForSidebarReopen", () => {
  test("preserves the conversation width when there is room", () => {
    expect(
      workPanelWidthForSidebarReopen({
        containerWidth: 1920,
        sidebarWidth: 240,
        currentPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
      }),
    ).toBe(WORK_PANEL_DEFAULT_WIDTH);
  });

  test("shrinks the panel to the reopen target when the floor would break", () => {
    // 1024 − 240 = 784 remaining; keeping the 360 panel would leave 424 < 450,
    // so the panel shrinks to leave the 460 reopen target for the conversation.
    expect(
      workPanelWidthForSidebarReopen({
        containerWidth: 1024,
        sidebarWidth: 240,
        currentPanelWidth: WORK_PANEL_DEFAULT_WIDTH,
      }),
    ).toBe(1024 - 240 - (MAIN_PANE_MIN_WIDTH + 10));
  });
});

describe("workPanelKeyboardWidth", () => {
  test("mirrors the rail handle steps and stops at the bounds", () => {
    expect(
      workPanelKeyboardWidth({ key: "ArrowLeft", shiftKey: false }, 360, 750),
    ).toBe(376);
    expect(
      workPanelKeyboardWidth({ key: "ArrowRight", shiftKey: false }, 360, 750),
    ).toBe(344);
    expect(
      workPanelKeyboardWidth({ key: "ArrowLeft", shiftKey: true }, 360, 750),
    ).toBe(392);
    expect(
      workPanelKeyboardWidth({ key: "ArrowRight", shiftKey: true }, 360, 750),
    ).toBe(328);
    expect(workPanelKeyboardWidth({ key: "Home", shiftKey: false }, 360, 750)).toBe(
      WORK_PANEL_MIN_WIDTH,
    );
    expect(workPanelKeyboardWidth({ key: "End", shiftKey: false }, 360, 750)).toBe(750);
    // The minimum wins when the live budget is smaller than it.
    expect(
      workPanelKeyboardWidth(
        { key: "ArrowRight", shiftKey: false },
        WORK_PANEL_MIN_WIDTH,
        750,
      ),
    ).toBe(WORK_PANEL_MIN_WIDTH);
    expect(workPanelKeyboardWidth({ key: "End", shiftKey: false }, 360, 100)).toBe(
      WORK_PANEL_MIN_WIDTH,
    );
    expect(workPanelKeyboardWidth({ key: "Tab", shiftKey: false }, 360, 750)).toBeNull();
  });
});

describe("parseStoredWorkPanelWidth", () => {
  test("rejects unusable values and bounds the stored one", () => {
    expect(parseStoredWorkPanelWidth("")).toBeNull();
    expect(parseStoredWorkPanelWidth("abc")).toBeNull();
    expect(parseStoredWorkPanelWidth("0")).toBeNull();
    expect(parseStoredWorkPanelWidth("-40")).toBeNull();
    expect(parseStoredWorkPanelWidth("360")).toBe(360);
    expect(parseStoredWorkPanelWidth("1")).toBe(WORK_PANEL_COMPACT_MIN_WIDTH);
    expect(parseStoredWorkPanelWidth("99999")).toBe(WORK_PANEL_STORAGE_MAX_WIDTH);
  });
});
