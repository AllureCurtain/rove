import { describe, expect, it } from "vitest";

import { markdownSegmentPropsEqual, segmentMarkdown } from "./streaming-blocks";

describe("segmentMarkdown", () => {
  it("keeps plain prose as one segment", () => {
    expect(segmentMarkdown("hello\nworld")).toEqual([{ kind: "prose", text: "hello\nworld" }]);
  });

  it("keeps an unclosed fence as prose so the text after it stays visible", () => {
    const segments = segmentMarkdown("before\n```rust\nfn main() {}\nstill coming");
    expect(segments.every((segment) => segment.kind === "prose")).toBe(true);
    expect(segments.map((segment) => segment.text).join("\n")).toContain("still coming");
  });

  it("splits a closed fence out as code and keeps the prose around it", () => {
    const segments = segmentMarkdown("intro\n```rust\nfn main() {}\n```\nafter");
    expect(segments.map((segment) => segment.kind)).toEqual(["prose", "code", "prose"]);
    expect(segments[1]?.text).toBe("```rust\nfn main() {}\n```");
    expect(segments[2]?.text).toBe("after");
  });

  it("ignores indented fences", () => {
    const segments = segmentMarkdown("    ```\n    not a fence\n    ```");
    expect(segments).toHaveLength(1);
    expect(segments[0]?.kind).toBe("prose");
  });

  it("does not let a tilde fence close a backtick fence", () => {
    const segments = segmentMarkdown("```\ncode\n~~~");
    expect(segments).toHaveLength(1);
    expect(segments[0]?.kind).toBe("prose");
  });

  it("handles an empty string", () => {
    expect(segmentMarkdown("")).toEqual([{ kind: "prose", text: "" }]);
  });
});

describe("streaming stability", () => {
  /** The prefix of segments whose text a growing document must not disturb. */
  function stablePrefix(before: string[], after: string[]): number {
    let shared = 0;
    while (shared < before.length && shared < after.length && before[shared] === after[shared]) {
      shared += 1;
    }
    return shared;
  }

  it("leaves every finished segment byte-identical as deltas arrive", () => {
    // Three blocks: prose, a closed fence, and a growing tail.
    const document = [
      "# Result",
      "",
      "First paragraph.",
      "",
      "```rust",
      "fn main() {}",
      "```",
      "",
      "Trailing paragraph that keeps growing",
    ];
    let previous = segmentMarkdown("");
    let previousTexts: string[] = [];
    for (let line = 0; line < document.length; line += 1) {
      const markdown = document.slice(0, line + 1).join("\n");
      const texts = segmentMarkdown(markdown).map((segment) => segment.text);
      const shared = stablePrefix(previousTexts, texts);
      // Only the tail may differ; everything before it is reused verbatim.
      expect(shared).toBeGreaterThanOrEqual(Math.min(previousTexts.length, texts.length) - 1);
      previous = segmentMarkdown(markdown);
      previousTexts = previous.map((segment) => segment.text);
    }
    expect(previousTexts).toHaveLength(3);
  });

  it("rewrites only the block that a closing fence completes", () => {
    const open = segmentMarkdown("intro\n\n```rust\nfn main() {}");
    const closed = segmentMarkdown("intro\n\n```rust\nfn main() {}\n```\n\noutro");
    const shared = stablePrefix(
      open.map((segment) => segment.text),
      closed.map((segment) => segment.text),
    );
    // "intro" survives; the second block changes from prose to code once.
    expect(shared).toBe(1);
  });
});

describe("markdownSegmentPropsEqual", () => {
  it("reuses a segment whose text did not change", () => {
    expect(
      markdownSegmentPropsEqual({ text: "intro\n\nbody" }, { text: "intro\n\nbody" }),
    ).toBe(true);
  });

  it("re-renders the segment that grew", () => {
    expect(markdownSegmentPropsEqual({ text: "body" }, { text: "body and more" })).toBe(false);
  });
});
