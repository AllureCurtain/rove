import { expect, test } from "@playwright/test";
import type { FinalizationRecord, StreamEvent } from "../../lib/rove-types";
import { completedTranscript, createMockSession, createMockWorkspace, installMockProductApi } from "./product-api-mock";

const output = "Goal: inspect files\noutcome: success\nEvidence: tool_call:read-1\nWorkspace mutations: none";

test("runtime results stay readable after refresh and preserve the full copy", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const workspace = createMockWorkspace();
  const session = createMockSession("session-result", workspace.id, "Execution result");
  const transcript = completedTranscript(workspace, session, "Inspect files", "Model answer stays visible.");
  const segment = transcript.segments[0]!;
  const events = segment.events as Array<{ seq: number; event: StreamEvent }>;
  const record: FinalizationRecord = {
    finalization_id: "final-1", phase: "completed", mode: "deterministic",
    finish_reason: "completed", started_at: "2026-11-09T00:00:00Z", output, outcome: "success",
  };
  events.splice(-1, 1,
    { seq: 4, event: { type: "finalization_completed", record } },
    { seq: 5, event: { type: "run_completed", reason: "final", output } });
  segment.last_event_seq = segment.observed_through_seq = 5;
  await installMockProductApi(page, {
    workspaces: [workspace], sessions: [session], transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id, activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  for (let pass = 0; pass < 2; pass += 1) {
    if (pass) await page.reload();
    await expect(page.getByText("Model answer stays visible.", { exact: true })).toBeVisible();
    const result = page.getByRole("region", { name: "执行结果", exact: true });
    await expect(result).toContainText("本轮执行已完成。");
    await expect(result.locator("pre")).toHaveCount(0);
    const details = result.getByRole("button", { name: "执行详情", exact: true });
    await details.focus();
    await page.keyboard.press("Enter");
    await expect(details).toHaveAttribute("aria-expanded", "true");
    await expect(result.locator("pre")).toHaveText(output);
    await details.click();
    await expect(result.locator("pre")).toHaveCount(0);
  }
  await page.getByRole("button", { name: "复制", exact: true }).last().click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(output);
});
