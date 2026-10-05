import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * R9: the manual compaction entry point, and the answers it must not blur.
 *
 * The endpoint reuses the CLI's one manual path and takes a turn's exclusive
 * per-session claim, so it is offered at idle only: a refusal is never retried,
 * and the response's `triggered` (what this call did) stays distinct from `mode`
 * and `degraded` (what the session now holds).
 */

async function openShell(
  page: Page,
  compactionResult?: Record<string, unknown>,
) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-compact", workspace.id, "Compact session");
  // The restored run is what gives the session a binding: without one the API
  // answers that there is nothing to compact, which is its own state.
  const transcript = completedTranscript(
    workspace,
    session,
    "Compact question",
    "Compact answer",
  );
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    ...(compactionResult === undefined ? {} : { compactionResult }),
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Compact answer", { exact: true })).toBeVisible();
  const panel = page.locator(".compaction-panel");
  await expect(panel).toBeVisible();
  return { api, workspace, session, panel };
}

test("a manual compaction reports what the call did and what the session holds", async ({
  page,
}) => {
  const { api, session, panel } = await openShell(page, {
    triggered: true,
    mode: "degraded",
    degraded: true,
    failure_code: "summary_provider_unavailable",
    summary: "Earlier turns were summarised.",
    summary_truncated: true,
    token_estimate: 128,
    source_message_count: 4,
  });

  await panel.getByRole("button", { name: "手动压缩上下文" }).click();

  expect(api.compactionRequests).toEqual([session.id]);
  // `mode` is rendered through the locale's own label, and the degraded note
  // carries the typed code — never a provider message, which the response does
  // not contain at all.
  await expect(panel).toContainText("上下文已压缩。");
  await expect(panel).toContainText("降级摘要");
  await expect(panel).toContainText("summary_provider_unavailable");
  await expect(panel).toContainText("只显示了摘要的开头部分");
  await expect(panel).toContainText("压缩后上下文估计：128 tokens");
});

test("a call that compacted nothing says so instead of reporting success", async ({
  page,
}) => {
  const { panel } = await openShell(page, {
    triggered: false,
    mode: "none",
    degraded: false,
    circuit_open: true,
    consecutive_failures: 3,
    summary: undefined,
  });

  await panel.getByRole("button", { name: "手动压缩上下文" }).click();

  // Nothing was compacted, and the breaker's count is a fact about the session,
  // not about this call — so the two are said in one sentence rather than the
  // success line with a footnote.
  await expect(panel).toContainText("没有可压缩的内容；摘要模型已连续失败 3 次");
  await expect(panel).not.toContainText("上下文已压缩。");
});

test("a live session is refused with no automatic retry", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-busy", workspace.id, "Busy session");
  // `waiting_model` is the mode that makes "the turn is live" a fact rather than
  // a window: it has no completion event to replay, so the session holds its
  // claim for as long as this test needs. The default `completed` mode finishes
  // inside the same burst that starts the run, which is why this test used to
  // flake two ways (7 of 10 runs in the production profile): the 停止 button
  // either never became visible, or the panel re-enabled before the assertion.
  const api = await installMockProductApi(page, {
    mode: "waiting_model",
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  // A live run: the composer is disabled while the turn holds the claim, so the
  // press waits for it instead of landing in the restore window, where
  // `submitDraft` returns early and the keystroke is simply dropped.
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(composer).toBeEnabled();
  await composer.fill("first turn");
  await composer.press("Control+Enter");
  await expect(page.getByRole("button", { name: "停止", exact: true })).toBeVisible();

  const panel = page.locator(".compaction-panel");
  const button = panel.getByRole("button", { name: "手动压缩上下文" });
  // Offered but disabled while the turn holds the claim: the endpoint would only
  // refuse, so the panel does not spend the refusal.
  await expect(button).toBeDisabled();
  expect(api.compactionRequests).toEqual([]);
});