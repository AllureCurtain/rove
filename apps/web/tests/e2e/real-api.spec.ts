import { mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  expect,
  test,
  type APIRequestContext,
  type Page,
  type Request,
} from "@playwright/test";

import {
  M1_BROWSER_MIGRATION_STATE_KEY,
  M1_BROWSER_STORAGE_KEYS,
} from "../../product/m1-storage-keys";
import type {
  ProductSessionsResponse,
  ProductWorkspacesResponse,
} from "../../product/product-api-types";

const realApiEnabled = process.env.ROVE_REAL_API_E2E === "1";
const advancedWorkbenchEnabled =
  process.env.ROVE_REAL_API_WORKBENCH_SMOKE === "1";

interface ProductPreferencesSnapshot {
  schema_version: number;
  revision: number;
  theme: "light" | "dark" | "system";
  default_approval_policy: "ask" | "auto" | "never";
  active_workspace_id?: string;
  active_session_id?: string;
  provider_selection?: unknown;
}

interface ProductTranscriptSnapshot {
  product_session_id: string;
  status: "complete" | "partial";
  segments: Array<{
    binding: {
      ordinal: number;
      runtime_job_id: string;
      runtime_run_id: string;
      resumed_from_run_id?: string;
    };
  }>;
}

interface StartedTurn {
  body: Record<string, unknown>;
  jobId: string;
  runId: string;
  resumedFromRunId: string | null;
}

interface CreatedProductRecords {
  workspaceIds: string[];
  sessionIds: string[];
  profileIds: string[];
  jobIds: string[];
}

