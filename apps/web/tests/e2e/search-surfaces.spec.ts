import { expect, test, type Page } from "@playwright/test";

import type { ProductMessage, ProductSearchHit } from "../../product/product-api-types";
import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  longCompletedTranscript,
} from "./product-api-mock";

/**
 * R7: the two search surfaces, and the answers they must not flatten.
 *
 * The endpoint rules are the reason these cases exist: a page that stopped on its
 * own budget is not the end of the history, a refused term and a cleaned trace are
 * not "no results", and a cursor only ever travels with the term and scope that
 * issued it. The mock refuses and pages exactly as the routes do.
 */

const NOW = "2026-09-26T00:00:00.000Z";

function messageHit(
  sessionId: string,
  seq: number,
  snippet: string,
): ProductSearchHit {
  return { session_id: sessionId, source: "message", seq, snippet, created_at: NOW };
}

function traceHit(
  sessionId: string,
  seq: number,
  snippet: string,
  runOrdinal: number,
  createdAt?: string,
): ProductSearchHit {
  return {
    session_id: sessionId,
    source: "trace",
    seq,
    run_id: "run-restored-1",
    run_ordinal: runOrdinal,
    snippet,
    ...(createdAt === undefined ? {} : { created_at: createdAt }),
  };
}

/** A ledger entry for the run the restored transcript projects. */
function ledgerMessage(sessionId: string, runId = "run-restored-1"): ProductMessage {
  return {
    id: "message-1",
    product_session_id: sessionId,
    content: "Palette question",
    requested_delivery: "successor",
    status: "claimed_successor",
    seq: 1,
    run_id: runId,
    created_at: NOW,
  };
}

async function openShell(
  page: Page,
  options: {
    searchCorpus?: Record<string, ProductSearchHit[]>;
    searchExpiredCursors?: string[];
    searchExpiredScopes?: string[];
    sessionMessageSearchFailures?: number;
    traceSearchPageSize?: number;
    /** Restore this many contiguous runs instead of the single-run fixture. */
    runCount?: number;
  } = {},
) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-search", workspace.id, "Search session");
  const other = createMockSession("session-other", workspace.id, "Release checklist");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session, other],
    transcripts: {
      [session.id]:
        options.runCount === undefined
          ? completedTranscript(
              workspace,
              session,
              "Palette question",
              "Palette answer",
            )
          : longCompletedTranscript(workspace, session, options.runCount),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    ...options,
  });
  // The ledger row that names the restored turn. It is what makes a hit's
  // `message_seq` resolvable at all: the search response never carries a
  // transcript identity.
  api.messages[session.id] = [ledgerMessage(session.id)];
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(
    page.getByText(
      options.runCount === undefined
        ? "Palette answer"
        : `Restored answer ${options.runCount}`,
      { exact: true },
    ),
  ).toBeVisible();
  return { api, workspace, session, other };
}

/** Open one of the two search surfaces through the command palette. */
async function openSearchSurface(page: Page, action: string) {
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "命令面板" });
  await expect(palette).toBeVisible();
  await palette.getByRole("combobox").fill(action);
  const option = palette.getByRole("option").filter({ hasText: action });
  await expect(option.first()).toBeVisible();
  await option.first().click();
}

test("session search renders the server's bounded windows and locates a hit", async ({
  page,
}) => {
  const { api, session } = await openShell(page, {
    searchCorpus: {
      "session:session-search": [
        messageHit("session-search", 1, "…[REDACTED:secret_pattern] appears here…"),
      ],
    },
  });

  await openSearchSurface(page, "搜索本会话消息");
  const panel = page.getByRole("dialog", { name: "搜索本会话消息" });
  await panel.locator("#session-message-search-term").fill("needle");
  await panel.locator("#session-message-search-term").press("Enter");

  // The first page of a term carries no cursor; the page size is the client's.
  expect(api.sessionMessageSearchRequests).toEqual([
    { sessionId: session.id, q: "needle", cursor: null, limit: "32" },
  ]);
  // The window is the server's, already redacted: it is shown exactly as sent.
  await expect(panel).toContainText("…[REDACTED:secret_pattern] appears here…");
  await expect(panel).toContainText("已到最后一页。");

  // The reveal resolves the ledger sequence to the identity the restored turn
  // renders under, and says so only because that identity is on the DOM.
  await expect(
    page.locator('[data-message-id="message:run-restored-1:user"]'),
  ).toHaveCount(1);
  await panel.getByRole("button", { name: "定位到该消息" }).click();
  await expect(panel).toContainText("已定位到该消息。");
});

test("a hit in an unmounted older run is mounted before it is revealed", async ({
  page,
}) => {
  await openShell(page, {
    runCount: 20,
    searchCorpus: {
      "session:session-search": [messageHit("session-search", 1, "…older…")],
    },
  });

  // The mount window is tail-anchored, so the first run is loaded but not on the
  // DOM: a reveal there has to grow the window first.
  const target = page.locator('[data-message-id="message:run-restored-1:user"]');
  await expect(target).toHaveCount(0);

  await openSearchSurface(page, "搜索本会话消息");
  const panel = page.getByRole("dialog", { name: "搜索本会话消息" });
  await panel.locator("#session-message-search-term").fill("needle");
  await panel.locator("#session-message-search-term").press("Enter");
  await panel.getByRole("button", { name: "定位到该消息" }).click();

  await expect(panel).toContainText("已定位到该消息。");
  await expect(target).toHaveCount(1);
});

