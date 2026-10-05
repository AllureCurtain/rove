import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * A stop re-reads the transcript to pick up the run's terminal tail. Two things
 * about that window are contracts of the shell rather than of the read:
 *
 * - it is one read. The shell must not re-attach to the run it has just watched
 *   end: attaching to a terminal job reports terminal immediately, which starts
 *   the reconciliation that restores again. With a snapshot that still called
 *   the run live the cycle had no exit — measured at 88-175 transcript reads in
 *   six seconds after one stop, against 3 when the snapshot was current.
 * - a send pressed inside it is deferred, not swallowed. The composer used to be
 *   disabled outright while a restore ran, so the keystroke never reached the
 *   textarea and the reader had to press again.
 */

const COMPOSER = /输入消息/u;
const STOP = { name: "停止", exact: true } as const;
const RESTORING = "正在完成会话恢复";
const QUEUED = "恢复完成后自动发送";

async function openSessionWithRun(page: import("@playwright/test").Page) {
  const workspace = createMockWorkspace();
  const session = createMockSession("session-1", workspace.id, "Stopped run");
  const api = await installMockProductApi(page, {
    mode: "waiting_model",
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    // The re-read a stop triggers is what the second case presses into.
    transcriptDelayMs: { [session.id]: 1_500 },
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const composer = page.getByRole("textbox", { name: COMPOSER });
  await expect(composer).toBeEnabled();
  return { api, composer };
}

test("a stop re-reads the transcript once, not in a loop", async ({ page }) => {
  const { api, composer } = await openSessionWithRun(page);
  await composer.fill("stop me");
  await composer.press("Control+Enter");
  const stop = page.getByRole("button", STOP);
  await expect(stop).toBeVisible();
  await expect(page.locator(".chat-composer__meta")).not.toContainText(RESTORING);

  const before = api.transcriptRequests.length;
  await stop.click();
  await expect(stop).toHaveCount(0);
  // The window the loop lived in: it produced roughly thirty reads a second, so
  // a loop cannot pass a bound this small.
  await page.waitForTimeout(2_000);
  expect(api.transcriptRequests.length - before).toBeLessThanOrEqual(2);
});

test("a send pressed while the stop re-reads the transcript goes out when it lands", async ({
  page,
}) => {
  const { api, composer } = await openSessionWithRun(page);
  await composer.fill("stop me");
  await composer.press("Control+Enter");
  const stop = page.getByRole("button", STOP);
  await expect(stop).toBeVisible();
  // The initial restore owns the first pause; wait it out so the press below
  // lands in the one the stop starts.
  await expect(page.locator(".chat-composer__meta")).not.toContainText(RESTORING);

  const turnsBefore = api.jobStarts.length;
  await stop.click();
  const meta = page.locator(".chat-composer__meta");
  await expect(meta).toContainText(RESTORING);
  // F6 puts the stopped send back; that restore is independent of the re-read.
  await expect(composer).toHaveValue("stop me");
  const readsBefore = api.transcriptRequests.length;

  await composer.press("Control+Enter");
  await expect(meta).toContainText(QUEUED);
  expect(api.jobStarts.length - turnsBefore).toBe(0);

  // The press was not lost: the send goes out once the read lands, exactly once.
  await expect(composer).toHaveValue("", { timeout: 15_000 });
  await expect(page.getByRole("button", STOP)).toBeVisible();
  expect(api.jobStarts.length - turnsBefore).toBe(1);
  expect(api.transcriptRequests.length - readsBefore).toBeLessThanOrEqual(1);
});
