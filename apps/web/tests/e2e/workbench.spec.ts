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
  await installRunsMock(page);
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

test("loads models, tests, and submits an OpenAI provider profile", async ({ page }) => {
  let sawProviderModels = false;
  let sawProviderTest = false;
  let sawProviderJob = false;
  await installRunsMock(page);
  await page.route("/api/providers/models", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: {
        provider_type?: string;
        name?: string;
        api_base?: string;
        api_key_env?: string;
      };
    };
    sawProviderModels =
      body.provider?.provider_type === "openai" &&
      body.provider.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        models: ["relay/deepseek-v3.2", "official/gpt-compatible"],
        models_count: 2,
      }),
    });
  });
  await page.route("/api/providers/test", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: {
        provider_type?: string;
        name?: string;
        api_base?: string;
        api_key_env?: string;
      };
      model?: string;
    };
    sawProviderTest =
      body.provider?.provider_type === "openai" &&
      body.provider.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY" &&
      body.model === "relay/deepseek-v3.2";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "pass",
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        model: "relay/deepseek-v3.2",
        model_present: true,
        models_count: 2,
      }),
    });
  });
  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: {
        provider_type?: string;
        name?: string;
        api_base?: string;
        api_key_env?: string;
      };
      model?: string;
    };
    sawProviderJob =
      body.provider?.provider_type === "openai" &&
      body.provider.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY" &&
      body.model === "relay/deepseek-v3.2";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: "job-provider-1",
        run_id: "run-provider-1",
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
          job_id: "job-provider-1",
          run_id: "run-provider-1",
          user_message: "Use the configured provider",
        }),
        sse("run_completed", 2, {
          type: "run_completed",
          reason: "final",
          output: "Provider run complete",
        }),
      ].join(""),
    });
  });

  await page.goto("/");
  await page.getByLabel("Type").selectOption("openai");
  await page.getByLabel("API base").fill("https://gateway.test/v1");
  await page.getByLabel("Key env").fill("GATEWAY_API_KEY");
  await page.getByRole("button", { name: "Load models" }).click();
  await expect(page.getByText("2 models available")).toBeVisible();
  await page.getByLabel("Model").fill("relay/deepseek-v3.2");
  await page.getByRole("button", { name: "Test" }).click();
  await expect(page.getByText("selected model ready")).toBeVisible();
  await page.getByLabel("Task").fill("Use the configured provider");
  await page.getByRole("button", { name: "Run" }).click();

  await expect
    .poll(() => sawProviderModels, {
      message: "provider models should send profile without key value",
    })
    .toBe(true);
  await expect
    .poll(() => sawProviderTest, {
      message: "provider test should send profile without key value",
    })
    .toBe(true);
  await expect
    .poll(() => sawProviderJob, {
      message: "job creation should send provider profile",
    })
    .toBe(true);
  await expect(page.locator(".message-stream").getByText("Provider run complete")).toBeVisible();
});

