import { expect, test, type Page, type Route } from "@playwright/test";

test("empty → open workspace → run → complete on live shell mock", async ({ page }) => {
  await installShellMocks(page, "completed");

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Open a workspace to start" })).toBeVisible();

  await page.getByLabel("Absolute path").fill("D:/tmp/rove-shell-demo");
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();

  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
  await page.getByRole("textbox", { name: "Message" }).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "Send" }).click();

  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("Summarize the runtime state")).toBeVisible();
  await expect(conversation.getByText("Runtime summary complete")).toBeVisible();
  await expect(page.getByLabel("Run inspector").getByText("Run completed", { exact: true })).toBeVisible();
});

test("inline approval works in product shell", async ({ page }) => {
  await installShellMocks(page, "approval");

  await page.goto("/");
  await page.getByLabel("Absolute path").fill("D:/tmp/rove-shell-demo");
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();
  await page.getByRole("textbox", { name: "Message" }).fill("Write a note");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByLabel("Pending approval")).toBeVisible();
  await expect(
    page.getByLabel("Pending approval").getByText("destructive tool requires explicit approval"),
  ).toBeVisible();

  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.url().includes("/approvals/") && response.status() === 200,
    ),
    page.getByRole("button", { name: "Approve" }).click(),
  ]);

  await expect(page.getByLabel("Conversation").getByText("Approved write completed")).toBeVisible();
});

test("second turn hard-resumes with resume latest and workspace root", async ({ page }) => {
  const jobs: Array<{
    message?: string;
    resume?: string;
    workspace?: { kind?: string; root?: string };
  }> = [];

  await installRunsMock(page);
  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as {
      message?: string;
      resume?: string;
      workspace?: { kind?: string; root?: string };
    };
    jobs.push(body);
    const index = jobs.length;
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: `job-${index}`,
        run_id: `run-${index}`,
        resumed_from_run_id: index === 2 ? "run-1" : null,
      }),
    });
  });
  await page.route(/\/api\/jobs\/[^/]+\/events$/, async (route) => {
    const index = jobs.length;
    await route.fulfill({
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
      },
      body: [
        sse("run_started", 1, {
          type: "run_started",
          job_id: `job-${index}`,
          run_id: `run-${index}`,
          user_message: jobs[index - 1]?.message ?? "turn",
        }),
        sse("run_completed", 2, {
          type: "run_completed",
          reason: "final",
          output: index === 1 ? "First turn done" : "Second turn done",
        }),
      ].join(""),
    });
  });

  await page.goto("/");
  await page.getByLabel("Absolute path").fill("D:/tmp/rove-shell-demo");
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();

  await page.getByRole("textbox", { name: "Message" }).fill("First turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByLabel("Conversation").getByText("First turn done")).toBeVisible();
  await expect(page.getByText("next turn: hard resume (latest)")).toBeVisible();

  await page.getByRole("textbox", { name: "Message" }).fill("Second turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByLabel("Conversation").getByText("Second turn done")).toBeVisible();

  await expect
    .poll(() => jobs.length, { message: "two jobs should be created" })
    .toBe(2);
  expect(jobs[0]?.resume).toBeUndefined();
  expect(jobs[0]?.workspace).toEqual({
    kind: "folder",
    root: "D:/tmp/rove-shell-demo",
  });
  expect(jobs[1]?.resume).toBe("latest");
  expect(jobs[1]?.workspace).toEqual({
    kind: "folder",
    root: "D:/tmp/rove-shell-demo",
  });
  await expect(page.getByLabel("Run inspector").getByText("run-1").first()).toBeVisible();
});

