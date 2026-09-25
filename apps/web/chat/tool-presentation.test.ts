import { describe, expect, it } from "vitest";

import type { ToolCallView } from "../lib/rove-state";
import { buildToolPresentation, trimDiff } from "./tool-presentation";

function tool(overrides: Partial<ToolCallView>): ToolCallView {
  return {
    id: "call-1",
    name: "read",
    status: "done",
    details: "",
    ...overrides,
  };
}

describe("buildToolPresentation", () => {
  it("computes no body blocks while the card is collapsed", () => {
    const presentation = buildToolPresentation(
      tool({ output: "fn main() {}" }),
      false,
    );
    expect(presentation.blocks).toEqual([]);
    expect(presentation.title).toBe("read");
  });

  it("renders a mutation as a diff block", () => {
    const presentation = buildToolPresentation(
      tool({
        name: "edit",
        mutations: [{ path: "src/main.rs", operation: "update", diff: "@@ -1 +1 @@\n-old\n+new" }],
      }),
      true,
    );
    const block = presentation.blocks[0]!;
    expect(block.kind).toBe("diff");
    if (block.kind === "diff") {
      expect(block.path).toBe("src/main.rs");
      expect(block.truncated).toBe(false);
    }
  });

  it("groups search hits into matches", () => {
    const presentation = buildToolPresentation(
      tool({
        name: "search",
        metadata: {
          status: "ok",
          risk_level: "low",
          read_only: true,
          workspace_changed: false,
          affected_paths: [],
          diff_summary: ["src/a.rs:12:let value", "src/b.rs:4:let value"],
        },
      }),
      true,
    );
    const block = presentation.blocks.find((candidate) => candidate.kind === "matches");
    expect(block && block.kind === "matches" ? block.entries : []).toEqual([
      { path: "src/a.rs", line: 12, text: "let value" },
      { path: "src/b.rs", line: 4, text: "let value" },
    ]);
  });

  it("lists affected paths when nothing was mutated", () => {
    const presentation = buildToolPresentation(
      tool({
        metadata: {
          status: "ok",
          risk_level: "low",
          read_only: true,
          workspace_changed: false,
          affected_paths: ["src/a.rs", "src/b.rs"],
          diff_summary: [],
        },
      }),
      true,
    );
    const block = presentation.blocks.find((candidate) => candidate.kind === "files");
    expect(block && block.kind === "files" ? block.entries.map((entry) => entry.path) : []).toEqual([
      "src/a.rs",
      "src/b.rs",
    ]);
  });

  it("drops summary lines that carry a secret", () => {
    const presentation = buildToolPresentation(
      tool({
        metadata: {
          status: "ok",
          risk_level: "low",
          read_only: true,
          workspace_changed: false,
          affected_paths: [],
          diff_summary: ["authorization: Bearer secret", "renamed a field"],
        },
      }),
      true,
    );
    const fields = presentation.blocks.find((candidate) => candidate.kind === "fields");
    const values = fields && fields.kind === "fields" ? fields.rows.map((row) => row.value) : [];
    expect(values).toEqual(["renamed a field"]);
  });

  it("falls back to a note for a tool with nothing structured", () => {
    const presentation = buildToolPresentation(tool({ details: "no output" }), true);
    expect(presentation.blocks).toEqual([{ kind: "note", text: "no output" }]);
  });

  it("marks a failed tool as danger", () => {
    const presentation = buildToolPresentation(tool({ status: "error" }), false);
    expect(presentation.chips[0]).toEqual({ label: "error", tone: "danger" });
  });
});

describe("trimDiff", () => {
  it("returns a short diff unchanged", () => {
    const diff = "@@ -1 +1 @@\n-old\n+new";
    expect(trimDiff(diff)).toEqual({ text: diff, truncated: false });
  });

  it("keeps two lines of context around changes and drops the rest", () => {
    const lines = ["@@ -1,600 +1,600 @@"];
    for (let index = 0; index < 600; index += 1) {
      lines.push(index === 300 ? "+changed" : " context");
    }
    const trimmed = trimDiff(lines.join("\n"));
    expect(trimmed.truncated).toBe(true);
    expect(trimmed.text).toContain("+changed");
    expect(trimmed.text).toContain("lines omitted");
    expect(trimmed.text.split("\n").length).toBeLessThan(lines.length);
  });
});
