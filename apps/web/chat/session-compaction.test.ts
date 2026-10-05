import { describe, expect, it } from "vitest";

import type { ProductSessionCompaction } from "../product/product-api-types";
import {
  compactionAvailability,
  compactionModeLabel,
  compactionOutcome,
} from "./session-compaction";

function compaction(
  overrides: Partial<ProductSessionCompaction> = {},
): ProductSessionCompaction {
  return {
    product_session_id: "s1",
    runtime_run_id: "run-1",
    triggered: true,
    mode: "model_generated",
    degraded: false,
    consecutive_failures: 0,
    circuit_open: false,
    source_message_count: 12,
    summary_truncated: false,
    token_estimate: 900,
    ...overrides,
  };
}

describe("compactionOutcome", () => {
  it("separates this call from the state it left behind", () => {
    const outcome = compactionOutcome(compaction());
    expect(outcome.status).toBe("compacted");
    expect(outcome.mode.copyKey).toBe("chat.compactModeModelGenerated");
    expect(outcome.tokenEstimate).toBe(900);
  });

  it("reports no run when the response carries no runtime run", () => {
    expect(
      compactionOutcome(
        compaction({ triggered: false, mode: "none", runtime_run_id: undefined }),
      ).status,
    ).toBe("no-run");
  });

  it("reports a no-op with the persisted breaker state", () => {
    const outcome = compactionOutcome(
      compaction({
        triggered: false,
        runtime_run_id: "run-1",
        consecutive_failures: 3,
        circuit_open: true,
      }),
    );
    expect(outcome.status).toBe("no-op");
    expect(outcome.consecutiveFailures).toBe(3);
    expect(outcome.circuitOpen).toBe(true);
  });

  it("keeps a degraded answer distinct and carries only the typed code", () => {
    const outcome = compactionOutcome(
      compaction({ mode: "degraded", degraded: true, failure_code: "rate_limited" }),
    );
    expect(outcome.degraded).toBe(true);
    expect(outcome.failureCode).toBe("rate_limited");
    // The provider's own message is not part of the contract, so nothing here
    // can render it.
    expect(Object.keys(outcome)).not.toContain("last_error");
  });

  it("keeps a truncated excerpt flagged", () => {
    const outcome = compactionOutcome(
      compaction({ summary: "excerpt", summary_truncated: true }),
    );
    expect(outcome.summary).toBe("excerpt");
    expect(outcome.summaryTruncated).toBe(true);
  });

  it("does not invent a summary the server did not send", () => {
    const outcome = compactionOutcome(compaction({ summary: undefined }));
    expect(outcome.summary).toBeNull();
  });
});

describe("compactionModeLabel", () => {
  it("translates the modes this build knows", () => {
    expect(compactionModeLabel("deterministic").copyKey).toBe(
      "chat.compactModeDeterministic",
    );
  });

  it("shows an unknown mode verbatim instead of guessing", () => {
    const label = compactionModeLabel("some_future_mode");
    expect(label.copyKey).toBeNull();
    expect(label.raw).toBe("some_future_mode");
  });
});

describe("compactionAvailability", () => {
  it("is available only at idle", () => {
    expect(
      compactionAvailability({ sessionId: "s1", sessionStatus: "idle", busy: false })
        .available,
    ).toBe(true);
    expect(
      compactionAvailability({ sessionId: "s1", sessionStatus: "running", busy: false })
        .reasonCopyKey,
    ).toBe("chat.compactIdleOnly");
    expect(
      compactionAvailability({ sessionId: "s1", sessionStatus: "idle", busy: true })
        .reasonCopyKey,
    ).toBe("chat.compactIdleOnly");
    expect(
      compactionAvailability({ sessionId: null, sessionStatus: null, busy: false })
        .available,
    ).toBe(false);
  });
});
