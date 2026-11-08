import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { FinalizationRecord } from "../lib/rove-types";
import { RuntimeResult } from "./RuntimeResult";

const record: FinalizationRecord = {
  finalization_id: "final-1", phase: "completed", mode: "deterministic",
  finish_reason: "completed", started_at: "2026-11-09T00:00:00Z", outcome: "success",
};
const content = "Goal: inspect\noutcome: success\n<script>alert(1)</script>";

describe("RuntimeResult", () => {
  it("starts a recorded success collapsed without exposing diagnostic prose", () => {
    const html = renderToStaticMarkup(<RuntimeResult record={record} content={content} />);
    expect(html).toContain("本轮执行已完成。");
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain("Goal:");
  });

  it.each(["partial", "blocked", "rejected", "cancelled", "interrupted", "exhausted", "indeterminate", "failed", null] as const)(
    "opens details for %s without reporting success", (outcome) => {
      const html = renderToStaticMarkup(<RuntimeResult record={{ ...record, outcome }} content={content} />);
      expect(html).toContain('aria-expanded="true"');
      expect(html).toContain("Goal: inspect");
      expect(html).not.toContain("本轮执行已完成。");
      expect(html).toContain("&lt;script&gt;");
      expect(html).not.toContain("<script>");
    },
  );

  it("keeps contradictory records visible and explains fallback", () => {
    const html = renderToStaticMarkup(<RuntimeResult record={{
      ...record, mode: "deterministic_fallback", incomplete_step_ids: ["step-2"],
    }} content={content} />);
    expect(html).toContain("执行结果需要核对。");
    expect(html).toContain("模型未能生成总结");
    expect(html).toContain("记录中仍有 1 个步骤未完成。");
    expect(html).toContain('aria-expanded="true"');
  });
});
