import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockSession,
  type MockTranscript,
} from "./product-api-mock";

/**
 * Retained session panes (design §5.1) and per-session view snapshots (F1).
 *
 * The last visited sessions keep their transcript mounted under
 * `visibility:hidden`, so the reader's viewport, mount window and disclosure
 * choices survive a switch in the DOM itself. The snapshot store still owns the
 * reinstate decision for panes whose data was replaced — a session that moved
 * on in the background opens at its newest turn, not where it was left.
 *
 * Every transcript locator below scopes to `data-visible="true"`: a hidden
 * pane is still in the DOM by design, so unscoped queries match its rows too.
 */

function visibleTranscript(page: Page) {
  return page
    .locator('.session-pane[data-visible="true"]')
    .getByLabel("Conversation");
}

const NOW = "2026-09-26T00:00:00.000Z";
const LONG_ANSWER = Array.from(
  { length: 160 },
  (_value, index) => `restored answer line ${index + 1}`,
).join("\n\n");

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

function answerEvents(
  jobId: string,
  runId: string,
  question: string,
  answer: string,
): Array<{ seq: number; event: Record<string, unknown> }> {
  return [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: question },
    },
    { seq: 2, event: { type: "llm_chunk", delta: answer } },
    {
      seq: 3,
      event: {
        type: "llm_message",
        full: answer,
        usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 },
      },
    },
    { seq: 4, event: { type: "run_completed", reason: "final", output: answer } },
  ];
}

function answerTranscript(
  workspaceId: string,
  session: MockSession,
  question: string,
  answer: string,
): MockTranscript {
  const jobId = "job-snapshot-1";
  const runId = "run-snapshot-1";
  session.runtime_binding = {
    ordinal: 1,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: jobId,
    latest_run_id: runId,
  };
  return {
    product_session_id: session.id,
    workspace_id: workspaceId,
    status: "complete",
    partial_reasons: [],
    segments: [segment(session, 1, jobId, runId, answerEvents(jobId, runId, question, answer))],
  };
}

/** A transcript whose first turn holds one finished tool call. */
function toolTranscript(
  workspaceId: string,
  session: MockSession,
  question = "Session A question",
): MockTranscript {
  const jobId = "job-tools-1";
  const runId = "run-tools-1";
  session.runtime_binding = {
    ordinal: 1,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: jobId,
    latest_run_id: runId,
  };
  const events: Array<{ seq: number; event: Record<string, unknown> }> = [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: question },
    },
    {
      seq: 2,
      event: { type: "tool_call_started", call_id: "call-1", name: "read_file", args: { path: "notes.md" } },
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

function appendTurn(transcript: MockTranscript, session: MockSession, answer: string) {
  const ordinal = transcript.segments.length + 1;
  const jobId = `job-snapshot-${ordinal}`;
  const runId = `run-snapshot-${ordinal}`;
  transcript.segments.push(
    segment(
      session,
      ordinal,
      jobId,
      runId,
      answerEvents(jobId, runId, `Follow-up ${ordinal}`, answer),
    ),
  );
  session.runtime_binding = {
    ordinal,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: jobId,
    latest_run_id: runId,
  };
}

async function openSession(page: Page, workspaceId: string, sessionId: string) {
  await page.goto(`/w/${workspaceId}/s/${sessionId}`);
  await expect(visibleTranscript(page)).toBeVisible();
}

async function switchTo(page: Page, title: string) {
  // The rail lists a session once per zone it belongs to (§7); the row's
  // accessible name is comma-joined ("Title, workspace"), so match the title
  // prefix inside a project group where each session resolves once.
  await page
    .locator(".workspace-group")
    .getByRole("button", { name: new RegExp(`^${title}(?:,|$)`) })
    .click();
}

function distanceFromBottom(scroller: Locator) {
  return scroller.evaluate((node) => {
    const element = node as HTMLElement;
    return Math.max(0, element.scrollHeight - element.scrollTop - element.clientHeight);
  });
}

function scrollTop(scroller: Locator) {
  return scroller.evaluate((node) => (node as HTMLElement).scrollTop);
}

test("a session round trip restores the reading position and shows no other session's text", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    transcripts: {
      [sessionA.id]: answerTranscript(workspace.id, sessionA, "Session A question", LONG_ANSWER),
      [sessionB.id]: answerTranscript(workspace.id, sessionB, "Session B question", "Session B answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
  });

  await openSession(page, workspace.id, sessionA.id);
  const scroller = visibleTranscript(page);
  await expect(scroller.getByText("restored answer line 160")).toBeVisible();
  await scroller.hover();
  await page.mouse.wheel(0, -900);
  await expect(page.getByRole("button", { name: /回到最新|Return to latest/ })).toBeVisible();
  const readingPosition = await scrollTop(scroller);
  expect(readingPosition).toBeGreaterThan(0);
  expect(await distanceFromBottom(scroller)).toBeGreaterThan(100);

  await switchTo(page, "Session B");
  await expect(page).toHaveURL(`/w/${workspace.id}/s/${sessionB.id}`);
  await expect(scroller.getByText("Session B question")).toBeVisible();
  await expect(scroller.getByText("restored answer line 160")).toHaveCount(0);
  // §5.1: the session we just left stays mounted — hidden and inert, but its
  // DOM (and its scroll position) is retained for the round trip.
  const hiddenPane = page.locator('.session-pane[data-visible="false"]');
  await expect(hiddenPane.getByText("restored answer line 160")).toBeAttached();
  await expect(hiddenPane.getByText("restored answer line 160")).toBeHidden();

  await switchTo(page, "Session A");
  await expect(page).toHaveURL(`/w/${workspace.id}/s/${sessionA.id}`);
  await expect(scroller.getByText("restored answer line 160")).toBeVisible();

  await expect.poll(() => scrollTop(scroller)).toBeGreaterThan(0);
  const restored = await scrollTop(scroller);
  expect(Math.abs(restored - readingPosition)).toBeLessThanOrEqual(4);
  // Still reading in the middle: the jump control is offered, not a forced tail.
  await expect(page.getByRole("button", { name: /回到最新|Return to latest/ })).toBeVisible();
  expect(await distanceFromBottom(scroller)).toBeGreaterThan(100);
});

