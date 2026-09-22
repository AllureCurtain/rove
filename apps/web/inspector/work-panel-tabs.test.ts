import { describe, expect, test } from "vitest";

import {
  activateWorkPanelTab,
  closeWorkPanelTab,
  defaultWorkPanelTabs,
  openWorkPanelTab,
  replaceWorkPanelTab,
  sanitizeWorkPanelTabs,
  workPanelTabStep,
  type WorkPanelTabsState,
} from "./work-panel-tabs";

describe("defaultWorkPanelTabs", () => {
  test("starts on the run status", () => {
    expect(defaultWorkPanelTabs()).toEqual({
      tabs: [{ kind: "status" }],
      activeKind: "status",
    });
  });
});

describe("openWorkPanelTab", () => {
  test("appends a new tab and activates it", () => {
    const state = openWorkPanelTab(defaultWorkPanelTabs(), "files");
    expect(state.tabs.map((tab) => tab.kind)).toEqual(["status", "files"]);
    expect(state.activeKind).toBe("files");
  });

  test("reuses an already open tab instead of duplicating it", () => {
    const once = openWorkPanelTab(defaultWorkPanelTabs(), "files");
    const twice = openWorkPanelTab(once, "files");
    expect(twice.tabs).toHaveLength(2);
    expect(twice.activeKind).toBe("files");
  });
});

describe("closeWorkPanelTab", () => {
  test("activates the next tab, else the previous one", () => {
    const state: WorkPanelTabsState = {
      tabs: [{ kind: "pending" }, { kind: "status" }, { kind: "files" }],
      activeKind: "status",
    };
    expect(closeWorkPanelTab(state, "status")).toEqual({
      tabs: [{ kind: "pending" }, { kind: "files" }],
      activeKind: "files",
    });
    expect(closeWorkPanelTab({ ...state, activeKind: "files" }, "files")).toEqual({
      tabs: [{ kind: "pending" }, { kind: "status" }],
      activeKind: "status",
    });
  });

  test("closing the last tab leaves an empty strip", () => {
    expect(closeWorkPanelTab(defaultWorkPanelTabs(), "status")).toEqual({
      tabs: [],
      activeKind: null,
    });
  });

  test("closing an inactive tab keeps the active one", () => {
    const state: WorkPanelTabsState = {
      tabs: [{ kind: "pending" }, { kind: "status" }],
      activeKind: "pending",
    };
    expect(closeWorkPanelTab(state, "status")).toEqual({
      tabs: [{ kind: "pending" }],
      activeKind: "pending",
    });
  });
});

describe("activateWorkPanelTab", () => {
  test("ignores a kind that is not open", () => {
    const state = defaultWorkPanelTabs();
    expect(activateWorkPanelTab(state, "review")).toBe(state);
  });
});

describe("replaceWorkPanelTab", () => {
  test("turns the launcher tab into the selected kind", () => {
    const withLauncher = openWorkPanelTab(defaultWorkPanelTabs(), "new");
    expect(replaceWorkPanelTab(withLauncher, "new", "review")).toEqual({
      tabs: [{ kind: "status" }, { kind: "review" }],
      activeKind: "review",
    });
  });

  test("reuses an open destination instead of stacking duplicates", () => {
    const state = openWorkPanelTab(openWorkPanelTab(defaultWorkPanelTabs(), "new"), "files");
    expect(replaceWorkPanelTab(state, "new", "files")).toEqual({
      tabs: [{ kind: "status" }, { kind: "files" }],
      activeKind: "files",
    });
  });

  test("falls back to opening when the launcher is gone", () => {
    expect(replaceWorkPanelTab(defaultWorkPanelTabs(), "new", "changes")).toEqual({
      tabs: [{ kind: "status" }, { kind: "changes" }],
      activeKind: "changes",
    });
  });
});

describe("sanitizeWorkPanelTabs", () => {
  test("drops unknown kinds and duplicates and repairs the active kind", () => {
    const dirty = {
      tabs: [
        { kind: "status" },
        { kind: "status" },
        { kind: "unknown" },
        { kind: "files" },
      ],
      activeKind: "missing",
    } as unknown as WorkPanelTabsState;
    expect(sanitizeWorkPanelTabs(dirty)).toEqual({
      tabs: [{ kind: "status" }, { kind: "files" }],
      activeKind: "files",
    });
  });

  test("keeps a valid active kind and survives an empty strip", () => {
    expect(
      sanitizeWorkPanelTabs({ tabs: [{ kind: "review" }], activeKind: "review" }),
    ).toEqual({ tabs: [{ kind: "review" }], activeKind: "review" });
    expect(sanitizeWorkPanelTabs({ tabs: [], activeKind: "review" })).toEqual({
      tabs: [],
      activeKind: null,
    });
  });
});

describe("workPanelTabStep", () => {
  test("wraps around and supports Home/End", () => {
    expect(workPanelTabStep({ key: "ArrowLeft" }, 0, 3)).toBe(2);
    expect(workPanelTabStep({ key: "ArrowRight" }, 2, 3)).toBe(0);
    expect(workPanelTabStep({ key: "Home" }, 2, 3)).toBe(0);
    expect(workPanelTabStep({ key: "End" }, 0, 3)).toBe(2);
    expect(workPanelTabStep({ key: "Tab" }, 0, 3)).toBeNull();
    expect(workPanelTabStep({ key: "ArrowLeft" }, 0, 0)).toBeNull();
    expect(workPanelTabStep({ key: "ArrowLeft" }, -1, 3)).toBeNull();
  });
});