test("theme toggle flips data-theme on the document", async ({ page }) => {
  await installRunsMock(page);
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "Switch to dark theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "Switch to light theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("inspector shows empty then loading states during a run", async ({ page }) => {
  await installShellMocks(page, "completed");
  await page.goto("/");
  await page.getByLabel("Absolute path").fill("D:/tmp/rove-shell-demo");
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();

  const inspector = page.getByLabel("Run inspector");
  await expect(inspector.getByText("No active run")).toBeVisible();
  await expect(inspector.getByText(/Plan, tools, and approvals/)).toBeVisible();

  await page.getByRole("textbox", { name: "Message" }).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByLabel("Conversation").getByText("Runtime summary complete")).toBeVisible();
  await expect(inspector.getByText("Run completed", { exact: true })).toBeVisible();
});

test("sidebar shows running badges for parallel sessions", async ({ page }) => {
  await installShellMocks(page, "approval");
  await page.goto("/");
  await page.getByLabel("Absolute path").fill("D:/tmp/rove-shell-demo");
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();
  await page.getByRole("textbox", { name: "Message" }).fill("Write a note");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByLabel("Pending approval")).toBeVisible();
  // Approval pauses the turn as needs_attention; running badge is used while busy.
  await expect(
    page
      .locator(".session-badge")
      .filter({ hasText: /Running|Attention|Needs attention/i })
      .first(),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /Write a note/ })).toBeVisible();

  await page.getByRole("button", { name: "New session" }).click();
  // Original session remains non-idle while a second session exists in the workspace.
  await expect(
    page
      .locator(".session-badge")
      .filter({ hasText: /Running|Attention|Needs attention/i })
      .first(),
  ).toBeVisible();
});

test("benchmark lives under Settings Advanced, not primary nav", async ({ page }) => {
  await installRunsMock(page);
  await page.route(/\/api\/bench\/.*/, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ suites: [], runs: [] }),
    });
  });

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Benchmarks" })).toHaveCount(0);
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "Advanced / Developer" }).click();
  await expect(page.getByRole("heading", { name: "Advanced / Developer" })).toBeVisible();
  await page.getByRole("button", { name: /Benchmark runner/ }).click();
  await expect(page.getByLabel("Benchmark runner")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Benchmark Runner" })).toBeVisible();
});

test("settings providers can test and list models without raw keys", async ({ page }) => {
  let sawModels = false;
  let sawTest = false;
  await installRunsMock(page);
  await page.route("/api/providers/models", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { api_key_env?: string; api_base?: string };
    };
    sawModels =
      body.provider?.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY" &&
      !JSON.stringify(body).includes("sk-");
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        provider: "gateway.test",
        provider_type: "openai",
        wire_protocol: "openai_chat",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        models: ["relay/model-a", "relay/model-b"],
        models_count: 2,
      }),
    });
  });
  await page.route("/api/providers/test", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { api_key_env?: string; api_base?: string };
    };
    sawTest =
      body.provider?.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "pass",
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        model: "relay/model-a",
        model_present: true,
        models_count: 2,
      }),
    });
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByRole("heading", { name: "Providers & Models" })).toBeVisible();

  await page.getByLabel("API base").fill("https://gateway.test/v1");
  await page.getByLabel("API key env name").fill("GATEWAY_API_KEY");
  await page.getByLabel("Default model").fill("relay/model-a");
  await page.getByRole("button", { name: "List models" }).click();
  await expect(page.getByText(/Models \(2\):/)).toBeVisible();
  await page.getByRole("button", { name: "Test" }).click();
  await expect(page.getByText(/Test: pass/)).toBeVisible();

  await expect.poll(() => sawModels).toBe(true);
  await expect.poll(() => sawTest).toBe(true);
});

type MockMode = "completed" | "approval";

async function installShellMocks(page: Page, mode: MockMode) {
  await installRunsMock(page);

  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as {
      message?: string;
      workspace?: { kind?: string; root?: string };
    };
    expect(body.workspace?.root).toBeTruthy();
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: mode === "approval" ? "job-approval-1" : "job-complete-1",
        run_id: mode === "approval" ? "run-approval-1" : "run-complete-1",
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
      body: mode === "approval" ? approvalEventStream() : completedEventStream(),
    });
  });

  await page.route(/\/api\/jobs\/[^/]+\/state$/, async (route) => {
    await fulfillJobState(route, mode, "running");
  });

  await page.route(/\/api\/jobs\/[^/]+\/approvals\/[^/]+$/, async (route) => {
    await fulfillJobState(route, "approval", "done");
  });
}

async function installRunsMock(page: Page) {
  await page.route(/\/api\/runs(?:\?.*)?$/, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ runs: [] }),
    });
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
              name: "write_file",
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
      name: "write_file",
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
