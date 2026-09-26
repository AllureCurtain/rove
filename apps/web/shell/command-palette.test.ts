import { describe, expect, it } from "vitest";

import {
  COMMAND_PALETTE_GROUP_LIMIT,
  COMMAND_PALETTE_GROUP_ORDER,
  buildCommandGroups,
  firstEnabledCommandId,
  flattenCommandGroups,
  matchCommandEntry,
  mergeHighlightRanges,
  moveCommandSelection,
  normalizeCommandText,
  splitCommandHighlight,
  tokenizeCommandQuery,
  type CommandPaletteEntry,
} from "./command-palette";

function entry(
  id: string,
  title: string,
  overrides: Partial<CommandPaletteEntry> = {},
): CommandPaletteEntry {
  return {
    id,
    groupKey: "actions",
    title,
    order: 0,
    action: { kind: "toggle-theme" },
    ...overrides,
  };
}

describe("command palette query", () => {
  it("normalizes case and whitespace, and keeps an empty query empty", () => {
    expect(normalizeCommandText("  Open   Settings ")).toBe("open settings");
    expect(tokenizeCommandQuery("   ")).toEqual([]);
    expect(tokenizeCommandQuery("外观   语言")).toEqual(["外观", "语言"]);
  });
});

describe("command palette matching", () => {
  it("matches substrings, not subsequences", () => {
    expect(matchCommandEntry({ title: "外观 · 语言" }, ["语言"])).not.toBeNull();
    // The reference's CJK case: a subsequence would match, and that noise is
    // exactly why the palette uses substring semantics.
    expect(matchCommandEntry({ title: "设计与配置" }, ["设置"])).toBeNull();
  });

  it("requires every token to hit, in any order", () => {
    const target = { title: "外观 · 语言", subtitle: "主题与语言" };
    expect(matchCommandEntry(target, ["语言", "外观"])).not.toBeNull();
    expect(matchCommandEntry(target, ["外观", "主题"])).not.toBeNull();
    expect(matchCommandEntry(target, ["外观", "不存在"])).toBeNull();
  });

  it("scores a title hit above a subtitle-only hit", () => {
    const inTitle = matchCommandEntry({ title: "设置", subtitle: "x" }, ["设置"]);
    const inSubtitle = matchCommandEntry({ title: "x", subtitle: "设置" }, [
      "设置",
    ]);
    expect(inTitle!.score).toBeGreaterThan(inSubtitle!.score);
  });

  it("scores a prefix above a word start above a mid-word hit", () => {
    const prefix = matchCommandEntry({ title: "设置" }, ["设置"])!;
    const wordStart = matchCommandEntry({ title: "打开 设置" }, ["设置"])!;
    const anywhere = matchCommandEntry({ title: "打开设置面板" }, ["设置"])!;
    expect(prefix.score).toBeGreaterThan(wordStart.score);
    expect(wordStart.score).toBeGreaterThan(anywhere.score);
  });

  it("merges overlapping ranges so highlights never nest", () => {
    expect(
      mergeHighlightRanges([
        { start: 0, end: 3 },
        { start: 2, end: 5 },
        { start: 8, end: 9 },
      ]),
    ).toEqual([
      { start: 0, end: 5 },
      { start: 8, end: 9 },
    ]);
  });

  it("splits a title into runs without losing text", () => {
    const runs = splitCommandHighlight("打开设置面板", [{ start: 2, end: 4 }]);
    expect(runs.map((run) => run.text).join("")).toBe("打开设置面板");
    expect(runs.filter((run) => run.highlighted).map((run) => run.text)).toEqual([
      "设置",
    ]);
    expect(splitCommandHighlight("设置", [])).toEqual([
      { text: "设置", highlighted: false },
    ]);
  });
});

describe("command palette grouping", () => {
  it("keeps the constant group order instead of ranking across groups", () => {
    const entries = [
      entry("action-1", "切换主题"),
      entry("session-1", "主题讨论", { groupKey: "sessions" }),
    ];
    const groups = buildCommandGroups(entries, "主题");
    // The session entry matches at a higher score, but a session group may never
    // move above actions: a late arrival must not shift the row under the cursor.
    expect(groups.map((group) => group.key)).toEqual(["actions", "sessions"]);
    expect(groups[0]!.items[0]!.entry.id).toBe("action-1");
  });

  it("limits each group and reports what it hid", () => {
    const entries = Array.from({ length: COMMAND_PALETTE_GROUP_LIMIT + 2 }, (_, index) =>
      entry(`action-${index}`, `命令 ${index}`),
    );
    const [group] = buildCommandGroups(entries, "命令");
    expect(group!.items).toHaveLength(COMMAND_PALETTE_GROUP_LIMIT);
    expect(group!.hiddenCount).toBe(2);
  });

  it("drops groups that matched nothing", () => {
    const groups = buildCommandGroups([entry("a", "主题")], "设置");
    expect(groups).toEqual([]);
  });

  it("keeps the builder order for an empty query", () => {
    const entries = [
      entry("b", "第二个", { order: 2 }),
      entry("a", "第一个", { order: 1 }),
    ];
    const [group] = buildCommandGroups(entries, "");
    expect(group!.items.map((item) => item.entry.id)).toEqual(["a", "b"]);
  });
});

describe("command palette selection", () => {
  it("walks enabled entries only, wrapping at both ends", () => {
    const groups = buildCommandGroups(
      [
        // Distinct orders: equal scores and equal orders would fall through to
        // title collation, which is not what this case is about.
        entry("a", "一", { order: 0 }),
        entry("b", "二", { order: 1, disabled: true }),
        entry("c", "三", { order: 2 }),
      ],
      "",
    );
    const items = flattenCommandGroups(groups);
    expect(items.map((item) => item.entry.id)).toEqual(["a", "b", "c"]);
    expect(firstEnabledCommandId(items)).toBe("a");
    expect(moveCommandSelection(items, "a", 1)).toBe("c");
    expect(moveCommandSelection(items, "c", 1)).toBe("a");
    expect(moveCommandSelection(items, "a", -1)).toBe("c");
  });

  it("anchors on the id, so a vanished selection falls back instead of drifting", () => {
    const groups = buildCommandGroups([entry("a", "一")], "");
    const items = flattenCommandGroups(groups);
    expect(moveCommandSelection(items, "gone", 1)).toBe("a");
    expect(moveCommandSelection([], "a", 1)).toBeNull();
  });

  it("exposes the group order it shipped with", () => {
    expect(COMMAND_PALETTE_GROUP_ORDER).toEqual([
      "actions",
      "workspaces",
      "sessions",
      "settings",
    ]);
  });
});