test("a failed session search is not presented as no results", async ({ page }) => {
  const { session } = await openShell(page, {
    searchCorpus: { "session:session-search": [messageHit("session-search", 1, "hit")] },
    sessionMessageSearchFailures: 1,
  });

  await openSearchSurface(page, "搜索本会话消息");
  const panel = page.getByRole("dialog", { name: "搜索本会话消息" });
  await panel.locator("#session-message-search-term").fill("needle");
  await panel.locator("#session-message-search-term").press("Enter");

  await expect(panel).toContainText("本会话消息搜索失败");
  await expect(panel).not.toContainText("本会话没有匹配的消息。");
});

test("only a successful empty page reads as no matching message", async ({ page }) => {
  // No corpus for this scope: the route answers 200 with an empty page, which is
  // the one answer that may be rendered as "nothing matched".
  await openShell(page);

  await openSearchSurface(page, "搜索本会话消息");
  const panel = page.getByRole("dialog", { name: "搜索本会话消息" });
  await panel.locator("#session-message-search-term").fill("needle");
  await panel.locator("#session-message-search-term").press("Enter");

  await expect(panel).toContainText("本会话没有匹配的消息。");
});

test("the workspace scope refuses a term shorter than the trigram floor", async ({
  page,
}) => {
  const { api } = await openShell(page);

  await openSearchSurface(page, "跨会话与事件记录搜索");
  const view = page.getByRole("dialog", { name: "跨语料搜索" });
  await view.locator("#product-search-scope").selectOption("workspace");
  await view.locator("#product-search-term").fill("ab");
  await view.locator("#product-search-term").press("Enter");

  // Refused before dispatch: the route has no path to a sub-trigram term in this
  // scope, and an empty page would misreport that as "nothing matched".
  await expect(view).toContainText("工作区范围需要至少 3 个字符");
  await expect(view).not.toContainText("没有匹配结果。");
  expect(api.productSearchRequests).toEqual([]);
});

test("a bounded trace page keeps its cursor, and an expired one restarts", async ({
  page,
}) => {
  const traceScope = "trace:session-search";
  const expiredCursor = `mock-cursor:${encodeURIComponent("needle")}:${encodeURIComponent(traceScope)}:3`;
  const { api } = await openShell(page, {
    searchCorpus: {
      [traceScope]: [
        traceHit("session-search", 11, '{"type":"run_started"}', 1, NOW),
        traceHit("session-search", 12, '{"type":"llm_chunk"}', 1, NOW),
        traceHit("session-search", 13, '{"type":"llm_message"}', 1),
        traceHit("session-search", 14, '{"type":"run_completed"}', 1),
        traceHit("session-search", 15, '{"type":"note"}', 1),
      ],
    },
    // The request's byte budget is shared across the runs it reads, so the page
    // can come back short and still have more to read.
    traceSearchPageSize: 3,
    searchExpiredCursors: [expiredCursor],
  });

  await openSearchSurface(page, "跨会话与事件记录搜索");
  const view = page.getByRole("dialog", { name: "跨语料搜索" });
  await view.locator("#product-search-scope").selectOption("trace");
  await view.locator("#product-search-term").fill("needle");
  await view.locator("#product-search-term").press("Enter");

  await expect(view).toContainText("本次只读完了一部分事件记录");
  expect(api.productSearchRequests[0]).toEqual({
    q: "needle",
    scope: traceScope,
    cursor: null,
    limit: "32",
  });
  // A legacy trace line has no timestamp; the view reports that instead of
  // inventing one.
  await expect(view).toContainText("无时间戳");

  await view.getByRole("button", { name: "继续加载" }).click();

  // The cursor's run is gone: the cursor is dropped, the search restarts from the
  // first page once, and the restart stays visible.
  await expect(view).toContainText("已丢弃游标从第一页重新开始");
  expect(api.productSearchRequests).toHaveLength(3);
  expect(api.productSearchRequests[1]).toEqual({
    q: "needle",
    scope: traceScope,
    cursor: expiredCursor,
    limit: "32",
  });
  expect(api.productSearchRequests[2]).toEqual({
    q: "needle",
    scope: traceScope,
    cursor: null,
    limit: "32",
  });
});

test("an expired first trace page is content that is gone, not no results", async ({
  page,
}) => {
  const traceScope = "trace:session-search";
  await openShell(page, {
    searchCorpus: { [traceScope]: [traceHit("session-search", 11, "hit", 1, NOW)] },
    searchExpiredScopes: [traceScope],
  });

  await openSearchSurface(page, "跨会话与事件记录搜索");
  const view = page.getByRole("dialog", { name: "跨语料搜索" });
  await view.locator("#product-search-scope").selectOption("trace");
  await view.locator("#product-search-term").fill("needle");
  await view.locator("#product-search-term").press("Enter");

  await expect(view).toContainText("事件记录可能已被清理");
  await expect(view).not.toContainText("这些 run 的事件记录里没有匹配内容。");
});

test("a workspace hit opens the session it came from", async ({ page }) => {
  const { workspace, other } = await openShell(page, {
    searchCorpus: {
      // `createMockWorkspace()`'s default id: the canonical scope the view asks
      // for is `workspace:<id>`.
      "workspace:workspace-1": [
        messageHit("session-other", 4, "…release checklist…"),
      ],
    },
  });

  await openSearchSurface(page, "跨会话与事件记录搜索");
  const view = page.getByRole("dialog", { name: "跨语料搜索" });
  await view.locator("#product-search-scope").selectOption("workspace");
  await view.locator("#product-search-term").fill("release");
  await view.locator("#product-search-term").press("Enter");

  await expect(view).toContainText("…release checklist…");
  await view.getByRole("button", { name: "打开会话" }).click();

  // The hit names another session; opening it is the jump this scope offers.
  await expect(page.locator(".chat-pane__header h1")).toHaveText(
    "Release checklist",
  );
  expect(page.url()).toContain(`/w/${workspace.id}/s/${other.id}`);
});