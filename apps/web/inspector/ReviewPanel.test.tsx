import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { ProductReview } from "../product/product-api-types";
import { ReviewPanel } from "./ReviewPanel";

const target = {
  schema_version: 1,
  spec: { kind: "uncommitted" as const },
  workspace_kind: "repo" as const,
  workspace_digest: "sha256:workspace",
  captured_at: "2026-08-17T00:00:00.000Z",
  entries: 1,
  entries_truncated: 0,
  digest: "sha256:target",
};

const finding = {
  finding_id: "rfd_1",
  severity: "high" as const,
  confidence: "high" as const,
  category: "correctness",
  path: "src/lib.rs",
  location: { start_line: 12, start_col: 1, end_line: 12, end_col: 8 },
  location_status: "validated" as const,
  title: "Incorrect branch",
  explanation: "The changed branch returns the wrong value.",
  evidence: [],
  rule: "correctness",
  suggestion: "Return the validated value.",
  status: "open",
};

function review(status: ProductReview["status"]): ProductReview {
  return {
    id: "01J00000000000000000000010",
    product_session_id: "01J00000000000000000000002",
    workspace_id: "01J00000000000000000000001",
    target,
    status,
    conclusion:
      status === "needs_attention"
        ? "stale"
        : status === "running" || status === "queued"
          ? undefined
          : status,
    findings_count: status === "findings" ? 1 : 0,
    unchecked_count: status === "partial" ? 1 : 0,
    warnings_count: status === "partial" ? 1 : 0,
    created_at: "2026-08-17T00:00:00.000Z",
    updated_at: "2026-08-17T00:00:01.000Z",
    captured_at: "2026-08-17T00:00:00.000Z",
  };
}

function renderReview(selected: ProductReview, findings = [] as Array<{ finding: typeof finding; sort_key: string }>) {
  return renderToStaticMarkup(
    <ReviewPanel
      reviews={[selected]}
      selectedReviewId={selected.id}
      selectedReview={selected}
      findings={findings}
      findingsCursor={null}
      findingsLoading={false}
      loading={false}
      error={null}
      onSelect={vi.fn()}
      onRefresh={vi.fn()}
      onCancel={vi.fn()}
      onLoadFindings={vi.fn()}
      onOpenFinding={vi.fn()}
    />,
  );
}

describe("ReviewPanel", () => {
  it("renders sanitized findings with path and line navigation", () => {
    const html = renderReview(review("findings"), [{ finding, sort_key: "001" }]);

    expect(html).toContain("Incorrect branch");
    expect(html).toContain("src/lib.rs:12");
    expect(html).toContain("Open in Files");
    expect(html).not.toContain("snapshot_bytes");
  });

  it.each([
    ["pass", "No actionable findings"],
    ["partial", "bounded or unchecked"],
    ["stale", "target changed"],
    ["needs_attention", "needs attention"],
    ["unavailable", "unavailable"],
    ["cancelled", "cancelled"],
    ["error", "runtime failed"],
  ] as const)("renders the %s terminal state", (status, text) => {
    expect(renderReview(review(status))).toContain(text);
  });

  it("keeps cancellation available for a running Review", () => {
    expect(renderReview(review("running"))).toContain('aria-label="Cancel Review"');
  });
});