test.describe("real API product shell integration", () => {
  test.skip(
    !realApiEnabled,
    "set ROVE_REAL_API_E2E=1 to run against a real rove-api server",
  );
  test.describe.configure({ mode: "serial" });

  test("migrates browser state before mounting the live product catalog", async ({
    page,
    request,
  }) => {
    test.setTimeout(120_000);
    const baseline = await readPreferences(request);
    const workspaceRoot = await mkdtemp(join(tmpdir(), "rove-live-migration-"));
    const created = emptyCreatedRecords();
    let primaryError: unknown | null = null;
    let migrationPosts = 0;
    let migrationStatus: number | null = null;
    page.on("request", (browserRequest) => {
      if (
        browserRequest.method() === "POST" &&
        new URL(browserRequest.url()).pathname ===
          "/api/product/migrations/m1-browser"
      ) {
        migrationPosts += 1;
      }
    });
    page.on("response", (response) => {
      if (
        response.request().method() === "POST" &&
        new URL(response.url()).pathname ===
          "/api/product/migrations/m1-browser"
      ) {
        migrationStatus = response.status();
      }
    });
    try {
      await detachActiveProductRoute(request, baseline);
      await seedLegacyState(page, workspaceRoot);

      await page.goto(
        "/w/live-legacy-workspace/s/live-legacy-session?inspector=open#latest",
      );
      await expect.poll(() => migrationStatus).toBe(200);
      expect(
        await registerProductRecordsForWorkspaceRoot(
          request,
          created,
          workspaceRoot,
        ),
      ).toBe(1);
      await page.waitForURL((url) => {
        const match = url.pathname.match(/^\/w\/([^/]+)\/s\/([^/]+)$/);
        return (
          match?.[1] !== undefined &&
          match[1] !== "live-legacy-workspace" &&
          match?.[2] !== undefined &&
          match[2] !== "live-legacy-session" &&
          url.search === "?inspector=open" &&
          url.hash === "#latest"
        );
      });
      const routeMatch = new URL(page.url()).pathname.match(
        /^\/w\/([^/]+)\/s\/([^/]+)$/,
      );
      expect(routeMatch).not.toBeNull();
      const [, workspaceId, sessionId] = routeMatch!;
      expect(workspaceId).not.toBe("live-legacy-workspace");
      expect(sessionId).not.toBe("live-legacy-session");

      await expect(page).toHaveURL(
        `/w/${workspaceId}/s/${sessionId}?inspector=open#latest`,
      );
      await expect(
        page.getByRole("heading", { name: "Live imported session" }),
      ).toBeVisible();
      await expect(page.getByText("Browser data imported (2 records).")).toBeVisible();
      expect(migrationPosts).toBe(1);
      await expect
        .poll(() => browserValue(page, M1_BROWSER_MIGRATION_STATE_KEY))
        .not.toBeNull();
      await expect
        .poll(() => browserValue(page, M1_BROWSER_STORAGE_KEYS.workspaces))
        .not.toBeNull();

      await page.reload();
      await expect(
        page.getByRole("heading", { name: "Live imported session" }),
      ).toBeVisible();
      await expect(page.getByText(/Browser data imported/iu)).toHaveCount(0);
      expect(migrationPosts).toBe(1);
    } catch (error) {
      primaryError = error;
    } finally {
      await finalizeProductTest(
        page,
        request,
        created,
        baseline,
        workspaceRoot,
        primaryError,
      );
    }
  });

  test("runs exact A/B continuity, refresh, tools, cancellation, and Settings", async ({
    page,
    request,
  }) => {
    test.setTimeout(180_000);
    const baseline = await readPreferences(request);
    const workspaceRoot = await mkdtemp(join(tmpdir(), "rove-real-api-e2e-"));
    const created = emptyCreatedRecords();
    let primaryError: unknown | null = null;

    try {
      await detachActiveProductRoute(request, baseline);
      await page.goto("/");
      await expect(
        page.getByRole("heading", { name: "Open a workspace to start" }),
      ).toBeVisible();

      const firstSession = await openWorkspace(
        page,
        workspaceRoot,
        created,
      );
      const workspaceId = firstSession.workspaceId;
      const sessionA = firstSession.sessionId;
      const routeA = productSessionRoute(workspaceId, sessionA);

      const promptA1 = `real API A1 ${Date.now()}`;
      const turnA1 = await sendMessage(page, promptA1, created);
      expectProductSessionRequest(turnA1, sessionA);
      expect(turnA1.resumedFromRunId).toBeNull();
      await expectTurnCompleted(page, `fake response: ${promptA1}`);

      const sessionB = await createSession(page, workspaceId, created);
      const promptB1 = `real API B1 ${Date.now()}`;
      const turnB1 = await sendMessage(page, promptB1, created);
      expectProductSessionRequest(turnB1, sessionB);
      expect(turnB1.resumedFromRunId).toBeNull();
      await expectTurnCompleted(page, `fake response: ${promptB1}`);

      await page.goto(routeA);
      await expect(
        page.getByLabel("Conversation").getByText(`fake response: ${promptA1}`),
      ).toBeVisible();
      const promptA2 = `real API A2 ${Date.now()}`;
      const turnA2 = await sendMessage(page, promptA2, created);
      expectProductSessionRequest(turnA2, sessionA);
      expect(turnA2.resumedFromRunId).toBe(turnA1.runId);
      expect(turnA2.resumedFromRunId).not.toBe(turnB1.runId);
      await expectTurnCompleted(page, `fake response: ${promptA2}`);

      await page.reload();
      await expect(page).toHaveURL(routeA);
      const conversation = page.getByLabel("Conversation");
      await expect(conversation.getByText(promptA1, { exact: true })).toBeVisible();
      await expect(conversation.getByText(promptA2, { exact: true })).toBeVisible();
      const promptA3 = `real API A3 ${Date.now()}`;
      const turnA3 = await sendMessage(page, promptA3, created);
      expectProductSessionRequest(turnA3, sessionA);
      expect(turnA3.resumedFromRunId).toBe(turnA2.runId);
      await expectTurnCompleted(page, `fake response: ${promptA3}`);

      const transcriptA = await readTranscript(request, sessionA);
      const transcriptB = await readTranscript(request, sessionB);
      expect(transcriptA.status).toBe("complete");
      expect(transcriptA.segments).toHaveLength(3);
      expect(transcriptA.segments.map((segment) => segment.binding.ordinal)).toEqual([
        1, 2, 3,
      ]);
      expect(
        transcriptA.segments.map((segment) => ({
          jobId: segment.binding.runtime_job_id,
          runId: segment.binding.runtime_run_id,
        })),
      ).toEqual([
        { jobId: turnA1.jobId, runId: turnA1.runId },
        { jobId: turnA2.jobId, runId: turnA2.runId },
        { jobId: turnA3.jobId, runId: turnA3.runId },
      ]);
      expect(transcriptA.segments[1].binding.resumed_from_run_id).toBe(
        turnA1.runId,
      );
      expect(transcriptA.segments[2].binding.resumed_from_run_id).toBe(
        turnA2.runId,
      );
      expect(transcriptB.segments).toHaveLength(1);
      expect(transcriptB.segments[0].binding.runtime_job_id).toBe(turnB1.jobId);
      expect(transcriptB.segments[0].binding.runtime_run_id).toBe(turnB1.runId);

      await page.goto("/settings/tools");
      await page.getByRole("button", { name: "Never", exact: true }).click();
      await expect
        .poll(async () => (await readPreferences(request)).default_approval_policy)
        .toBe("never");
      await page.getByRole("button", { name: "Ask", exact: true }).click();
      await expect
        .poll(async () => (await readPreferences(request)).default_approval_policy)
        .toBe("ask");
      await page.getByLabel("Maximum steps per job").fill("1");
      await Promise.all([
        page.waitForResponse((response) => {
          if (
            response.request().method() !== "PUT" ||
            new URL(response.url()).pathname !== "/api/product/preferences" ||
            !response.ok()
          ) {
            return false;
          }
          try {
            const body = response.request().postDataJSON() as {
              provider_selection?: { max_steps?: number };
            };
            return body.provider_selection?.max_steps === 1;
          } catch {
            return false;
          }
        }),
        page.getByRole("button", { name: "Save limit" }).click(),
      ]);

      await page.goto("/settings/providers");
      const profileId = await selectFakeRawProfile(page, created);
      expect(profileId).toBeTruthy();
      await page.goto(routeA);
      await expect(
        page
          .getByLabel("Message composer")
          .getByRole("button", { name: "Change global next-run model default" })
          .getByText("fake-raw", { exact: true }),
      ).toBeVisible();

      await page.evaluate(() => {
        window.localStorage.clear();
        window.sessionStorage.clear();
      });
      await page.reload();
      await expect(
        page
          .getByLabel("Message composer")
          .getByRole("button", { name: "Change global next-run model default" })
          .getByText("fake-raw", { exact: true }),
      ).toBeVisible();

      const outputName = `approved-${Date.now()}.txt`;
      const outputContent = "ok from the real product shell";
      const approvalTurn = await sendMessage(
        page,
        JSON.stringify({
          tool: "write_file",
          args: { path: outputName, content: outputContent },
        }),
        created,
      );
      expect(approvalTurn.body.max_steps).toBe(1);
      const approval = page.getByLabel("Pending approval");
      await expect(approval.getByText(/Approval needed.*write_file/u)).toBeVisible();
      await Promise.all([
        page.waitForResponse(
          (response) =>
            response.request().method() === "POST" &&
            new URL(response.url()).pathname.includes("/api/jobs/") &&
            new URL(response.url()).pathname.includes("/approvals/") &&
            response.ok(),
        ),
        approval.getByRole("button", { name: "Approve" }).click(),
      ]);
      await expect(
        page.getByLabel("Conversation").getByText(/write_file.*done/u),
      ).toBeVisible({ timeout: 30_000 });
      expect(await readFile(join(workspaceRoot, outputName), "utf8")).toBe(
        outputContent,
      );
      await cancelCurrentRun(page);

      const inputPrompt = "Which branch should the real product shell use?";
      const inputTurn = await sendMessage(
        page,
        JSON.stringify({
          tool: "request_input",
          args: { prompt: inputPrompt },
        }),
        created,
      );
      expect(inputTurn.body.max_steps).toBe(1);
      const inputCard = page.locator(".input-card").filter({ hasText: inputPrompt });
      await expect(inputCard.getByText("Input requested")).toBeVisible();
      await inputCard.getByRole("textbox", { name: inputPrompt }).fill("main");
      await Promise.all([
        page.waitForResponse(
          (response) =>
            response.request().method() === "POST" &&
            new URL(response.url()).pathname.includes("/api/jobs/") &&
            new URL(response.url()).pathname.includes("/inputs/") &&
            response.ok(),
        ),
        inputCard.getByRole("button", { name: "Send" }).click(),
      ]);
      await expect(
        page.getByLabel("Conversation").getByText("main", { exact: true }),
      ).toBeVisible({ timeout: 30_000 });
      await cancelCurrentRun(page);

      const cancelPrompt = "Cancel this real product input";
      const cancelTurn = await sendMessage(
        page,
        JSON.stringify({
          tool: "request_input",
          args: { prompt: cancelPrompt },
        }),
        created,
      );
      expect(cancelTurn.body.max_steps).toBe(1);
      await expect(
        page.locator(".input-card").filter({ hasText: cancelPrompt }),
      ).toBeVisible();
      await cancelCurrentRun(page);

      await page.goto("/settings/providers");
      await expect(
        page.locator(".profile-row").filter({ hasText: "Real API fake raw" }),
      ).toBeVisible();
      await page.goto("/settings/memory");
      await expect(page.getByRole("heading", { name: "Memory" })).toBeVisible();
      await expect(
        page.getByText("No durable memory topics are available."),
      ).toBeVisible();
      await page.goto("/settings/keyboard");
      await expect(
        page.getByRole("heading", { name: "Keyboard shortcuts" }),
      ).toBeVisible();
      await page.goto("/settings/about");
      await expect(page.getByRole("heading", { name: "Resume health" })).toBeVisible();
      await page.goto("/settings/general");
      await page.getByRole("button", { name: "Dark", exact: true }).click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
      await page.reload();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

      await page.goto(routeA);
      await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
      await page.keyboard.press("/");
      await expect(page.getByRole("textbox", { name: "Message" })).toBeFocused();
    } catch (error) {
      primaryError = error;
    } finally {
      await finalizeProductTest(
        page,
        request,
        created,
        baseline,
        workspaceRoot,
        primaryError,
      );
    }
  });
});

