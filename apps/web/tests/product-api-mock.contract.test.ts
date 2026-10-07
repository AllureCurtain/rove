/**
 * Contract between `tests/e2e/product-api-mock.ts` and the checked-in OpenAPI
 * snapshot (`../api/openapi.json`, kept in sync with the runtime spec by the
 * API test suite).
 *
 * Two directions, both path-level (the mock dispatches on method inside each
 * matched branch, so only the path is statically visible):
 *
 * - Coverage: every `/product/*` path the OpenAPI document declares must be
 *   answered by a mock matcher, or named in `UNMOCKED_PRODUCT_PATHS` below.
 *   Adding a product route without touching either fails this test.
 * - Staleness: every `path === "..."` literal and every `path.match(...)`
 *   pattern in the mock must correspond to a declared product route, so a
 *   deleted or renamed endpoint cannot keep a phantom handler alive.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const spec = JSON.parse(
  readFileSync(new URL("../../api/openapi.json", import.meta.url), "utf8"),
) as { paths: Record<string, unknown> };

const mockSource = readFileSync(
  new URL("./e2e/product-api-mock.ts", import.meta.url),
  "utf8",
);

/**
 * Product routes the mock deliberately does not serve. Each entry is a route
 * no e2e spec exercises today; the mock's fallthrough already answers these
 * with a typed `product_not_found` 404. Shrink this list whenever a spec
 * starts driving one of the routes.
 */
const UNMOCKED_PRODUCT_PATHS = [
  // SSE product event stream — e2e drives state through the /jobs mock instead.
  "/product/events",
  // Loopback-only credential onboarding; e2e seeds provider state directly.
  "/product/provider-onboarding",
  // Review lifecycle: not exercised by any spec yet.
  "/product/reviews/{review_id}",
  "/product/reviews/{review_id}/cancel",
  "/product/reviews/{review_id}/findings",
  "/product/sessions/{session_id}/reviews",
  // Artifact, authorization, diff, and workspace file/preview surfaces.
  "/product/sessions/{session_id}/artifacts",
  "/product/sessions/{session_id}/artifacts/{artifact_id}/content",
  "/product/sessions/{session_id}/artifacts/{artifact_id}/download",
  "/product/sessions/{session_id}/artifacts/{artifact_id}/preview",
  "/product/sessions/{session_id}/authorizations",
  "/product/sessions/{session_id}/diff",
  "/product/workspaces/{workspace_id}/files",
  "/product/workspaces/{workspace_id}/files/content",
  "/product/workspaces/{workspace_id}/files/download",
  "/product/workspaces/{workspace_id}/files/preview",
  "/product/workspaces/{workspace_id}/previews",
  "/product/workspaces/{workspace_id}/previews/{preview_id}",
  // Control-plane routes driven only through the generic followups path today.
  "/product/sessions/{session_id}/controls",
  "/product/sessions/{session_id}/controls/{control_id}/revoke",
  "/product/sessions/{session_id}/followups",
  "/product/sessions/{session_id}/steers",
  // Interactive workspace picker dialog has no e2e surface.
  "/product/workspace-picker",
] as const;

/** Render an OpenAPI path template into one concrete path for matching. */
function renderPath(template: string): string {
  return template.replace(/\{[^}]+\}/gu, "id-1");
}

/** `path === "..."` literals declared in the mock. */
function literalMatchers(source: string): string[] {
  const found: string[] = [];
  for (const match of source.matchAll(/path === "([^"]+)"/gu)) {
    found.push(match[1] ?? "");
  }
  return found;
}

/** Regex literals passed to `path.match(...)`, including multi-line ones. */
function regexMatchers(source: string): RegExp[] {
  const found: RegExp[] = [];
  const needle = /path\.match\(/gu;
  let cursor: RegExpExecArray | null;
  while ((cursor = needle.exec(source)) !== null) {
    let i = cursor.index + cursor[0].length;
    while (/\s/u.test(source.charAt(i))) {
      i += 1;
    }
    if (source.charAt(i) !== "/") {
      continue;
    }
    let j = i + 1;
    let inClass = false;
    while (j < source.length) {
      const char = source.charAt(j);
      if (char === "\\") {
        j += 2;
        continue;
      }
      if (char === "[") {
        inClass = true;
      } else if (char === "]") {
        inClass = false;
      } else if (char === "/" && !inClass) {
        break;
      }
      j += 1;
    }
    found.push(new RegExp(source.slice(i + 1, j), "u"));
  }
  return found;
}

const productPaths = Object.keys(spec.paths).filter((path) =>
  path.startsWith("/product"),
);
const literals = literalMatchers(mockSource);
const regexes = regexMatchers(mockSource);
const unmocked = new Set<string>(UNMOCKED_PRODUCT_PATHS);

describe("product API mock contract", () => {
  it("answers or explicitly lists every declared product route", () => {
    const gaps = productPaths.filter((template) => {
      if (unmocked.has(template)) {
        return false;
      }
      const rendered = renderPath(template);
      return (
        !literals.includes(rendered) &&
        !regexes.some((pattern) => pattern.test(rendered))
      );
    });
    expect(gaps).toEqual([]);
  });

  it("keeps the unmocked list honest — listed routes stay unmocked", () => {
    const stale = UNMOCKED_PRODUCT_PATHS.filter((template) => {
      const rendered = renderPath(template);
      return (
        literals.includes(rendered) ||
        regexes.some((pattern) => pattern.test(rendered))
      );
    });
    expect(stale).toEqual([]);
  });

  it("keeps the unmocked list limited to routes the spec still declares", () => {
    const phantom = UNMOCKED_PRODUCT_PATHS.filter(
      (template) => !productPaths.includes(template),
    );
    expect(phantom).toEqual([]);
  });

  it("has no mock literal for a route the spec does not declare", () => {
    const phantom = literals.filter((literal) => {
      if (!literal.startsWith("/product")) {
        return false;
      }
      return !productPaths.includes(literal);
    });
    expect(phantom).toEqual([]);
  });

  it("has no mock pattern that answers only undeclared routes", () => {
    const rendered = productPaths.map(renderPath);
    const orphan = regexes.filter(
      (pattern) =>
        pattern.source.startsWith("^\\/product") &&
        !rendered.some((path) => pattern.test(path)) &&
        !literals.some((literal) => pattern.test(literal)),
    );
    expect(orphan).toEqual([]);
  });
});
