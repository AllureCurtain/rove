import { describe, expect, it } from "vitest";

import { segmentMarkdown } from "./streaming-blocks";

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
