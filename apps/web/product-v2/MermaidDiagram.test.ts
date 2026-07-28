import { describe, expect, it } from "vitest";

import {
  hasUnsafeMermaidCssUrl,
  MERMAID_TEXT_LABEL_CONFIG,
} from "./MermaidDiagram";

describe("Mermaid render configuration", () => {
  it("renders labels as SVG text before strict sanitization", () => {
    expect(MERMAID_TEXT_LABEL_CONFIG).toMatchObject({
      htmlLabels: false,
      flowchart: { htmlLabels: false },
    });
  });
});

describe("Mermaid SVG CSS URL boundary", () => {
  it("allows local SVG fragment references", () => {
    expect(hasUnsafeMermaidCssUrl("marker-end: url(#arrowhead)")).toBe(false);
    expect(hasUnsafeMermaidCssUrl('fill: url("#gradient")')).toBe(false);
  });

  it("rejects every non-fragment CSS resource", () => {
    expect(hasUnsafeMermaidCssUrl("fill: url(//example.test/pixel)")).toBe(true);
    expect(hasUnsafeMermaidCssUrl("fill: url(https://example.test/pixel)")).toBe(true);
    expect(hasUnsafeMermaidCssUrl("fill: url(data:image/png;base64,AA==)")).toBe(true);
    expect(hasUnsafeMermaidCssUrl("fill: url(../pixel.png)")).toBe(true);
  });
});