test.describe("optional real API advanced workbench smoke", () => {
  test.skip(
    !realApiEnabled || !advancedWorkbenchEnabled,
    "set ROVE_REAL_API_E2E=1 and ROVE_REAL_API_WORKBENCH_SMOKE=1 to run",
  );

  test("keeps the bounded advanced direct-run surface available", async ({ page }) => {
    await page.goto("/dev/workbench");
    const task = `advanced workbench smoke ${Date.now()}`;
    await page.getByLabel("Task").fill(task);
    await page.getByLabel("Model").fill("fake");
    await page.getByLabel("Steps").fill("4");
    await page.getByRole("button", { name: "Run" }).click();

    await expect(
      page.getByLabel("Run summary").getByText("Run completed").first(),
    ).toBeVisible({ timeout: 20_000 });
    await expect(
      page.locator(".message-stream").getByText(`fake response: ${task}`),
    ).toBeVisible();
  });
});

function emptyCreatedRecords(): CreatedProductRecords {
  return { workspaceIds: [], sessionIds: [], profileIds: [], jobIds: [] };
}

async function seedLegacyState(page: Page, workspaceRoot: string) {
  const values: Record<string, string> = {
    [M1_BROWSER_STORAGE_KEYS.workspaces]: JSON.stringify([
      {
        id: "live-legacy-workspace",
        rootPath: workspaceRoot,
        kind: "folder",
        displayName: "Live imported workspace",
        pinned: false,
        lastOpenedAt: "2026-07-27T00:00:00.000Z",
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.sessions]: JSON.stringify([
      {
        id: "live-legacy-session",
        workspaceId: "live-legacy-workspace",
        title: "Live imported session",
        createdAt: "2026-07-27T00:00:00.000Z",
        updatedAt: "2026-07-27T00:00:00.000Z",
        status: "idle",
        hasDurableTurn: false,
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.active]: JSON.stringify({
      workspaceId: "live-legacy-workspace",
      sessionId: "live-legacy-session",
    }),
  };
  await page.addInitScript((entries) => {
    for (const [key, value] of Object.entries(entries)) {
      window.localStorage.setItem(key, value);
    }
  }, values);
}

async function openWorkspace(
  page: Page,
  workspaceRoot: string,
  created: CreatedProductRecords,
): Promise<{ workspaceId: string; sessionId: string }> {
  const workspaceResponsePromise = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname === "/api/product/workspaces" &&
      response.status() === 201,
  );
  const sessionResponsePromise = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname === "/api/product/sessions" &&
      response.status() === 201,
  );
  await page.getByLabel("Absolute path").fill(workspaceRoot);
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();

  const workspaceId = await responseId(await workspaceResponsePromise);
  created.workspaceIds.push(workspaceId);
  const sessionId = await responseId(await sessionResponsePromise);
  created.sessionIds.push(sessionId);
  await expect(page).toHaveURL(productSessionRoute(workspaceId, sessionId));
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
  return { workspaceId, sessionId };
}

async function createSession(
  page: Page,
  workspaceId: string,
  created: CreatedProductRecords,
): Promise<string> {
  const responsePromise = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname === "/api/product/sessions" &&
      response.status() === 201,
  );
  await page.getByRole("button", { name: "New session", exact: true }).click();
  const sessionId = await responseId(await responsePromise);
  created.sessionIds.push(sessionId);
  await expect(page).toHaveURL(productSessionRoute(workspaceId, sessionId));
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
  return sessionId;
}

async function responseId(response: { json(): Promise<unknown> }): Promise<string> {
  const value = (await response.json()) as { id?: unknown };
  if (typeof value.id !== "string" || !value.id) {
    throw new Error("product create response did not include an id");
  }
  return value.id;
}

async function sendMessage(
  page: Page,
  message: string,
  created: CreatedProductRecords,
): Promise<StartedTurn> {
  const responsePromise = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname === "/api/jobs",
  );
  await page.getByRole("textbox", { name: "Message" }).fill(message);
  await page.getByRole("button", { name: "Send" }).click();
  const response = await responsePromise;
  expect(response.ok()).toBe(true);
  const started = (await response.json()) as {
    job_id?: unknown;
    run_id?: unknown;
    resumed_from_run_id?: unknown;
  };
  if (typeof started.job_id !== "string" || typeof started.run_id !== "string") {
    throw new Error("product job response did not include exact job/run ids");
  }
  const resumedFromRunId = started.resumed_from_run_id ?? null;
  if (resumedFromRunId !== null && typeof resumedFromRunId !== "string") {
    throw new Error("product job response contained an invalid resume id");
  }
  created.jobIds.push(started.job_id);
  return {
    body: jobBody(response.request()),
    jobId: started.job_id,
    runId: started.run_id,
    resumedFromRunId,
  };
}

