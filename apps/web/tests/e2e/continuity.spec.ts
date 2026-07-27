import { expect, test } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

test("root redirects to the server-preferred session and restores its transcript", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = completedTranscript(
    workspace,
    session,
    "Preferred question",
    "Preferred answer",
  );
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto("/");

  await expect(page).toHaveURL(`/w/${workspace.id}/s/${session.id}`);
  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("Preferred question")).toBeVisible();
  await expect(conversation.getByText("Preferred answer")).toBeVisible();
});

test("a deep session URL wins over stale server focus without a wrong-session flash", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  const transcriptA = completedTranscript(
    workspace,
    sessionA,
    "Question A",
    "Answer A",
  );
  const transcriptB = completedTranscript(
    workspace,
    sessionB,
    "Question B",
    "Answer B",
  );
  await page.addInitScript(() => {
    const observed = window as typeof window & { __productSessionHeadings: string[] };
    observed.__productSessionHeadings = [];
    const start = () => {
      const record = () => {
        const heading = document.querySelector(".chat-pane__header h1")?.textContent;
        if (heading && !observed.__productSessionHeadings.includes(heading)) {
          observed.__productSessionHeadings.push(heading);
        }
      };
      new MutationObserver(record).observe(document, {
        childList: true,
        subtree: true,
        characterData: true,
      });
      record();
    };
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", start, { once: true });
    } else {
      start();
    }
  });
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    transcripts: {
      [sessionA.id]: transcriptA,
      [sessionB.id]: transcriptB,
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
  });

  await page.goto(`/w/${workspace.id}/s/${sessionB.id}`);

  await expect(page.getByRole("heading", { name: "Session B" })).toBeVisible();
  await expect(page.getByLabel("Conversation").getByText("Answer B")).toBeVisible();
  const headings = await page.evaluate(
    () =>
      (window as typeof window & { __productSessionHeadings: string[] })
        .__productSessionHeadings,
  );
  expect(headings).not.toContain("Session A");
});

test("a failed deep-route preference write is attempted once", async ({ page }) => {
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
    preferenceUpdateFailures: 10,
  });

  await page.goto(`/w/${workspace.id}/s/${sessionB.id}`);

  await expect(page.getByRole("heading", { name: "Session B" })).toBeVisible();
  await expect(page.locator(".shell-alert")).toContainText(
    "Could not persist preferences",
  );
  await expect.poll(() => api.preferenceUpdateRequests).toBe(1);
  await page.waitForTimeout(600);
  expect(api.preferenceUpdateRequests).toBe(1);
});

test("consecutive preference failures roll back to the confirmed theme", async ({
  page,
}) => {
  const api = await installMockProductApi(page, {
    preferenceUpdateFailures: 2,
    preferenceUpdateDelayMs: 100,
  });

  await page.goto("/settings/general");
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  await page.getByRole("button", { name: "Light", exact: true }).click();

  await expect.poll(() => api.preferenceUpdateRequests).toBe(2);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator(".shell-alert")).toContainText(
    "Could not persist preferences",
  );
  await page.waitForTimeout(400);
  expect(api.preferenceUpdateRequests).toBe(2);
  expect(api.preferences.theme).toBe("light");
});

test("a repeated new-session command creates one durable session", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    sessionCreateDelayMs: 200,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page
    .getByRole("button", { name: "New session", exact: true })
    .evaluate((button: HTMLButtonElement) => {
      button.click();
      button.click();
    });

  await expect(page).toHaveURL(`/w/${workspace.id}/s/session-2`);
  expect(api.sessionCreateRequests).toBe(1);
  expect(api.sessions).toHaveLength(2);
});

test("a completed catalog mutation does not override a newer settings route", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    sessionCreateDelayMs: 300,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("button", { name: "New session", exact: true }).click();
  await page.getByRole("button", { name: "Open settings" }).click();

  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await expect(page.getByRole("heading", { name: "Providers & Models" })).toBeVisible();
  await expect.poll(() => api.sessionCreateRequests).toBe(1);
  await page.waitForTimeout(450);
  await expect(page).toHaveURL(/\/settings\/providers$/u);
  expect(api.sessions).toHaveLength(2);
});

test("removing the active workspace does not override a newer settings route", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    workspaceDeleteDelayMs: 300,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("button", { name: "Remove workspace from list" }).click();
  await page.getByRole("button", { name: "Open settings" }).click();

  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await expect.poll(() => api.workspaces).toHaveLength(0);
  await page.waitForTimeout(450);
  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await expect(page.getByRole("heading", { name: "Providers & Models" })).toBeVisible();
});

test("a missing selected profile fails before adding an optimistic turn", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  api.preferences.provider_selection = {
    profile_id: "provider-missing",
    model: "gpt-missing",
    approval: "ask",
    max_steps: 8,
  };

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("textbox", { name: "Message" }).fill("Do not submit this turn");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(
    page
      .getByLabel("Message composer")
      .getByText(/selected provider profile is no longer available/i),
  ).toBeVisible();
  await expect(
    page.getByLabel("Conversation").getByText("Do not submit this turn", { exact: true }),
  ).toHaveCount(0);
  expect(api.jobs).toHaveLength(0);
});

