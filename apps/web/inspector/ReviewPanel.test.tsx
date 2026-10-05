import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { ProductReview } from "../product/product-api-types";
import { ReviewPanel } from "./ReviewPanel";
const copyState = vi.hoisted(() => ({ locale: "zh-CN" as "zh-CN" | "en-US" }));
vi.mock("../copy/CopyProvider", async () => {
  const { createTranslator } = await import("../copy");
  return { useCopy: () => ({ t: createTranslator(copyState.locale) }) };
});
import type { Locale } from "../copy";

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

function renderReview(selected: ProductReview, findings = [] as Array<{ finding: typeof finding; sort_key: string }>, locale: Locale = "zh-CN", error: string | null = null) {
  copyState.locale = locale;
  return renderToStaticMarkup(
    <ReviewPanel
      reviews={[selected]}
      selectedReviewId={selected.id}
      selectedReview={selected}
      findings={findings}
      findingsCursor={null}
      findingsLoading={false}
      loading={false}
      error={error}
      onSelect={vi.fn()}
      onRefresh={vi.fn()}
      onCancel={vi.fn()}
      onLoadFindings={vi.fn()}
      onOpenFinding={vi.fn()}
    />,
  );
}

describe("ReviewPanel", () => {
  it("localizes chrome without translating findings or exposing diagnostics", () => {
    const html = renderReview(review("findings"), [{ finding, sort_key: "001" }], "en-US", "secret-token raw payload");
    expect(html).toContain("Read-only Review");
    expect(html).toContain("Open in Files");
    expect(html).toContain(finding.title);
    expect(html).toContain(finding.explanation);
    expect(html).toContain("Refresh and retry");
    expect(html).not.toContain("secret-token");
    expect(html).not.toContain("raw payload");
    expect(html).not.toContain("Apply fix");
  });
  it("renders sanitized findings with path and line navigation", () => {
    const html = renderReview(review("findings"), [{ finding, sort_key: "001" }]);

    expect(html).toContain("Incorrect branch");
    expect(html).toContain("src/lib.rs:12");
    expect(html).toContain("在文件中打开");
    expect(html).not.toContain("snapshot_bytes");
  });

  it.each([
    ["pass", "未报告需要处理的问题"],
    ["partial", "部分内容未完成检查"],
    ["stale", "目标已更改"],
    ["needs_attention", "需要关注"],
    ["unavailable", "审查目标或服务不可用"],
    ["cancelled", "审查已取消"],
    ["error", "审查失败"],
  ] as const)("renders the %s terminal state", (status, text) => {
    expect(renderReview(review(status))).toContain(text);
  });

  it("keeps cancellation available for a running Review", () => {
    expect(renderReview(review("running"))).toContain('aria-label="取消审查"');
  });
});