function jobBody(request: Request): Record<string, unknown> {
  const body: unknown = request.postDataJSON();
  if (!body || typeof body !== "object" || Array.isArray(body)) {
    throw new Error("product job request did not contain a JSON object");
  }
  return body as Record<string, unknown>;
}

function expectProductSessionRequest(turn: StartedTurn, sessionId: string) {
  expect(turn.body.product_session_id).toBe(sessionId);
  expect(turn.body).not.toHaveProperty("resume");
}

async function expectTurnCompleted(page: Page, assistantText: string) {
  await expect(
    page.getByLabel("Conversation").getByText(assistantText, { exact: true }),
  ).toBeVisible({ timeout: 30_000 });
  await expectRunCompleted(page);
}

async function expectRunCompleted(page: Page) {
  await expect(
    page.getByLabel("Run inspector").getByText("Run completed", { exact: true }),
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
}

async function cancelCurrentRun(page: Page) {
  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname.endsWith("/cancel") &&
        response.ok(),
    ),
    page.getByRole("button", { name: "Stop run" }).click(),
  ]);
  await expect(
    page.getByLabel("Run inspector").getByText("Run cancelled", {
      exact: true,
    }),
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
}

async function readTranscript(
  request: APIRequestContext,
  sessionId: string,
): Promise<ProductTranscriptSnapshot> {
  const response = await request.get(
    `/api/product/sessions/${encodeURIComponent(sessionId)}/transcript`,
  );
  expect(response.ok()).toBe(true);
  const transcript = (await response.json()) as ProductTranscriptSnapshot;
  expect(transcript.product_session_id).toBe(sessionId);
  return transcript;
}