test("an empty transcript for a live session stays fail-closed when reattach fails", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  session.status = "running";
  session.runtime_binding = {
    ordinal: 1,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: "job-live-missing",
    latest_run_id: "run-live-missing",
  };
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  const restoreError = page.getByRole("alert").filter({
    hasText: "Conversation restore failed",
  });
  await expect(restoreError).toContainText("Live follow could not reconnect");
  await expect(restoreError).toContainText("Durable transcript restore remains available");
  await expect(restoreError.getByRole("button", { name: "Retry restore" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Message" })).toBeDisabled();
  expect(api.jobs).toHaveLength(0);
});

test("reload restores bubbles and the next turn resumes the exact product session", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = completedTranscript(
    workspace,
    session,
    "First turn",
    "First turn done",
  );
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByLabel("Conversation").getByText("First turn done")).toBeVisible();

  await page.reload();
  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("First turn", { exact: true })).toBeVisible();
  await expect(conversation.getByText("First turn done", { exact: true })).toBeVisible();

  await page.getByRole("textbox", { name: "Message" }).fill("Second turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(conversation.getByText("Second turn done")).toBeVisible();

  expect(api.jobs).toHaveLength(1);
  expect(api.jobs[0]).toMatchObject({
    product_session_id: session.id,
    workspace: { kind: "folder", root: workspace.canonical_root },
  });
  expect(api.jobs[0]).not.toHaveProperty("resume");
  expect(api.jobStarts).toEqual([
    {
      job_id: "job-1",
      run_id: "run-1",
      resumed_from_run_id: "run-restored-1",
    },
  ]);
  expect(api.sessions[0]?.runtime_binding).toMatchObject({
    ordinal: 2,
    latest_job_id: "job-1",
    latest_run_id: "run-1",
  });
  await expect(page.getByLabel("Run inspector").getByText("run-restor")).toBeVisible();
});

test("a committed job survives a disconnected response and delayed binding visibility", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    mode: "approval",
    disconnectJobStartResponses: 1,
    jobBindingVisibilityDelayReads: 1,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const conversation = page.getByLabel("Conversation");
  await page.getByRole("textbox", { name: "Message" }).fill("Write after disconnect");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByLabel("Pending approval")).toBeVisible();
  await expect(
    conversation.getByText("Write after disconnect", { exact: true }),
  ).toHaveCount(1);
  expect(api.disconnectedJobStartResponses).toBe(1);
  expect(api.delayedJobBindingReads).toBe(1);
  expect(api.jobs).toHaveLength(1);
  expect(api.jobStarts).toEqual([
    {
      job_id: "job-1",
      run_id: "run-1",
      resumed_from_run_id: null,
    },
  ]);
  expect(api.sessions[0]?.runtime_binding).toMatchObject({
    ordinal: 1,
    latest_job_id: "job-1",
    latest_run_id: "run-1",
  });
  expect(api.transcripts[session.id]?.segments).toHaveLength(1);
  await expect.poll(() => api.eventConnections).toContain("job-1");
});

test("settings sections have durable routes and invalid sections fail explicitly", async ({
  page,
}) => {
  await installMockProductApi(page);

  await page.goto("/settings/memory");
  await expect(page).toHaveURL(/\/settings\/memory$/u);
  await expect(page.getByRole("heading", { name: "Memory" })).toBeVisible();

  await page.goto("/settings");
  await expect(page).toHaveURL(/\/settings\/general$/u);
  await expect(page.getByRole("heading", { name: "General" })).toBeVisible();

  await page.goto("/settings/not-real");
  const routeError = page.locator(".route-state").filter({
    hasText: "Route unavailable",
  });
  await expect(routeError.getByRole("heading", { name: "Route unavailable" })).toBeVisible();
  await expect(routeError).toContainText("not recognized");
});

