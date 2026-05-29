import { expect, test, type Page, type Route } from "@playwright/test";

type MockMode = "completed" | "approval";

test("creates a job, receives SSE events, and renders completed state", async ({
  page,
}) => {
  await installMockApi(page, "completed");

  await page.goto("/");
  await page.getByLabel("Task").fill("Summarize the runtime state");
  await page.getByRole("button", { name: "Run" }).click();

  const conversation = page.locator(".message-stream");
  const summary = page.getByLabel("Run summary");
  await expect(conversation.getByText("Summarize the runtime state")).toBeVisible();
  await expect(conversation.getByText("Runtime summary complete")).toBeVisible();
  await expect(summary.getByText("Run completed: final").first()).toBeVisible();
  await expect(summary.getByText("run-comple")).toBeVisible();
});

test("renders pending approval and submits approval through the API", async ({
  page,
}) => {
  await installMockApi(page, "approval");

  await page.goto("/");
  await page.getByLabel("Task").fill("Write a note");
  await page.getByRole("button", { name: "Run" }).click();

  const tools = page.getByLabel("Run details");
  await expect(page.getByText("pending approval")).toBeVisible();
  await expect(
    tools
      .getByText("destructive tool requires explicit approval")
      .first(),
  ).toBeVisible();

  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.url().includes("/approvals/") && response.status() === 200,
    ),
    page.getByRole("button", { name: "Approve" }).click(),
  ]);

  await expect(
    page.getByLabel("Run summary").getByText("Run completed").first(),
  ).toBeVisible();
  await expect(page.locator(".message-stream").getByText("Approved write completed")).toBeVisible();
});

test("starts a resume-latest job and displays resumed source identity", async ({
  page,
}) => {
  let sawResumeLatest = false;
  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as { resume?: string };
    sawResumeLatest = body.resume === "latest";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: "job-resume-1",
        run_id: "run-resume-2",
        resumed_from_run_id: "run-resume-1",
      }),
    });
  });
  await page.route(/\/api\/jobs\/[^/]+\/events$/, async (route) => {
    await route.fulfill({
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
      },
      body: [
        sse("run_started", 1, {
          type: "run_started",
          job_id: "job-resume-1",
          run_id: "run-resume-2",
          user_message: "Continue the last run",
        }),
        sse("run_completed", 2, {
          type: "run_completed",
          reason: "final",
          output: "Resume complete",
        }),
      ].join(""),
    });
  });

  await page.goto("/");
  await page.getByLabel("Task").fill("Continue the last run");
  await page.getByRole("button", { name: "Resume" }).click();

  await expect
    .poll(() => sawResumeLatest, {
      message: "resume button should send resume latest payload",
    })
    .toBe(true);
  const summary = page.getByLabel("Run summary");
  await expect(summary.getByText("from run-resume")).toBeVisible();
  await expect(page.locator(".message-stream").getByText("Resume complete")).toBeVisible();
});

async function installMockApi(page: Page, mode: MockMode) {
  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as { message?: string };
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: mode === "approval" ? "job-approval-1" : "job-complete-1",
        run_id: mode === "approval" ? "run-approval-1" : "run-complete-1",
        received_message: body.message,
      }),
    });
  });

  await page.route(/\/api\/jobs\/[^/]+\/events$/, async (route) => {
    await route.fulfill({
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
      },
      body:
        mode === "approval"
          ? approvalEventStream()
          : completedEventStream(),
    });
  });

  await page.route(/\/api\/jobs\/[^/]+\/state$/, async (route) => {
    await fulfillJobState(route, mode, "running");
  });

  await page.route(/\/api\/jobs\/[^/]+\/approvals\/[^/]+$/, async (route) => {
    await fulfillJobState(route, "approval", "done");
  });
}

async function fulfillJobState(
  route: Route,
  mode: MockMode,
  status: "running" | "done",
) {
  const approvalPending = mode === "approval" && status === "running";
  await route.fulfill({
    contentType: "application/json",
    body: JSON.stringify({
      job_id: mode === "approval" ? "job-approval-1" : "job-complete-1",
      run_id: mode === "approval" ? "run-approval-1" : "run-complete-1",
      status,
      event_count: approvalPending ? 2 : 4,
      events: status === "done" ? approvalCompletedEvents() : [],
      pending_approvals: approvalPending
        ? [
            {
              call_id: "call-approval-1",
              name: "fs_write",
              args: { path: "notes.md" },
              reason: "destructive tool requires explicit approval",
            },
          ]
        : [],
      pending_inputs: [],
    }),
  });
}

function completedEventStream(): string {
  return [
    sse("run_started", 1, {
      type: "run_started",
      job_id: "job-complete-1",
      run_id: "run-complete-1",
      user_message: "Summarize the runtime state",
    }),
    sse("llm_chunk", 2, {
      type: "llm_chunk",
      delta: "Runtime summary",
    }),
    sse("llm_message", 3, {
      type: "llm_message",
      full: "Runtime summary complete",
      usage: {
        prompt_tokens: 4,
        completion_tokens: 3,
        total_tokens: 7,
      },
    }),
    sse("run_completed", 4, {
      type: "run_completed",
      reason: "final",
      output: "Runtime summary complete",
    }),
  ].join("");
}

function approvalEventStream(): string {
  return [
    sse("run_started", 1, {
      type: "run_started",
      job_id: "job-approval-1",
      run_id: "run-approval-1",
      user_message: "Write a note",
    }),
    sse("tool_call_approval_needed", 2, {
      type: "tool_call_approval_needed",
      call_id: "call-approval-1",
      name: "fs_write",
      args: { path: "notes.md" },
      reason: "destructive tool requires explicit approval",
    }),
  ].join("");
}

function approvalCompletedEvents() {
  return [
    {
      seq: 3,
      event: {
        type: "tool_call_completed",
        call_id: "call-approval-1",
        result: {
          call_id: "call-approval-1",
          output: "Approved write completed",
        },
      },
    },
    {
      seq: 4,
      event: {
        type: "run_completed",
        reason: "final",
        output: "Approved write completed",
      },
    },
  ];
}

function sse(event: string, id: number, data: unknown): string {
  return `id: ${id}\nevent: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
}