async function selectFakeRawProfile(
  page: Page,
  created: CreatedProductRecords,
): Promise<string> {
  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await page.getByLabel("Label").fill("Real API fake raw");
  await page.getByLabel("Type").selectOption("fake");
  await expect(page.getByLabel("API base")).toHaveValue("");
  await page.getByLabel("Default model").fill("fake-raw");

  await page.getByRole("button", { name: "Test", exact: true }).click();
  await expect(page.getByText(/Test: pass/iu)).toBeVisible();
  await page.getByRole("button", { name: "List models", exact: true }).click();
  await expect(page.getByText(/Models \(\d+\):/u)).toBeVisible();

  const profileResponsePromise = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname ===
        "/api/product/provider-profiles" &&
      response.status() === 201,
  );
  const preferencesResponsePromise = page.waitForResponse((response) => {
    if (
      response.request().method() !== "PUT" ||
      new URL(response.url()).pathname !== "/api/product/preferences" ||
      !response.ok()
    ) {
      return false;
    }
    try {
      const body = response.request().postDataJSON() as {
        provider_selection?: { model?: string };
      };
      return body.provider_selection?.model === "fake-raw";
    } catch {
      return false;
    }
  });
  await page.getByRole("button", { name: "Save profile" }).click();
  const profileId = await responseId(await profileResponsePromise);
  created.profileIds.push(profileId);
  await preferencesResponsePromise;
  await expect(
    page.locator(".profile-row").filter({ hasText: "Real API fake raw" }),
  ).toBeVisible();
  return profileId;
}

