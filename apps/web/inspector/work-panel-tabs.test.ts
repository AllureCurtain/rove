import { describe, expect, test } from "vitest";

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
  workPanelTabStep,
  type WorkPanelTabsState,
} from "./work-panel-tabs";

describe("defaultWorkPanelTabs", () => {
  test("starts on the evidence view, not the status dump (design §2.2)", () => {
    expect(defaultWorkPanelTabs()).toEqual({
      tabs: [{ id: "changes", kind: "changes" }],
      activeId: "changes",
    });
  });
});

describe("openWorkPanelTab", () => {
  test("appends a new tab and activates it", () => {
    const state = openWorkPanelTab(defaultWorkPanelTabs(), "files");
    expect(state.tabs.map((tab) => tab.id)).toEqual(["changes", "files"]);
    expect(state.activeId).toBe("files");
  });

  test("reuses an already open tab instead of duplicating it", () => {
    const once = openWorkPanelTab(defaultWorkPanelTabs(), "files");
    const twice = openWorkPanelTab(once, "files");
    expect(twice.tabs).toHaveLength(2);
    expect(twice.activeId).toBe("files");
  });
});

describe("openWorkPanelFileTab", () => {
  test("every opened file is its own tab (design §8)", () => {
    const state = openWorkPanelFileTab(
      openWorkPanelFileTab(defaultWorkPanelTabs(), "src/a.ts"),
      "src/b.ts",
    );
    expect(state.tabs).toEqual([
      { id: "changes", kind: "changes" },
      { id: "file:src/a.ts", kind: "file", path: "src/a.ts" },
      { id: "file:src/b.ts", kind: "file", path: "src/b.ts" },
    ]);
    expect(state.activeId).toBe("file:src/b.ts");
  });

  test("reopening the same file reuses its tab", () => {
    const state = openWorkPanelFileTab(
      openWorkPanelFileTab(defaultWorkPanelTabs(), "src/a.ts"),
      "src/a.ts",
    );
    expect(state.tabs).toHaveLength(2);
    expect(state.activeId).toBe("file:src/a.ts");
  });
});

describe("closeWorkPanelTab", () => {
  test("activates the next tab, else the previous one", () => {
    const state: WorkPanelTabsState = {
      tabs: [
        { id: "pending", kind: "pending" },
        { id: "status", kind: "status" },
        { id: "files", kind: "files" },
      ],
      activeId: "status",
    };
    expect(closeWorkPanelTab(state, "status")).toEqual({
      tabs: [
        { id: "pending", kind: "pending" },
        { id: "files", kind: "files" },
      ],
      activeId: "files",
    });
    expect(closeWorkPanelTab({ ...state, activeId: "files" }, "files")).toEqual({
      tabs: [
        { id: "pending", kind: "pending" },
        { id: "status", kind: "status" },
      ],
      activeId: "status",
    });
  });

  test("closing the last tab leaves an empty strip", () => {
    expect(closeWorkPanelTab(defaultWorkPanelTabs(), "changes")).toEqual({
      tabs: [],
      activeId: null,
    });
  });

  test("closing an inactive tab keeps the active one", () => {
    const state: WorkPanelTabsState = {
      tabs: [
        { id: "pending", kind: "pending" },
        { id: "status", kind: "status" },
      ],
      activeId: "pending",
    };
    expect(closeWorkPanelTab(state, "status")).toEqual({
      tabs: [{ id: "pending", kind: "pending" }],
      activeId: "pending",
    });
  });
});

describe("activateWorkPanelTab", () => {
  test("ignores a tab that is not open", () => {
    const state = defaultWorkPanelTabs();
    expect(activateWorkPanelTab(state, "review")).toBe(state);
  });
});