test("tests and submits an OpenAI Responses provider profile", async ({ page }) => {
  let sawProviderTest = false;
  let sawProviderJob = false;
  await installRunsMock(page);
  await page.route("/api/providers/test", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { provider_type?: string; api_base?: string; api_key_env?: string };
      model?: string;
    };
    sawProviderTest =
      body.provider?.provider_type === "openai-responses" &&
      body.provider.api_base === "https://api.openai.com/v1" &&
      body.provider.api_key_env === "OPENAI_API_KEY" &&
      body.model === "gpt-4.1-mini";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "pass",
        provider: "openai-responses",
        provider_type: "openai-responses",
        api_base: "https://api.openai.com/v1",
        key_env: "OPENAI_API_KEY",
        key_present: true,
        model: "gpt-4.1-mini",
        model_present: true,
        models_count: 8,
      }),
    });
  });
  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { provider_type?: string; api_base?: string; api_key_env?: string };
      model?: string;
    };
    sawProviderJob =
      body.provider?.provider_type === "openai-responses" &&
      body.provider.api_base === "https://api.openai.com/v1" &&
      body.provider.api_key_env === "OPENAI_API_KEY" &&
      body.model === "gpt-4.1-mini";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: "job-responses-1",
        run_id: "run-responses-1",
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
          job_id: "job-responses-1",
          run_id: "run-responses-1",
          user_message: "Use responses provider",
        }),
        sse("run_completed", 2, {
          type: "run_completed",
          reason: "final",
          output: "Responses run complete",
        }),
      ].join(""),
    });
  });

  await page.goto("/");
  await page.getByLabel("Type").selectOption("openai-responses");
  await page.getByLabel("Model").fill("gpt-4.1-mini");
  await page.getByRole("button", { name: "Test" }).click();
  await expect(page.getByText("selected model ready")).toBeVisible();
  await page.getByLabel("Task").fill("Use responses provider");
  await page.getByRole("button", { name: "Run" }).click();

  await expect
    .poll(() => sawProviderTest, {
      message: "responses provider test should send profile without key value",
    })
    .toBe(true);
  await expect
    .poll(() => sawProviderJob, {
      message: "responses job creation should send provider profile",
    })
    .toBe(true);
  await expect(page.locator(".message-stream").getByText("Responses run complete")).toBeVisible();
});

test("submits Anthropic and Ollama provider profiles", async ({ page }) => {
  const jobs: Array<{
    provider?: { provider_type?: string; api_base?: string; api_key_env?: string };
    model?: string;
  }> = [];
  await installRunsMock(page);
  await page.route("/api/providers/test", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { provider_type?: string; api_base?: string; api_key_env?: string };
      model?: string;
    };
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "pass",
        provider: body.provider?.provider_type,
        provider_type: body.provider?.provider_type,
        api_base: body.provider?.api_base,
        key_env: body.provider?.api_key_env ?? "",
        key_present: Boolean(body.provider?.api_key_env),
        model: body.model,
        model_present: true,
        models_count: body.provider?.provider_type === "ollama" ? 1 : 4,
      }),
    });
  });
  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { provider_type?: string; api_base?: string; api_key_env?: string };
      model?: string;
    };
    jobs.push(body);
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        job_id: `job-provider-${jobs.length}`,
        run_id: `run-provider-${jobs.length}`,
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
          job_id: "job-provider",
          run_id: "run-provider",
          user_message: "Use provider",
        }),
        sse("run_completed", 2, {
          type: "run_completed",
          reason: "final",
          output: "Provider run complete",
        }),
      ].join(""),
    });
  });

  await page.goto("/");
  await page.getByLabel("Type").selectOption("anthropic");
  await page.getByLabel("API base").fill("https://api.anthropic.com");
  await page.getByLabel("Key env").fill("ANTHROPIC_API_KEY");
  await page.getByLabel("Model").fill("claude-3-5-haiku-latest");
  await page.getByRole("button", { name: "Run" }).click();

  await expect
    .poll(() => jobs[0]?.provider?.provider_type, {
      message: "anthropic job should send provider profile",
    })
    .toBe("anthropic");
  expect(jobs[0].provider?.api_key_env).toBe("ANTHROPIC_API_KEY");

  await page.getByLabel("Type").selectOption("ollama");
  await page.getByLabel("API base").fill("http://localhost:11434");
  await expect(page.getByLabel("Key env")).toHaveCount(0);
  await page.getByLabel("Model").fill("llama3.2");
  await page.getByRole("button", { name: "Run" }).click();

  await expect
    .poll(() => jobs[1]?.provider?.provider_type, {
      message: "ollama job should send provider profile",
    })
    .toBe("ollama");
  expect(jobs[1].provider?.api_key_env).toBeUndefined();
});

async function installMockApi(page: Page, mode: MockMode) {
  await installRunsMock(page);

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

async function installRunsMock(page: Page) {
  await page.route(/\/api\/runs(?:\?.*)?$/, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ runs: [] }),
    });
  });
  await page.route(/\/api\/runs\/[^/]+\/report$/, async (route) => {
    await route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ error: "not found" }),
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
