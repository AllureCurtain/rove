import { describe, expect, it } from "vitest";

import type { M1BrowserMigrationRunResult } from "../product/m1-browser-migration";
import {
  mappedLegacyProductHref,
  migrationAttentionContent,
  migrationIssueSummary,
} from "./M1MigrationGate";

const acknowledgement = {
  source_schema_version: 1,
  idempotency_key: "migration-1",
  receipt_id: "receipt-1",
  disposition: "applied" as const,
  workspace_mappings: [
    { source_id: "legacy-workspace", workspace_id: "server-workspace" },
  ],
  session_mappings: [
    { source_id: "legacy-session", product_session_id: "server-session" },
  ],
  provider_profile_mappings: [],
  issues: [],
  applied_at: "2026-07-27T00:00:00.000Z",
};

describe("M1MigrationGate recovery copy", () => {
  it("describes a pending request as exact replay without exposing an error", () => {
    const result: Extract<M1BrowserMigrationRunResult, { status: "pending" }> = {
      status: "pending",
      state: {
        status: "pending",
        source_schema_version: 1,
        idempotency_key: "migration-1",
        request: {
          source: "web_m1_local_storage",
          source_schema_version: 1,
          idempotency_key: "migration-1",
          workspaces: [],
          sessions: [],
          provider_profiles: [],
          safe_preferences: { theme: "dark" },
        },
        request_body: "{}",
        created_at: "2026-07-27T00:00:00.000Z",
      },
      failure: {
        code: "request_failed",
        message: "private path D:/secret/workspace and receipt migration-1",
      },
    };

    const content = migrationAttentionContent(result);

    expect(content.title).toBe("Import needs verification");
    expect(content.detail).toContain("exact saved request");
    expect(JSON.stringify(content)).not.toContain("D:/secret/workspace");
    expect(JSON.stringify(content)).not.toContain("migration-1");
  });

  it("keeps blocked storage and lock failures distinct", () => {
    const blocked = (code: "storage_write_failed" | "lock_unavailable") =>
      migrationAttentionContent({
        status: "blocked",
        failure: { code, message: "unsafe internal detail" },
      });

    expect(blocked("storage_write_failed").title).toBe(
      "Browser storage is unavailable",
    );
    expect(blocked("lock_unavailable").title).toBe(
      "Exclusive browser access is unavailable",
    );
  });

  it("summarizes issue codes without source identifiers", () => {
    expect(migrationIssueSummary(["preference_write_conflict"])).toBe(
      "1 item needs review: Newer server preferences were preserved.",
    );
    expect(
      migrationIssueSummary([
        "runtime_binding_not_found",
        "preference_write_conflict",
        "runtime_binding_not_found",
      ]),
    ).toBe(
      "3 items need review: Runtime history was not found; Newer server preferences were preserved.",
    );
  });

  it("maps legacy workspace and session routes from the acknowledgement", () => {
    expect(
      mappedLegacyProductHref(
        "/w/legacy-workspace/s/legacy-session",
        acknowledgement,
      ),
    ).toBe("/w/server-workspace/s/server-session");
    expect(
      mappedLegacyProductHref("/w/legacy-workspace", acknowledgement),
    ).toBe("/w/server-workspace");
  });

  it("leaves settings, new routes, and partial mappings alone", () => {
    expect(mappedLegacyProductHref("/settings/general", acknowledgement)).toBeNull();
    expect(mappedLegacyProductHref("/w/server-workspace", acknowledgement)).toBeNull();
    expect(
      mappedLegacyProductHref(
        "/w/legacy-workspace/s/unknown-session",
        acknowledgement,
      ),
    ).toBeNull();
  });
});
