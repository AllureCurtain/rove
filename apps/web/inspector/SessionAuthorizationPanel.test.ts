import { describe, expect, it } from "vitest";

import type { ProductAuthorizationsResponse } from "../product/product-api-types";
import { collectSessionAuthorizationRecords } from "./SessionAuthorizationPanel";

function record(overrides: Partial<ProductAuthorizationsResponse["authorizations"][number]> = {}) {
  return {
    call_id: "call-1",
    job_id: "job-1",
    run_id: "run-a",
    tool: "write_file",
    args: { path: "notes.md" },
    reason: "destructive tool requires explicit approval",
    status: "pending",
    requested_at: "2026-09-17T00:00:00Z",
    updated_at: "2026-09-17T00:00:01Z",
    ...overrides,
  } as ProductAuthorizationsResponse["authorizations"][number];
}

function response(
  authorizations: ProductAuthorizationsResponse["authorizations"],
  truncated = false,
): ProductAuthorizationsResponse {
  return {
    session_id: "ps-1",
    authorizations,
    truncated,
  };
}

describe("collectSessionAuthorizationRecords", () => {
  it("projects the durable request and decision side of every authorization", () => {
    const records = collectSessionAuthorizationRecords(
      response([
        record({
          call_id: "call-1",
          status: "approved",
          decided_via: "job_api",
          outcome: { event: "tool_call_completed", seq: 4 },
        }),
      ]),
    );

    expect(records).toEqual([
      record({
        call_id: "call-1",
        status: "approved",
        decided_via: "job_api",
        outcome: { event: "tool_call_completed", seq: 4 },
      }),
    ]);
  });

  it("keeps unknown decision fields unknown instead of inventing an actor", () => {
    const records = collectSessionAuthorizationRecords(
      response([
        record({
          status: "approved",
          decided_via: null,
        }),
      ]),
    );

    expect(records[0]?.decided_via).toBeNull();
    expect(records[0]?.outcome).toBeUndefined();
  });

  it("preserves newest-first order from the durable endpoint", () => {
    const records = collectSessionAuthorizationRecords(
      response([
        record({ call_id: "call-2", run_id: "run-b", updated_at: "2026-09-17T00:02:00Z" }),
        record({ call_id: "call-1", run_id: "run-a", updated_at: "2026-09-17T00:01:00Z" }),
      ]),
    );

    expect(records.map((item) => item.call_id)).toEqual(["call-2", "call-1"]);
  });

  it("returns nothing for a session with no authorization requests", () => {
    expect(collectSessionAuthorizationRecords(response([]))).toEqual([]);
  });

  it("bounds the section so one session cannot flood the panel", () => {
    const authorizations = Array.from({ length: 80 }, (_unused, index) =>
      record({ call_id: `call-${index}`, updated_at: `2026-09-17T00:00:${String(index).padStart(2, "0")}Z` }),
    );
    const records = collectSessionAuthorizationRecords(response(authorizations));

    expect(records).toHaveLength(50);
    expect(records[0]?.call_id).toBe("call-0");
  });
});