async function readPreferences(
  request: APIRequestContext,
): Promise<ProductPreferencesSnapshot> {
  const response = await request.get("/api/product/preferences");
  expect(response.ok()).toBe(true);
  return (await response.json()) as ProductPreferencesSnapshot;
}

async function detachActiveProductRoute(
  request: APIRequestContext,
  baseline: ProductPreferencesSnapshot,
) {
  const response = await request.put("/api/product/preferences", {
    data: preferenceUpdate(baseline, baseline, {
      active_workspace_id: null,
      active_session_id: null,
    }),
  });
  expect(response.ok()).toBe(true);
}

function preferenceUpdate(
  current: ProductPreferencesSnapshot,
  desired: ProductPreferencesSnapshot,
  active: {
    active_workspace_id: string | null;
    active_session_id: string | null;
  } = {
    active_workspace_id: desired.active_workspace_id ?? null,
    active_session_id: desired.active_session_id ?? null,
  },
) {
  return {
    schema_version: current.schema_version,
    expected_revision: current.revision,
    theme: desired.theme,
    default_approval_policy: desired.default_approval_policy,
    active_workspace_id: active.active_workspace_id,
    active_session_id: active.active_session_id,
    provider_selection: desired.provider_selection ?? null,
  };
}

async function cleanupOrThrow(
  request: APIRequestContext,
  created: CreatedProductRecords,
  baseline: ProductPreferencesSnapshot,
  workspaceRoot: string,
) {
  await registerProductRecordsForWorkspaceRoot(request, created, workspaceRoot);
  const cleaned = await removeProductRecords(request, created, baseline);
  if (!cleaned) {
    throw new Error(
      `real API E2E cleanup failed; preserved temporary workspace ${workspaceRoot}`,
    );
  }
  await rm(workspaceRoot, { recursive: true, force: true });
}

async function finalizeProductTest(
  page: Page,
  request: APIRequestContext,
  created: CreatedProductRecords,
  baseline: ProductPreferencesSnapshot,
  workspaceRoot: string,
  primaryError: unknown | null,
) {
  const errors: unknown[] = [];
  if (primaryError !== null) {
    errors.push(primaryError);
  }
  try {
    await page.close();
  } catch (error) {
    errors.push(error);
  }
  try {
    await cleanupOrThrow(request, created, baseline, workspaceRoot);
  } catch (error) {
    errors.push(error);
  }

  if (errors.length > 1) {
    throw new AggregateError(
      errors,
      "real API E2E assertions, browser close, or cleanup failed",
    );
  }
  if (errors.length === 1) {
    throw errors[0];
  }
}

