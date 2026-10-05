import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { RichText, safeRichTextUrl } from "./RichText";

describe("RichText", () => {
  it("renders Markdown without loading image URLs or raw HTML", () => {
    const html = renderToStaticMarkup(
      <RichText content={'# Result\n\n![workspace plot](https://example.test/secret.png)\n\n<script>alert(1)</script>'} />,
    );

    expect(html).toContain("Result");
    expect(html).toContain("Image unavailable");
    expect(html).not.toContain("secret.png");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
  });

  it("blocks executable and data URLs", () => {
    expect(safeRichTextUrl("javascript:alert(1)")).toBe("");
    expect(safeRichTextUrl("data:text/html,hello")).toBe("");
    expect(safeRichTextUrl("https://example.test/docs")).toBe(
      "https://example.test/docs",
    );
    expect(safeRichTextUrl("/settings/providers")).toBe("/settings/providers");
    expect(safeRichTextUrl("//example.test/escape")).toBe("");
    expect(safeRichTextUrl("/\\example.test/escape")).toBe("");
    expect(safeRichTextUrl("#details")).toBe("#details");
  });
});

  it("keeps the browser fallback attributes on external links", () => {
    // P5a added a Desktop-host click path. In a plain browser the anchor must
    // stay a normal, isolated new-tab link, so assert the rendered markup
    // still carries href, target and rel.
    const html = renderToStaticMarkup(
      <RichText content={"See [the docs](https://example.test/docs) for details."} />,
    );

    expect(html).toContain('href="https://example.test/docs"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noreferrer noopener"');
  });
  it("still refuses to render an executable or data scheme", () => {
    // Relative paths are intentionally allowed by safeRichTextUrl, so the
    // guard is asserted on the schemes that actually execute.
    const html = renderToStaticMarkup(
      <RichText content={"[run](javascript:alert(1)) and [open](data:text/html,x)"} />,
    );

    expect(html).not.toContain("javascript:");
    expect(html).not.toContain("data:text/html");
    expect(html).toContain("rich-text__blocked-link");
  });
