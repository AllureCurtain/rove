import { readFileSync } from "node:fs";

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ProductUiV2Preview } from "./ProductUiV2Preview";
import {
  PRODUCT_UI_V2_MOCK_SESSIONS,
  PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT,
  PRODUCT_UI_V2_PREVIEW_BOUNDARY,
} from "./product-ui-v2-mock";

const paletteSource = readFileSync(
  new URL("./product-ui-v2.module.css", import.meta.url),
  "utf8",
);
const previewSource = readFileSync(
  new URL("./ProductUiV2Preview.tsx", import.meta.url),
  "utf8",
);

describe("ProductUiV2Preview mock sessions", () => {
  it("defines stable, independent session evidence", () => {
    expect(PRODUCT_UI_V2_MOCK_SESSIONS).toHaveLength(3);
    expect(new Set(PRODUCT_UI_V2_MOCK_SESSIONS.map((session) => session.id)).size).toBe(3);

    for (const session of PRODUCT_UI_V2_MOCK_SESSIONS) {
      expect(session.id).toMatch(/^session-/);
      expect(session.updatedDateTime).toMatch(/^2026-07-27T/);
      expect(session.transcript.length).toBeGreaterThanOrEqual(5);
      expect(new Set(session.transcript.map((entry) => entry.id)).size).toBe(
        session.transcript.length,
      );
      expect(session.inspector.heading.length).toBeGreaterThan(0);
      expect(session.inspector.events.length).toBeGreaterThanOrEqual(4);
    }

    expect(new Set(PRODUCT_UI_V2_MOCK_SESSIONS.map((session) => session.status))).toEqual(
      new Set(["running", "complete", "attention"]),
    );
  });

  it("renders one current session and states the inert preview boundary", () => {
    const markup = renderToStaticMarkup(<ProductUiV2Preview />);

    expect(markup.match(/data-session-id="session-/g)).toHaveLength(4);
    expect(markup.match(/aria-current="page"/g)).toHaveLength(1);
    expect(markup).toContain("Inert UI mock");
    expect(markup).toContain(PRODUCT_UI_V2_PREVIEW_BOUNDARY);
    expect(markup).toContain(PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT);
    expect(
      previewSource.match(/\{PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT\}/g),
    ).toHaveLength(2);
    expect(PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT).toBe("mock-workspace/rove");
    expect(markup).not.toMatch(/[A-Za-z]:\\/);
    expect(previewSource).not.toMatch(/[A-Za-z]:\\/);
    expect(markup).toContain("C4 web control surface");
    expect(markup).toContain('dateTime="2026-07-27T09:25:00+08:00"');
  });

  it("locks the Ice Steel palette and excludes the retired lichen palette", () => {
    expect(paletteSource).toContain("--v2-canvas: #e7edf1;");
    expect(paletteSource).toContain("--v2-signal: #0d789f;");
    expect(paletteSource).toContain("--v2-rail-signal: #3fc5e8;");
    expect(paletteSource).toContain("--v2-canvas: #080d10;");
    expect(paletteSource).toContain("--v2-signal-strong: #69d8f2;");

    expect(paletteSource).not.toMatch(
      /#(?:e7e9e2|f1f2ec|697600|728000|c5d75a|c6d64a|1c221c|0f130f|151a15)/i,
    );
  });
});