test("provider profiles save, restore after browser storage clear, and delete durably", async ({
  page,
}) => {
  const api = await installMockProductApi(page);
  const profileLabel = "Continuity OpenAI";
  const defaultModel = "gpt-4.1-e2e";

  await page.goto("/settings/providers");
  await page.getByLabel("Label").fill(profileLabel);
  await page.getByLabel("API base").fill("https://api.example.test/v1");
  await page.getByLabel("API key env name").fill("ROVE_E2E_OPENAI_KEY");
  await page.getByLabel("Default model").fill(defaultModel);
  await page.getByRole("button", { name: "Save profile" }).click();

  const savedRow = page.locator(".profile-row").filter({ hasText: profileLabel });
  await expect(savedRow).toBeVisible();
  await expect.poll(() => api.providerProfiles.length).toBe(1);
  const savedProfile = api.providerProfiles[0]!;
  expect(savedProfile).toMatchObject({
    label: profileLabel,
    provider_type: "openai",
    api_base: "https://api.example.test/v1",
    api_key_env: "ROVE_E2E_OPENAI_KEY",
    default_model: defaultModel,
  });
  await expect
    .poll(() => selectedProfileId(api.preferences))
    .toBe(savedProfile.id);

  await page.evaluate(() => window.localStorage.clear());
  await page.reload();

  await expect(page.getByRole("heading", { name: "Providers & Models" })).toBeVisible();
  await expect(page.locator(".profile-row").filter({ hasText: profileLabel })).toBeVisible();
  await expect(page.getByLabel("Mode", { exact: true })).toHaveValue("profile");
  await expect(page.getByLabel("Profile", { exact: true })).toHaveValue(savedProfile.id);
  await expect(page.getByLabel("Model", { exact: true })).toHaveValue(defaultModel);

  await page
    .locator(".profile-row")
    .filter({ hasText: profileLabel })
    .getByRole("button", { name: "Remove" })
    .click();
  await expect(page.locator(".profile-row").filter({ hasText: profileLabel })).toHaveCount(0);
  await expect.poll(() => api.providerProfiles.length).toBe(0);
  await expect.poll(() => selectedProfileId(api.preferences)).toBeUndefined();

  await page.reload();
  await expect(page.getByText("No saved profiles yet.")).toBeVisible();
  await expect(page.getByLabel("Mode", { exact: true })).toHaveValue("default");
});

test("partial transcript remains distinct from a completed run and a run error", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = completedTranscript(
    workspace,
    session,
    "Visible question",
    "Visible answer",
  );
  transcript.status = "partial";
  transcript.partial_reasons = [
    {
      code: "missing_event_range",
      run_ordinal: 1,
      expected_seq: 3,
      observed_seq: 4,
    },
  ];
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("Partial conversation history")).toBeVisible();
  await expect(conversation.getByText("Visible answer")).toBeVisible();
  await expect(conversation.getByText(/Expected event 3, observed 4/)).toBeVisible();
  await expect(page.getByLabel("Run inspector").getByText("Run completed", { exact: true })).toBeVisible();
  await expect(page.getByText("Run interrupted")).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
});

test("transcript failure is explicit and retry restores the canonical history", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = completedTranscript(
    workspace,
    session,
    "Recovered question",
    "Recovered answer",
  );
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    transcriptFailures: { [session.id]: 1 },
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  const restoreError = page.getByRole("alert").filter({
    hasText: "Conversation restore failed",
  });
  await expect(restoreError).toContainText("transcript store unavailable");
  await expect(restoreError).toContainText("No empty history has been substituted");
  await expect(page.getByRole("textbox", { name: "Message" })).toBeDisabled();

  await restoreError.getByRole("button", { name: "Retry restore" }).click();
  await expect(page.getByLabel("Conversation").getByText("Recovered answer")).toBeVisible();
  await expect(restoreError).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
});

test("a delayed session restore cannot overwrite a faster session switch", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  const transcriptA = completedTranscript(
    workspace,
    sessionA,
    "Question A",
    "Delayed answer A",
  );
  const transcriptB = completedTranscript(
    workspace,
    sessionB,
    "Question B",
    "Fast answer B",
  );
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    transcripts: {
      [sessionA.id]: transcriptA,
      [sessionB.id]: transcriptB,
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
    transcriptDelayMs: { [sessionA.id]: 450 },
  });

  await page.goto(`/w/${workspace.id}/s/${sessionA.id}`);
  await expect(page.getByText("Restoring conversation")).toBeVisible();
  await page.getByRole("button", { name: "Session B", exact: true }).click();

  await expect(page).toHaveURL(`/w/${workspace.id}/s/${sessionB.id}`);
  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("Fast answer B")).toBeVisible();
  await page.waitForTimeout(650);
  await expect(conversation.getByText("Delayed answer A")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Session B" })).toBeVisible();
});

test("a background attention badge survives a new session without an EventSource", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession("session-1", workspace.id, "Attention session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    mode: "approval",
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("textbox", { name: "Message" }).fill("Write a note");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByLabel("Pending approval")).toBeVisible();
  await expect.poll(() => api.eventConnections.length).toBeGreaterThan(0);

  await page.getByRole("button", { name: "New session" }).click();
  await expect(page).toHaveURL(`/w/${workspace.id}/s/session-2`);
  await expect(page.getByText("Send a message to start a run in this session.")).toBeVisible();
  await expect(page.locator('.session-badge[data-status="needs_attention"]').first()).toBeVisible();

  const connectionsAfterSwitch = api.eventConnections.length;
  await page.waitForTimeout(3_500);
  expect(api.eventConnections).toHaveLength(connectionsAfterSwitch);
  expect(api.eventConnections.every((jobId) => jobId === "job-1")).toBe(true);
});

function selectedProfileId(preferences: Record<string, unknown>): string | undefined {
  const selection = preferences.provider_selection;
  if (!selection || typeof selection !== "object") {
    return undefined;
  }
  const profileId = (selection as Record<string, unknown>).profile_id;
  return typeof profileId === "string" ? profileId : undefined;
}
