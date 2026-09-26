import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockSession,
  type MockTranscript,
} from "./product-api-mock";

/**
 * Activity and tool durations (design F2).
 *
 * Canonical events carry no timestamps, so a live duration is measured from
 * when this client saw the events arrive, while a finished MCP call publishes
 * its own `duration_ms`. The two must not be confused: a restored turn has no
 * arrival history, so it may only show the runtime's own number.
 */

const NOW = "2026-09-26T00:00:00.000Z";

type MockSegment = MockTranscript["segments"][number];

function segment(
  session: MockSession,
  ordinal: number,
  jobId: string,
  runId: string,
  events: Array<{ seq: number; event: Record<string, unknown> }>,
): MockSegment {
  return {
    binding: {
      product_session_id: session.id,
      ordinal,
      runtime_session_id: `runtime-${session.id}`,
      runtime_job_id: jobId,
      runtime_run_id: runId,
      bound_at: NOW,
    },
    inherited: false,
    run_status: "done",
    observed_through_seq: events.at(-1)?.seq ?? 0,
    last_event_seq: events.at(-1)?.seq ?? 0,
    events,
  };
}

/** A finished turn whose tool call published the runtime's own duration. */
function durationTranscript(
  workspaceId: string,
  session: MockSession,
  durationMs: number,
): MockTranscript {
  const jobId = "job-duration-1";
  const runId = "run-duration-1";
  session.runtime_binding = {
    ordinal: 1,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: jobId,
    latest_run_id: runId,
  };
  const events: Array<{ seq: number; event: Record<string, unknown> }> = [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: "Read the notes" },
    },
    {
      seq: 2,
      event: {
        type: "tool_call_started",
        call_id: "call-1",
        name: "mcp__notes__read",
        args: { path: "notes.md" },
      },
    },
    {
      seq: 3,
      event: {
        type: "tool_call_completed",
        call_id: "call-1",
        result: {
          call_id: "call-1",
          output: "notes.md read",
          metadata: {
            status: "ok",
            risk_level: "low",
            read_only: true,
            affected_paths: ["notes.md"],
            workspace_changed: false,
            diff_summary: [],
          },
          envelope: {
            outcome: "success",
            summary_text: "notes.md read",
            protocol_metadata: {
              protocol: "mcp",
              remote_tool_name: "read",
              duration_ms: durationMs,
            },
          },
        },
      },
    },
    { seq: 4, event: { type: "llm_chunk", delta: "Read the notes." } },
    {
      seq: 5,
      event: {
        type: "llm_message",
        full: "Read the notes.",
        usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 },
      },
    },
    { seq: 6, event: { type: "run_completed", reason: "final", output: "Read the notes." } },
  ];
  return {
    product_session_id: session.id,
    workspace_id: workspaceId,
    status: "complete",
    partial_reasons: [],
    segments: [segment(session, 1, jobId, runId, events)],
  };
}

async function sendMessage(page: import("@playwright/test").Page, message: string) {
  await page.getByRole("textbox", { name: /输入消息/ }).fill(message);
  await page.getByRole("button", { name: "发送" }).click();
}

function elapsedSeconds(text: string | null): number {
  return Number(/(\d+)/u.exec(text ?? "")?.[1] ?? -1);
}

test("a live turn's activity head counts the seconds it has been working", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    mode: "running_tool",
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await sendMessage(page, "Run the tests");

  const elapsed = page.locator(".activity-group__elapsed");
  await expect(elapsed).toBeVisible();
  await expect(elapsed).toContainText("已运行");

  // The head is a ticker, not a single reading: the same element keeps moving.
  await expect
    .poll(async () => elapsedSeconds(await elapsed.textContent()), { timeout: 10_000 })
    .toBeGreaterThanOrEqual(1);
  const first = elapsedSeconds(await elapsed.textContent());
  await expect
    .poll(async () => elapsedSeconds(await elapsed.textContent()), { timeout: 10_000 })
    .toBeGreaterThan(first);
});

test("a restored turn shows the published duration and no measured elapsed", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: durationTranscript(workspace.id, session, 2_400) },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByLabel("Conversation").getByText("Read the notes.", { exact: true })).toBeVisible();

  // Nothing arrived live, so the head must not claim a measured duration.
  await expect(page.locator(".activity-group__elapsed")).toHaveCount(0);

  await page.locator("button.activity-group__head").click();
  const badge = page.locator(".tool-card__duration");
  await expect(badge).toHaveText("2.4s");

  await page.locator("button.tool-card__head").click();
  const details = page.locator(".tool-card__details");
  await expect(details).toContainText("耗时");
  await expect(details).toContainText("2400 ms");
});

test("a sub-second published duration is not badged", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: durationTranscript(workspace.id, session, 900) },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByLabel("Conversation").getByText("Read the notes.", { exact: true })).toBeVisible();

  await page.locator("button.activity-group__head").click();
  await expect(page.locator(".tool-card__duration")).toHaveCount(0);

  // The expanded card still states the exact value it was given.
  await page.locator("button.tool-card__head").click();
  await expect(page.locator(".tool-card__details")).toContainText("900 ms");
});
