import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockSession,
  type MockTranscript,
  type MockWorkspace,
} from "./product-api-mock";

/**
 * Conversation minimap.
 *
 * The rail is only useful if its markers map to the turns that are actually on
 * screen and if using it is a reading move rather than a scroll that follow mode
 * immediately undoes.
 */

const ANSWER = Array.from(
  { length: 60 },
  (_value, index) => `answer line ${index + 1}`,
).join("\n\n");

function multiTurnTranscript(
  workspace: MockWorkspace,
  session: MockSession,
  turns: Array<{ question: string; answer: string }>,
): MockTranscript {
  const base = completedTranscript(workspace, session, turns[0].question, turns[0].answer);
  const segments = turns.map((turn, index) => {
    const ordinal = index + 1;
    const jobId = `job-restored-${ordinal}`;
    const runId = `run-restored-${ordinal}`;
    const events = [
      { seq: 1, event: { type: "run_started", job_id: jobId, run_id: runId, user_message: turn.question } },
      { seq: 2, event: { type: "llm_chunk", delta: turn.answer } },
      { seq: 3, event: { type: "llm_message", full: turn.answer, usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 } } },
      { seq: 4, event: { type: "run_completed", reason: "final", output: turn.answer } },
    ];
    return {
      ...(base.segments[0] as Record<string, unknown>),
      binding: {
        ...((base.segments[0] as { binding: Record<string, unknown> }).binding),
        ordinal,
        runtime_job_id: jobId,
        runtime_run_id: runId,
      },
      run_status: "done",
      observed_through_seq: 4,
      last_event_seq: 4,
      events,
    };
  });
  const last = turns.length;
  return {
    ...base,
    segments,
    status: "complete",
  };
}

async function openMultiTurnTranscript(page: Page) {
  // The rail needs a lane beside the 840px reading cap: at 1440 the pane is
  // exactly the cap (measured), so this opens the shell wide enough for one.
  await page.setViewportSize({ width: 1680, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = multiTurnTranscript(workspace, session, [
    { question: "First question", answer: ANSWER },
    { question: "Second question", answer: ANSWER },
    { question: "Third question", answer: ANSWER },
  ]);
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const scroller = page.getByTestId("conversation-log");
  await expect(scroller.getByText("Third question")).toBeVisible();
  return scroller;
}

test("the minimap marks every rendered turn of a long conversation", async ({ page }) => {
  await openMultiTurnTranscript(page);

  const markers = page.locator(".conversation-minimap__marker");
  // One user marker and one assistant marker per turn, in order. The label is
  // asserted by position and role, not by copy: the shell ships zh-CN and en-US.
  await expect(markers).toHaveCount(6);
  await expect(markers.first()).toHaveAttribute("data-role", "user");
  await expect(markers.nth(1)).toHaveAttribute("data-role", "assistant");
  await expect(markers.first()).toHaveAttribute("aria-label", /1\D+6/);
  await expect(markers.nth(1)).toHaveAttribute("aria-label", /2\D+6/);
  // The tooltip preview is the turn's own text, capped for the rail.
  await expect(markers.first()).toHaveAttribute("title", "First question");
  await expect(markers.nth(1)).toHaveAttribute("title", /^answer line 1/);
  await expect(markers.first()).toBeVisible();
});

test("marker positions map to the turns' own place in the transcript", async ({ page }) => {
  await openMultiTurnTranscript(page);

  // Each marker's top is its turn's fraction of the scrollable content, so the
  // tops are strictly increasing and the last marker sits below the first.
  const tops = await page
    .locator(".conversation-minimap__marker")
    .evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect().top));
  for (let index = 1; index < tops.length; index += 1) {
    expect(tops[index]).toBeGreaterThan(tops[index - 1]);
  }
  expect(tops[5] - tops[0]).toBeGreaterThan(60);

  // The translucent band tracks the visible window: opened pinned to the
  // newest turn, it sits at the bottom of the rail.
  const railBottom = await page
    .locator(".conversation-minimap")
    .evaluate((node) => node.getBoundingClientRect().bottom);
  const bandBottom = await page
    .locator(".conversation-minimap__viewport")
    .evaluate((node) => node.getBoundingClientRect().bottom);
  expect(Math.abs(bandBottom - railBottom)).toBeLessThan(4);

  // A real navigation move — the marker jump — releases follow and moves the
  // band up with the viewport. The jump scrolls smoothly, so the band's final
  // position is polled, not read in the same frame.
  await page.locator(".conversation-minimap__marker").first().click();
  const railTop = await page
    .locator(".conversation-minimap")
    .evaluate((node) => node.getBoundingClientRect().top);
  await expect
    .poll(async () =>
      page
        .locator(".conversation-minimap__viewport")
        .evaluate((node) => node.getBoundingClientRect().top),
    )
    .toBeLessThan(railTop + 120);
});

test("a marker jump scrolls the turn into view and keeps follow released", async ({ page }) => {
  const scroller = await openMultiTurnTranscript(page);

  // Opens pinned to the newest turn, so the jump control is absent.
  await expect(page.locator("button.return-to-latest")).toHaveCount(0);

  await page.locator(".conversation-minimap__marker").first().click();

  // The first turn is now inside the viewport, and follow stays released: an
  // older turn is a reading position, not a clamp to correct.
  const firstQuestion = scroller.getByText("First question");
  await expect
    .poll(async () =>
      firstQuestion.evaluate((node) => {
        const scrollerNode = node.closest(".chat-transcript");
        if (!scrollerNode) {
          return false;
        }
        const turn = node.getBoundingClientRect();
        const viewport = scrollerNode.getBoundingClientRect();
        return turn.top >= viewport.top - 1 && turn.bottom <= viewport.bottom + 1;
      }),
    )
    .toBe(true);
  await expect(page.locator("button.return-to-latest")).toBeVisible();
});

test("the rail is one tab stop and arrow keys move within it", async ({ page }) => {
  await openMultiTurnTranscript(page);

  const markers = page.locator(".conversation-minimap__marker");
  await markers.first().focus();
  await page.keyboard.press("ArrowDown");
  await expect(markers.nth(1)).toBeFocused();
  await page.keyboard.press("End");
  await expect(markers.nth(5)).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(markers.nth(4)).toBeFocused();
});