async function registerProductRecordsForWorkspaceRoot(
  request: APIRequestContext,
  created: CreatedProductRecords,
  workspaceRoot: string,
): Promise<number> {
  const workspaceResponse = await request.get("/api/product/workspaces");
  if (!workspaceResponse.ok()) {
    throw new Error(
      `could not discover real API E2E workspaces (${workspaceResponse.status()})`,
    );
  }
  const workspaces = (await workspaceResponse.json()) as ProductWorkspacesResponse;
  let resolvedWorkspaceRoot = workspaceRoot;
  try {
    resolvedWorkspaceRoot = await realpath(workspaceRoot);
  } catch {
    // Cleanup still attempts the original path when canonicalization is unavailable.
  }
  const expectedRoot = comparableWorkspaceRoot(resolvedWorkspaceRoot);
  let matchedWorkspaces = 0;
  for (const workspace of workspaces.workspaces) {
    if (comparableWorkspaceRoot(workspace.canonical_root) !== expectedRoot) {
      continue;
    }
    matchedWorkspaces += 1;
    pushUnique(created.workspaceIds, workspace.id);
    const sessionResponse = await request.get(
      `/api/product/sessions?workspace_id=${encodeURIComponent(workspace.id)}`,
    );
    if (!sessionResponse.ok()) {
      throw new Error(
        `could not discover real API E2E sessions (${sessionResponse.status()})`,
      );
    }
    const sessions = (await sessionResponse.json()) as ProductSessionsResponse;
    for (const session of sessions.sessions) {
      pushUnique(created.sessionIds, session.id);
    }
  }
  return matchedWorkspaces;
}

function comparableWorkspaceRoot(value: string): string {
  const normalized = value
    .replace(/\\/gu, "/")
    .replace(/^\/\/\?\//u, "")
    .replace(/\/+$/u, "");
  return process.platform === "win32"
    ? normalized.toLocaleLowerCase("en-US")
    : normalized;
}

function pushUnique(values: string[], value: string) {
  if (!values.includes(value)) {
    values.push(value);
  }
}

async function removeProductRecords(
  request: APIRequestContext,
  created: CreatedProductRecords,
  baseline: ProductPreferencesSnapshot,
): Promise<boolean> {
  try {
    const current = await readPreferences(request);
    const restored = await request.put("/api/product/preferences", {
      data: preferenceUpdate(current, baseline),
    });
    if (!restored.ok()) {
      return false;
    }

    if (!(await cancelCreatedJobs(request, created.jobIds))) {
      return false;
    }

    for (const profileId of [...new Set(created.profileIds)].reverse()) {
      const response = await request.delete(
        `/api/product/provider-profiles/${encodeURIComponent(profileId)}`,
      );
      if (!response.ok()) {
        return false;
      }
    }
    for (const sessionId of [...new Set(created.sessionIds)].reverse()) {
      const response = await request.delete(
        `/api/product/sessions/${encodeURIComponent(sessionId)}`,
      );
      if (!response.ok()) {
        return false;
      }
    }
    for (const workspaceId of [...new Set(created.workspaceIds)].reverse()) {
      const response = await request.delete(
        `/api/product/workspaces/${encodeURIComponent(workspaceId)}`,
      );
      if (!response.ok()) {
        return false;
      }
    }
    return true;
  } catch {
    return false;
  }
}

async function cancelCreatedJobs(
  request: APIRequestContext,
  jobIds: string[],
): Promise<boolean> {
  for (const jobId of [...new Set(jobIds)].reverse()) {
    let stateResponse = await request.get(
      `/api/jobs/${encodeURIComponent(jobId)}/state`,
    );
    if (!stateResponse.ok()) {
      return false;
    }
    let state = (await stateResponse.json()) as { status?: unknown };
    if (state.status !== "running") {
      continue;
    }

    const cancelled = await request.post(
      `/api/jobs/${encodeURIComponent(jobId)}/cancel`,
    );
    if (!cancelled.ok()) {
      return false;
    }

    let reachedTerminalState = false;
    for (let attempt = 0; attempt < 50; attempt += 1) {
      stateResponse = await request.get(
        `/api/jobs/${encodeURIComponent(jobId)}/state`,
      );
      if (!stateResponse.ok()) {
        return false;
      }
      state = (await stateResponse.json()) as { status?: unknown };
      if (state.status !== "running") {
        reachedTerminalState = true;
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    if (!reachedTerminalState) {
      return false;
    }
  }
  return true;
}

function productSessionRoute(workspaceId: string, sessionId: string): string {
  return `/w/${encodeURIComponent(workspaceId)}/s/${encodeURIComponent(sessionId)}`;
}

async function browserValue(page: Page, key: string): Promise<string | null> {
  return page.evaluate((storageKey) => window.localStorage.getItem(storageKey), key);
}