test("a session that advanced in the background opens at its newest turn", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    transcripts: {
      [sessionA.id]: answerTranscript(workspace.id, sessionA, "Session A question", LONG_ANSWER),
      [sessionB.id]: answerTranscript(workspace.id, sessionB, "Session B question", "Session B answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
  });

  await openSession(page, workspace.id, sessionA.id);
  const scroller = visibleTranscript(page);
  await expect(scroller.getByText("restored answer line 160")).toBeVisible();
  await scroller.hover();
  await page.mouse.wheel(0, -900);
  await expect(page.getByRole("button", { name: /回到最新|Return to latest/ })).toBeVisible();

  await switchTo(page, "Session B");
  await expect(scroller.getByText("Session B question")).toBeVisible();
  // The session kept working while it was in the background.
  appendTurn(api.transcripts[sessionA.id]!, sessionA, "Background turn answer");

  await switchTo(page, "Session A");
  await expect(scroller.getByText("Background turn answer")).toBeVisible();
  await expect.poll(() => distanceFromBottom(scroller)).toBeLessThanOrEqual(2);
  await expect(page.getByRole("button", { name: /回到最新|Return to latest/ })).toHaveCount(0);
});

test("an expanded activity group stays expanded across a session round trip", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    transcripts: {
      [sessionA.id]: toolTranscript(workspace.id, sessionA),
      [sessionB.id]: answerTranscript(workspace.id, sessionB, "Session B question", "Session B answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
  });

  await openSession(page, workspace.id, sessionA.id);
  const head = page
    .locator('.session-pane[data-visible="true"]')
    .locator("button.activity-group__head");
  await expect(head).toHaveCount(1);
  await expect(head).toHaveAttribute("aria-expanded", "false");
  await head.click();
  await expect(head).toHaveAttribute("aria-expanded", "true");

  await switchTo(page, "Session B");
  await expect(visibleTranscript(page).getByText("Session B question")).toBeVisible();
  await switchTo(page, "Session A");

  const returned = page
    .locator('.session-pane[data-visible="true"]')
    .locator("button.activity-group__head");
  await expect(returned).toHaveAttribute("aria-expanded", "true");
});

test("a cold session switch veils the transcript while the composer stays usable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    transcripts: {
      [sessionA.id]: answerTranscript(workspace.id, sessionA, "Session A question", "Session A answer"),
      [sessionB.id]: answerTranscript(workspace.id, sessionB, "Session B question", "Session B answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
    // §5.5: the veil exists for the window while a cold restore is in flight.
    transcriptDelayMs: { [sessionB.id]: 1200 },
  });

  await openSession(page, workspace.id, sessionA.id);
  await expect(visibleTranscript(page).getByText("Session A question")).toBeVisible();

  await switchTo(page, "Session B");
  // The veil covers the transcript — never the composer — for the in-flight
  // restore, and the 2px switch progress rides the top of the region.
  await expect(page.locator(".session-settle-veil")).toBeVisible();
  await expect(page.locator(".session-switch-progress")).toBeAttached();
  const composer = page.getByRole("textbox", { name: /message|消息/i });
  await expect(composer).toBeEnabled();
  await composer.fill("still typing through the switch");
  await expect(composer).toHaveValue("still typing through the switch");

  await expect(visibleTranscript(page).getByText("Session B question")).toBeVisible();
  await expect(page.locator(".session-settle-veil")).toHaveCount(0);
  await expect(page.locator(".session-switch-progress")).toHaveCount(0);
});

test("the retained pane set is bounded and hidden panes are inert", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const workspace = createMockWorkspace();
  const sessions = Array.from({ length: 6 }, (_value, index) =>
    createMockSession(`session-${index}`, workspace.id, `Session ${index}`),
  );
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions,
    transcripts: Object.fromEntries(
      sessions.map((session) => [
        session.id,
        answerTranscript(
          workspace.id,
          session,
          `Question ${session.id}`,
          `Answer ${session.id}`,
        ),
      ]),
    ),
    activeWorkspaceId: workspace.id,
    activeSessionId: sessions[0].id,
  });

  await openSession(page, workspace.id, sessions[0].id);
  await expect(
    visibleTranscript(page).getByText(`Question ${sessions[0].id}`),
  ).toBeVisible();

  // Walk through every session: the retained set is bounded, so the oldest
  // pane is evicted once the cap is reached.
  for (const session of sessions.slice(1)) {
    await switchTo(page, `Session ${sessions.indexOf(session)}`);
    await expect(
      visibleTranscript(page).getByText(`Question ${session.id}`),
    ).toBeVisible();
  }
  const panes = page.locator(".session-pane");
  await expect(panes).toHaveCount(4);
  // The evicted session left the DOM entirely; the retained ones stay mounted
  // but unreachable: inert, aria-hidden, and out of the visible order.
  await expect(
    page.locator(`text=Question ${sessions[0].id}`),
  ).toHaveCount(0);
  const hidden = page.locator('.session-pane[data-visible="false"]');
  await expect(hidden).toHaveCount(3);
  for (let index = 0; index < 3; index += 1) {
    await expect(hidden.nth(index)).toHaveAttribute("inert", "");
    await expect(hidden.nth(index)).toHaveAttribute("aria-hidden", "true");
  }
});
