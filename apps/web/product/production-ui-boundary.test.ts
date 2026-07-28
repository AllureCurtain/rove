import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

import { describe, expect, it } from "vitest";

const WEB_ROOT = resolve(import.meta.dirname, "..");
const PRODUCTION_ROOTS = [
  "api",
  "chat",
  "inspector",
  "lib",
  "product",
  "product-v2",
  "settings",
  "shell",
  "sidebar",
  "state",
] as const;
const SOURCE_EXTENSION = /\.(?:ts|tsx|js|jsx)$/u;
const FORBIDDEN_IMPORTS = [
  "app/dev/product-ui-v2",
  "product-ui-v2-mock",
] as const;

describe("production Product UI boundary", () => {
  it("does not import the inert Product UI V2 preview or its Mock authority", () => {
    const violations: string[] = [];

    for (const root of PRODUCTION_ROOTS) {
      for (const file of sourceFiles(join(WEB_ROOT, root))) {
        if (file === import.meta.filename) {
          continue;
        }
        const source = readFileSync(file, "utf8");
        for (const forbidden of FORBIDDEN_IMPORTS) {
          if (source.includes(forbidden)) {
            violations.push(`${relative(WEB_ROOT, file)} -> ${forbidden}`);
          }
        }
      }
    }

    expect(violations).toEqual([]);
  });

  it("does not expose controls for contracts absent from the current product API", () => {
    const forbiddenAffordances = [
      ">Steer<",
      ">Follow-up<",
      ">Fork<",
      ">Reasoning<",
      ">Browse files<",
      ">Open artifact<",
      ">Download artifact<",
      ">Desktop<",
    ];
    const violations: string[] = [];

    for (const root of ["chat", "inspector", "product-v2", "shell", "sidebar"] as const) {
      for (const file of sourceFiles(join(WEB_ROOT, root))) {
        const source = readFileSync(file, "utf8").replace(/\s+/gu, "");
        for (const affordance of forbiddenAffordances) {
          if (source.includes(affordance)) {
            violations.push(`${relative(WEB_ROOT, file)} -> ${affordance}`);
          }
        }
      }
    }

    expect(violations).toEqual([]);
  });
});

function sourceFiles(root: string): string[] {
  return readdirSync(root).flatMap((entry) => {
    const path = join(root, entry);
    if (statSync(path).isDirectory()) {
      return sourceFiles(path);
    }
    return SOURCE_EXTENSION.test(entry) ? [path] : [];
  });
}
