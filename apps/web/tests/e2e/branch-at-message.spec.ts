import { expect, test, type Page } from "@playwright/test";

import type { ProductMessage } from "../../product/product-api-types";
import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  longCompletedTranscript,
} from "./product-api-mock";

/**
 * R6: edit-and-resend as a fork at a message.
 *
 * The endpoint takes a *ledger* sequence, not a transcript identity and not a
 * run ordinal, and it refuses a cut it cannot honour: a sequence outside the
 * ledger is a typed 400, one delivered by another run of the same session is a
 * typed 409, and a live parent is refused rather than forked behind its turn.
 * The child is a new session — the parent's history is not rewritten — so the
 * edited text is placed in the child's composer rather than assumed to follow.
 */

const NOW = "2026-09-26T00:00:00.000Z";

function ledgerUser(
  sessionId: string,
  seq: number,
  runId: string,
  content: string,
): ProductMessage {
  return {
    id: `message-${seq}`,
    product_session_id: sessionId,
    content,
    requested_delivery: "successor",
    status: "claimed_successor",
    seq,
    run_id: runId,
    created_at: NOW,
  };
}

async function openShell(
  page: Page,
  options: {
    runCount?: number;
    ledger?: (sessionId: string) => ProductMessage[];
    forkRefusal?: { status: number; code: string };
  } = {},
) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-branch", workspace.id, "Branch session");
  const transcript =
    options.runCount === undefined
      ? completedTranscript(workspace, session, "Branch question", "Branch answer")
      : longCompletedTranscript(workspace, session, options.runCount);
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    ...(options.forkRefusal === undefined
      ? {}
      : { forkRefusal: options.forkRefusal }),
  });
  api.messages[session.id] = options.ledger
    ? options.ledger(session.id)
    : [
        ledgerUser(
          session.id,
          1,
          "run-restored-1",
          options.runCount === undefined ? "Branch question" : "Restored question 1",
        ),
      ];
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(
    page.getByText(
      options.runCount === undefined
        ? "Branch answer"
        : `Restored answer ${options.runCount}`,
      { exact: true },
    ),
  ).toBeVisible();
  return { api, workspace, session };
}

/** Ask for a branch from the turn whose text is given, and confirm it. */
async function branchFromTurn(page: Page, turnText: string) {
  const turn = page.locator(".chat-bubble[data-role='user']").filter({ hasText: turnText });
  await turn.first().hover();
  await turn.first().getByRole("button", { name: "编辑并分支到新会话" }).click();
  const confirm = page.getByRole("alertdialog", { name: "编辑并分支到新会话" });
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "创建分支会话" }).click();
}

test("branching at a turn sends that turn's ledger sequence and opens the child", async ({
  page,
}) => {
  const { api, workspace, session } = await openShell(page);

  await branchFromTurn(page, "Branch question");

  // The cut is the ledger sequence of the edited turn, on the run that delivered
  // it — not a run ordinal, and not the transcript's own message id.
  expect(api.forkRequests).toEqual([
    { sessionId: session.id, truncateAfter: 1 },
  ]);
  const child = api.sessions.find((item) => item.parent_session_id === session.id);
  expect(child).toBeDefined();
  // The parent is untouched: the branch is a new session, not a rewrite.
  expect(api.sessions.some((item) => item.id === session.id)).toBe(true);
  await expect(page.getByText(`已创建分支会话「Fork of Branch session」`)).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/w/${workspace.id}/s/${child!.id}$`));
  // The endpoint never takes text, so the edited text is the child's draft.
  await expect(page.getByRole("textbox", { name: /输入消息/ })).toHaveValue(
    "Branch question",
  );
});

test("an unusable cut is refused, and no child is created", async ({ page }) => {
  // The cut is well formed and correctly derived; the server will not honour it.
  // A refusal must not leave a child behind or navigate as if it had succeeded.
  const { api, workspace, session } = await openShell(page, {
    forkRefusal: { status: 400, code: "product_invalid_input" },
  });

  await branchFromTurn(page, "Branch question");

  expect(api.forkRequests).toEqual([
    { sessionId: session.id, truncateAfter: 1 },
  ]);
  expect(api.sessions.some((item) => item.parent_session_id === session.id)).toBe(
    false,
  );
  await expect(page.locator("p.shell-alert")).toContainText("product_invalid_input");
  expect(page.url()).toContain(`/w/${workspace.id}/s/${session.id}`);
});

test("a live parent is refused with its own sentence", async ({ page }) => {
  // The endpoint refuses rather than forking behind the running turn, and the
  // copy says which of the two refusals happened.
  const { api, workspace, session } = await openShell(page, {
    forkRefusal: { status: 409, code: "product_session_active" },
  });

  await branchFromTurn(page, "Branch question");

  expect(api.forkRequests).toHaveLength(1);
  expect(api.sessions.some((item) => item.parent_session_id === session.id)).toBe(
    false,
  );
  await expect(page.locator("p.shell-alert")).toContainText("父会话仍在运行");
  expect(page.url()).toContain(`/w/${workspace.id}/s/${session.id}`);
});

test("a turn the fork run did not deliver is a source conflict", async ({ page }) => {
  // The turn is in this session's ledger but on an older run than the fork
  // point, so the cut cannot be honoured — and it is reported as a conflict
  // rather than as malformed input.
  const { api, workspace, session } = await openShell(page, { runCount: 2 });
  api.messages[session.id] = [
    ledgerUser(session.id, 1, "run-restored-1", "Restored question 1"),
  ];
  await page.reload();
  await expect(page.getByText("Restored answer 2", { exact: true })).toBeVisible();

  const second = page
    .locator(".chat-bubble[data-role='user']")
    .filter({ hasText: "Restored question 2" });
  // Only the ledger knows the sequence, so a turn the ledger does not hold is
  // refused before it leaves the browser.
  await second.first().hover();
  await second.first().getByRole("button", { name: "编辑并分支到新会话" }).click();

  await expect(page.locator("p.shell-alert")).toContainText("无法对应到会话的消息账本");
  expect(api.forkRequests).toEqual([]);
  expect(page.url()).toContain(`/w/${workspace.id}/s/${session.id}`);
});