import { expect, test, type Page } from "@playwright/test";

import type { ProductMessage } from "../../product/product-api-types";
import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * R4: the successor queue moves in one atomic reorder.
 *
 * `POST .../messages/reorder` requires `ordered_ids` to name *exactly* the
 * session's pending successor queue, so the interesting cases are the refusals:
 * a stale list (one of the two was promoted or revoked in the meantime) is a
 * typed conflict, malformed input is a typed 400, and neither may be reported as
 * a queue that moved.
 */

const NOW = "2026-09-26T00:00:00.000Z";

function queued(
  sessionId: string,
  id: string,
  seq: number,
  content: string,
  queueOrder?: number,
): ProductMessage {
  return {
    id,
    product_session_id: sessionId,
    content,
    requested_delivery: "successor",
    status: "queued",
    seq,
    created_at: NOW,
    ...(queueOrder === undefined ? {} : { queue_order: queueOrder }),
  };
}

async function openShell(
  page: Page,
  options: { ledger?: (sessionId: string) => ProductMessage[] } = {},
) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-queue", workspace.id, "Queue session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  const seeded = options.ledger
    ? options.ledger(session.id)
    : [
        queued(session.id, "message-a", 1, "first queued"),
        queued(session.id, "message-b", 2, "second queued"),
      ];
  api.messages[session.id] = seeded;
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  // The ledger is served by the mock and rendered beside the canonical history,
  // so a missing row would make the whole spec vacuous.
  for (const message of seeded) {
    await expect(page.getByText(message.content, { exact: true })).toBeVisible();
  }
  return { api, workspace, session };
}

/** The two queued bubbles in the order the transcript renders them. */
async function queuedOrder(page: Page) {
  return page.locator(".queued-message").allInnerTexts();
}

test("moving a queued successor sends one exact reorder", async ({ page }) => {
  const { api, session } = await openShell(page);

  const first = page.locator(".queued-message").filter({ hasText: "first queued" });
  await first.getByRole("button", { name: "下移一位" }).click();

  // One call, naming the complete queue in the wanted order — not a pair of
  // swaps, and not a partial list.
  expect(api.reorderRequests).toEqual([
    { sessionId: session.id, orderedIds: ["message-b", "message-a"] },
  ]);
  await expect
    .poll(async () => (await queuedOrder(page))[0])
    .toContain("second queued");
});

test("a stale reorder list is refused instead of merged", async ({ page }) => {
  const { api, session } = await openShell(page);
  // The queue changed under the client: one of the two was revoked elsewhere.
  api.messages[session.id] = api.messages[session.id]!.map((message) =>
    message.id === "message-b" ? { ...message, status: "revoked" } : message,
  );

  const first = page.locator(".queued-message").filter({ hasText: "first queued" });
  await first.getByRole("button", { name: "下移一位" }).click();

  expect(api.reorderRequests).toHaveLength(1);
  expect(api.reorderRequests[0]!.sessionId).toBe(session.id);
  // The list carried a message the server no longer has pending, so the move was
  // not applied, and the refusal says the queue changed rather than pretending
  // the order it assumed is the order the server holds.
  await expect(page.locator("p.shell-alert")).toContainText("队列已变化，未移动");
  await expect
    .poll(async () => (await queuedOrder(page))[0])
    .toContain("first queued");
});

test("the last queued successor has no direction to move in", async ({ page }) => {
  const { api } = await openShell(page, {
    ledger: (sessionId) => [queued(sessionId, "message-only", 1, "only queued")],
  });

  await expect(page.getByText("only queued", { exact: true })).toBeVisible();
  const only = page.locator(".queued-message").filter({ hasText: "only queued" });
  // One pending message has no adjacent swap in either direction, so the arrows
  // are absent rather than disabled-and-refused.
  await expect(only.getByRole("button", { name: "上移一位" })).toHaveCount(0);
  await expect(only.getByRole("button", { name: "下移一位" })).toHaveCount(0);
  expect(api.reorderRequests).toEqual([]);
});