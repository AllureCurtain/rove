import { describe, expect, it } from "vitest";

import {
  hasFileReference,
  parseFileReference,
  segmentFileReferences,
} from "./file-ref";

describe("parseFileReference", () => {
  it("parses a bare workspace-relative path", () => {
    expect(parseFileReference("src/main.rs")).toEqual({ path: "src/main.rs" });
  });

  it("parses path:line:column", () => {
    expect(parseFileReference("src/main.rs:12:4")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: 4,
    });
  });

  it("parses path:line without a column", () => {
    expect(parseFileReference("docs/guide.md:7")).toEqual({
      path: "docs/guide.md",
      line: 7,
    });
  });

  it("strips a file: prefix", () => {
    expect(parseFileReference("file:src/a.ts:3")).toEqual({
      path: "src/a.ts",
      line: 3,
    });
  });

  it("rejects absolute and home paths", () => {
    expect(parseFileReference("/etc/passwd")).toBeNull();
    expect(parseFileReference("~/secret.txt")).toBeNull();
    expect(parseFileReference("C:/Windows/system.ini")).toBeNull();
  });

  it("rejects traversal and empty paths", () => {
    expect(parseFileReference("../outside.ts")).toBeNull();
    expect(parseFileReference("a/../../b")).toBeNull();
    expect(parseFileReference("file:")).toBeNull();
  });
});

describe("segmentFileReferences", () => {
  it("passes plain text through untouched", () => {
    expect(segmentFileReferences("no refs here")).toEqual([
      { kind: "text", text: "no refs here" },
    ]);
  });

  it("segments a file ref inside prose", () => {
    expect(segmentFileReferences("see src/main.rs:10 for details")).toEqual([
      { kind: "text", text: "see " },
      {
        kind: "file",
        text: "src/main.rs:10",
        reference: { path: "src/main.rs", line: 10 },
      },
      { kind: "text", text: " for details" },
    ]);
  });

  it("segments a URL", () => {
    const segments = segmentFileReferences("docs at https://example.com/a/b now");
    expect(segments[0]).toEqual({ kind: "text", text: "docs at " });
    expect(segments[1]).toMatchObject({
      kind: "url",
      url: "https://example.com/a/b",
    });
    expect(segments[2]).toEqual({ kind: "text", text: " now" });
  });

  it("reconstructs the input when joined", () => {
    const text = "edited src/a.ts then read https://x.test/p?q=1 and dir/b.md:2";
    expect(segmentFileReferences(text).map((s) => s.text).join("")).toBe(text);
  });

  it("does not split a bare filename without a slash", () => {
    expect(segmentFileReferences("edited file.txt")).toEqual([
      { kind: "text", text: "edited file.txt" },
    ]);
  });
});

describe("hasFileReference", () => {
  it("is true for a string with a linkable span", () => {
    expect(hasFileReference("wrote src/main.rs")).toBe(true);
  });

  it("is false for plain text", () => {
    expect(hasFileReference("nothing linkable")).toBe(false);
  });
});
