import { expect, test, type Page } from "@playwright/test";

import type { ProductMessage } from "../../product/product-api-types";
import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockProductApiOptions,
} from "./product-api-mock";

/**
 * F.5: recovering a successor the server abandoned.
 *
 * When a claimed successor cannot be started the API escalates it instead of
 * pretending: the control becomes `abandoned`, the session becomes
 * `needs_attention`, and the ledger row keeps the server's reason. The browser's
 * only way back is `POST .../controls/{control_id}/confirm`, which returns the
 * control to `pending` and moves the session off `needs_attention`. These cases
 * pin what the UI sends, what it says while the request is unsettled, and what
 * it says once the server has answered — including the refusals, which must not
 * be reported as a recovery.
 */

const NOW = "2026-09-26T00:00:00.000Z";
const SESSION_ID = "session-stranded";
const STRANDED_CONTENT = "the work that never started";
const ABANDONED_REASON =
  "successor start failed 3 times; last failure: runtime run start (provider unavailable)";

function abandonedSuccessor(
  overrides: Partial<ProductMessage> = {},
): ProductMessage {
  return {
    id: "message-stranded",
    product_session_id: SESSION_ID,
    content: STRANDED_CONTENT,
    requested_delivery: "successor",
    status: "needs_attention",
    seq: 1,
    created_at: NOW,
    reason: ABANDONED_REASON,
    ...overrides,
  };
}

async function openShell(
  page: Page,
  messageOverrides: Partial<ProductMessage> = {},
  options: Omit<MockProductApiOptions, "workspaces" | "sessions"> = {},
) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  // The session is in the state the escalation leaves it in: the reader's
  // decision is what it waits for.
  const session = {
    ...createMockSession(SESSION_ID, workspace.id, "Stranded session"),
    status: "needs_attention" as const,
  };
  const row = abandonedSuccessor(messageOverrides);
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    ...options,
  });
  api.messages[SESSION_ID] = [row];
  await page.goto(`/w/${workspace.id}/s/${SESSION_ID}`);
  await expect(page.getByText(row.content, { exact: true })).toBeVisible();
  return { api, workspace, session, row };
}

/** The one ledger row on screen, addressed by the text it carries. */
function strandedRow(page: Page, content = STRANDED_CONTENT) {
  return page.locator(".queued-message").filter({ hasText: content });
}

test("retrying a stranded successor sends the confirm and reports the server's answer", async ({
  page,
}) => {
  // The confirm is held open long enough to read the in-flight state, which is
  // the state that must not claim anything yet.
  const { api, workspace, row } = await openShell(page, {}, {
    followupConfirmDelayMs: 1_200,
  });

  const bubble = strandedRow(page);
  // The row renders the reason the API recorded, so the reader sees why the
  // successor is stranded before deciding anything.
  await expect(bubble.locator(".queued-message__reason")).toHaveText(ABANDONED_REASON);
  await expect(bubble).toHaveAttribute("data-status", "needs_attention");
  await expect(page.locator(".session-item__status").first()).toHaveAttribute(
    "data-status",
    "needs_attention",
  );

  await bubble.getByRole("button", { name: "重试这条消息" }).click();

  // In flight: the action is disabled and says so, and nothing is claimed before
  // the server has answered.
  await expect(bubble.getByRole("button", { name: "重试中…" })).toBeDisabled();
  await expect(bubble.locator(".queued-message__recovery")).toHaveCount(0);
  expect(api.followupConfirmRequests).toEqual([
    { sessionId: SESSION_ID, controlId: row.id },
  ]);

  // Delivered: the notice names the status the server committed, and it appears
  // only after the response.
  await expect(bubble.locator('.queued-message__recovery[data-tone="ok"]')).toHaveText(
    "重试已被接受——服务器已把这条消息重新排队。",
  );

  // The states then refresh from the API: the row is a queued successor again
  // (and the retry is gone, because there is nothing left to retry), the
  // server's abandoned reason leaves with it, and the session is no longer
  // waiting on the reader.
  await expect(bubble).toHaveAttribute("data-status", "queued");
  await expect(bubble.getByRole("button", { name: "重试这条消息" })).toHaveCount(0);
  await expect(bubble.locator(".queued-message__reason")).toHaveCount(0);
  await expect(page.locator(".session-item__status").first()).toHaveAttribute(
    "data-status",
    "idle",
  );
  await expect(
    page.locator(`.session-badge[data-status="needs_attention"]`),
  ).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`${workspace.id}/s/${SESSION_ID}$`, "u"));
});

test("a refused confirm reports the server's reason and leaves the row stranded", async ({
  page,
}) => {
  const refusal =
    "an abandoned follow-up cannot be confirmed while a turn is active";
  const { api, row } = await openShell(page, {}, {
    followupConfirmFailures: [
      { status: 409, code: "product_session_active", error: refusal },
    ],
  });

  const bubble = strandedRow(page);
  await bubble.getByRole("button", { name: "重试这条消息" }).click();

  // The refusal is the server's own reason, not a generic failure, and it is
  // reported as a failure.
  await expect(bubble.locator('.queued-message__recovery[data-tone="error"]')).toHaveText(
    `重试失败：${refusal}`,
  );
  expect(api.followupConfirmRequests).toEqual([
    { sessionId: SESSION_ID, controlId: row.id },
  ]);

  // Nothing moved: the row is still stranded, its reason is still the server's,
  // the action is offered again, and the session still waits on the reader.
  await expect(bubble).toHaveAttribute("data-status", "needs_attention");
  await expect(bubble.locator(".queued-message__reason")).toHaveText(ABANDONED_REASON);
  await expect(bubble.getByRole("button", { name: "重试这条消息" })).toBeEnabled();
  await expect(page.locator(".session-item__status").first()).toHaveAttribute(
    "data-status",
    "needs_attention",
  );
});

test("a dropped interjection is not offered a retry the server would refuse", async ({
  page,
}) => {
  // `current_run` rows reach `needs_attention` too, but their control kind is
  // `steer`: the confirm route refuses them, so the action is absent instead of
  // offered and then refused.
  const { api } = await openShell(page, {
    id: "message-interjection",
    content: "an interjection the turn dropped",
    requested_delivery: "current_run",
    reason: "the turn ended before this interjection was applied",
  });

  const bubble = strandedRow(page, "an interjection the turn dropped");
  await expect(bubble).toHaveAttribute("data-status", "needs_attention");
  await expect(bubble.getByRole("button", { name: "重试这条消息" })).toHaveCount(0);
  // Its existing affordance is untouched.
  await expect(bubble.getByRole("button", { name: "撤销该消息" })).toBeVisible();
  expect(api.followupConfirmRequests).toEqual([]);
});
