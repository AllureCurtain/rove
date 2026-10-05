import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { DiffView } from "./DiffView";

describe("DiffView", () => {
  it("renders headerless canonical diff lines under the real mutation path", () => {
    const html = renderToStaticMarkup(
      <DiffView diff={"+canonical\n context"} label="notes.md diff" sourcePath="notes.md" />,
    );

    expect(html).toContain("notes.md");
    expect(html).toContain("Headerless canonical diff lines");
    expect(html).toContain("synthesized header");
    expect(html).not.toContain("Untitled mutation");
    expect(html).not.toContain('data-fallback="true"');
    expect(html).toContain("+1 / -0");
    expect(html).toContain("+canonical");
    expect(html).toContain("@@ -1,0 +1,1 @@");
  });

  it("renders headerless mixed add/delete lines with honest counts", () => {
    const html = renderToStaticMarkup(
      <DiffView
        diff={"-stale\n+fresh\n kept"}
        label="config diff"
        sourcePath="a/config.toml"
      />,
    );

    expect(html).toContain("config.toml");
    expect(html).toContain("+1 / -1");
    expect(html).toContain("@@ -1,1 +1,1 @@");
  });

  it("keeps inert fallback for headerless content without a mutation path", () => {
    const html = renderToStaticMarkup(
      <DiffView diff={"+line"} label="Markdown diff" />,
    );

    expect(html).toContain('data-fallback="true"');
    expect(html).toContain("Unstructured diff output");
    expect(html).toContain("+line");
    expect(html).not.toContain("Untitled mutation");
    expect(html).not.toContain("Canonical mutation");
  });

  it("keeps structured headers from a full unified diff", () => {
    const html = renderToStaticMarkup(
      <DiffView
        diff={"--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n"}
        label="app diff"
      />,
    );

    expect(html).toContain("src/app.ts");
    expect(html).toContain("+1 / -1");
    expect(html).not.toContain("synthesized header");
  });

  it("falls back inertly when headerless content is not diff-shaped", () => {
    const html = renderToStaticMarkup(
      <DiffView diff={"prose output\nnot a diff"} label="odd diff" sourcePath="out.txt" />,
    );

    expect(html).toContain('data-fallback="true"');
    expect(html).toContain("prose output");
  });
});
