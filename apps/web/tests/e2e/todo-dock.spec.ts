import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * The composer todo fold (design §6): the run's plan folds into a one-row
 * header above the queue — progress plus the step the run is on — and opens
 * into the full step list.
 */
async function openSessionWithPlan(page: Page) {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = completedTranscript(
    workspace,
    session,
    "Build the feature",
    "Done.",
  );
  // A plan_created row in the last segment is what the restored run surfaces.
  const segment = transcript.segments[0]! as {
    events: Array<{ seq: number; event: Record<string, unknown> }>;
    observed_through_seq: number;
    last_event_seq: number;
  };
  segment.events = [
    segment.events[0]!,
    {
      seq: 2,
      event: {
        type: "plan_created",
        plan: {
          goal: "Build the feature",
          current_step: 1,
          steps: [
            { id: "s1", title: "Read the spec", done: true },
            { id: "s2", title: "Write the code", done: false },
            { id: "s3", title: "Run the tests", done: false },
          ],
        },
      },
    },
    ...segment.events.slice(1),
  ].map((stored, index) => ({ ...stored, seq: index + 1 }));
  // The segment's observed seq must match the last delivered event.
  segment.observed_through_seq = segment.events.at(-1)!.seq;
  segment.last_event_seq = segment.events.at(-1)!.seq;
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(
    page.getByTestId("conversation-log").getByText("Build the feature"),
  ).toBeVisible();
}

test("the plan fold shows progress and expands into the step list", async ({
  page,
}) => {
  await openSessionWithPlan(page);

  const dock = page.locator(".todo-dock");
  const header = dock.getByRole("button", { name: "运行计划" });
  // Header reads "n/m done · current item" collapsed.
  await expect(header).toBeVisible();
  await expect(header).toContainText("1/3");
  await expect(header).toContainText("Write the code");

  await header.click();
  const rows = dock.locator(".todo-dock__list li");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toHaveAttribute("data-done", "true");
  await expect(rows.nth(1)).toHaveAttribute("data-current", "true");

  // Fold it back — the list unmounts and the header keeps the summary.
  await header.click();
  await expect(dock.locator(".todo-dock__list")).toHaveCount(0);
});

test("a session with no plan keeps the composer stack clean", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(
    page.getByTestId("conversation-log").getByText("Restored question"),
  ).toBeVisible();
  await expect(page.locator(".todo-dock")).toHaveCount(0);
});