describe("replaceWorkPanelTab", () => {
  test("turns the launcher tab into the selected kind", () => {
    const withLauncher = openWorkPanelTab(defaultWorkPanelTabs(), "new");
    expect(replaceWorkPanelTab(withLauncher, "new", "review")).toEqual({
      tabs: [
        { id: "changes", kind: "changes" },
        { id: "review", kind: "review" },
      ],
      activeId: "review",
    });
  });

  test("reuses an open destination instead of stacking duplicates", () => {
    const state = openWorkPanelTab(openWorkPanelTab(defaultWorkPanelTabs(), "new"), "files");
    expect(replaceWorkPanelTab(state, "new", "files")).toEqual({
      tabs: [
        { id: "changes", kind: "changes" },
        { id: "files", kind: "files" },
      ],
      activeId: "files",
    });
  });

  test("falls back to opening when the launcher is gone", () => {
    expect(replaceWorkPanelTab(defaultWorkPanelTabs(), "new", "files")).toEqual({
      tabs: [
        { id: "changes", kind: "changes" },
        { id: "files", kind: "files" },
      ],
      activeId: "files",
    });
  });
});

describe("moveWorkPanelTab", () => {
  const state: WorkPanelTabsState = {
    tabs: [
      { id: "changes", kind: "changes" },
      { id: "files", kind: "files" },
      { id: "file:src/a.ts", kind: "file", path: "src/a.ts" },
    ],
    activeId: "files",
  };

  test("reorders to an absolute index and keeps the active tab", () => {
    expect(moveWorkPanelTab(state, "files", 2).tabs.map((tab) => tab.id)).toEqual([
      "changes",
      "file:src/a.ts",
      "files",
    ]);
    expect(moveWorkPanelTab(state, "files", 2).activeId).toBe("files");
    expect(moveWorkPanelTab(state, "file:src/a.ts", 0).tabs[0].id).toBe(
      "file:src/a.ts",
    );
  });

  test("clamps out-of-range indices and no-ops identity moves", () => {
    expect(moveWorkPanelTab(state, "files", -5).tabs[0].id).toBe("files");
    expect(moveWorkPanelTab(state, "files", 99).tabs.at(-1)?.id).toBe("files");
    expect(moveWorkPanelTab(state, "files", 1)).toBe(state);
    expect(moveWorkPanelTab(state, "missing", 0)).toBe(state);
  });
});

describe("sanitizeWorkPanelTabs", () => {
  test("drops unknown kinds, duplicates, and malformed file tabs", () => {
    const dirty = {
      tabs: [
        { id: "status", kind: "status" },
        { id: "status", kind: "status" },
        { id: "bogus", kind: "unknown" },
        { id: "files", kind: "files" },
        { id: "file:", kind: "file" },
      ],
      activeId: "missing",
    } as unknown as WorkPanelTabsState;
    expect(sanitizeWorkPanelTabs(dirty)).toEqual({
      tabs: [
        { id: "status", kind: "status" },
        { id: "files", kind: "files" },
      ],
      activeId: "files",
    });
  });

  test("keeps a valid active id and survives an empty strip", () => {
    expect(
      sanitizeWorkPanelTabs({
        tabs: [{ id: "review", kind: "review" }],
        activeId: "review",
      }),
    ).toEqual({ tabs: [{ id: "review", kind: "review" }], activeId: "review" });
    expect(sanitizeWorkPanelTabs({ tabs: [], activeId: "review" })).toEqual({
      tabs: [],
      activeId: null,
    });
  });
});

describe("serializeWorkPanelTabs / parseStoredWorkPanelTabs", () => {
  test("round-trips order, active tab, and file paths", () => {
    const state = openWorkPanelFileTab(
      openWorkPanelTab(defaultWorkPanelTabs(), "files"),
      "src/deep/file.md",
    );
    expect(parseStoredWorkPanelTabs(serializeWorkPanelTabs(state))).toEqual(state);
  });

  test("rejects malformed payloads and repairs stale ids", () => {
    expect(parseStoredWorkPanelTabs("not json")).toBeNull();
    expect(parseStoredWorkPanelTabs('{"tabs":"nope"}')).toBeNull();
    expect(parseStoredWorkPanelTabs(null)).toBeNull();
    const stale = JSON.stringify({
      tabs: [{ kind: "files" }, { kind: "bogus" }],
      activeId: "bogus",
    });
    expect(parseStoredWorkPanelTabs(stale)).toEqual({
      tabs: [{ id: "files", kind: "files" }],
      activeId: "files",
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
