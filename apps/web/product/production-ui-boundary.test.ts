import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

import { describe, expect, it } from "vitest";

const WEB_ROOT = resolve(import.meta.dirname, "..");
const SOURCE_EXTENSION = /\.(?:ts|tsx|js|jsx)$/u;

describe("production Product UI boundary", () => {
  it("uses the server-backed G1 controls without preview-only or client-run ownership", () => {
    const forbiddenAffordances = [
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

    const composer = readFileSync(join(WEB_ROOT, "chat", "Composer.tsx"), "utf8");
    const continuity = readFileSync(
      join(WEB_ROOT, "state", "use-session-continuity.ts"),
      "utf8",
    );
    const transcript = readFileSync(join(WEB_ROOT, "chat", "Transcript.tsx"), "utf8");
    expect(composer).toContain('t("chat.send")');
    expect(composer).toContain("CopyProvider");
    expect(transcript).not.toMatch(/eventSeq|product_session|prompt hash|cache key/i);
    expect(continuity).toContain("productClient.sendMessage");
    expect(continuity).toContain("productClient.promoteMessage");
    expect(continuity).toContain("productClient.revokeMessage");

    const shell = readFileSync(join(WEB_ROOT, "shell", "ProductApp.tsx"), "utf8");
    const tree = readFileSync(join(WEB_ROOT, "sidebar", "WorkspaceTree.tsx"), "utf8");
    expect(shell).toContain("server.forkSession(activeSession.id)");
    expect(tree).toContain("forkPointRunId");
    expect(tree).toContain('t("workspace.sessionsAndBranches")');
    expect(transcript).toContain('t("workspace.forked")');
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
