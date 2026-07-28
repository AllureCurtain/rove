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
