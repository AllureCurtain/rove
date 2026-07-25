import { expect, test } from "@playwright/test";

const realApiEnabled = process.env.ROVE_REAL_API_E2E === "1";

test.describe("real API workbench integration", () => {
  test.skip(!realApiEnabled, "set ROVE_REAL_API_E2E=1 to run against a real rove-api server");
  test.describe.configure({ mode: "serial" });

  test("creates a plain fake-provider run and shows it in history", async ({ page }) => {
    await page.goto("/");
    const task = `local-full plain run ${Date.now()}`;

    await startRun(page, {
      task,
      model: "fake",
      steps: "4",
    });

    await expectRunCompleted(page);
    await expect(page.locator(".message-stream").getByText(`fake response: ${task}`)).toBeVisible();
    await expect(page.getByLabel("Run details").getByText("done").first()).toBeVisible();
  });

  test("approves a real write_file tool call from the UI", async ({ page }) => {
    await page.goto("/");

    const outputName = `approved-${Date.now()}.txt`;
    await startRun(page, {
      task: JSON.stringify({
        tool: "write_file",
        args: { path: outputName, content: "ok from real-api e2e" },
      }),
      model: "fake-raw",
      steps: "1",
    });

    const details = page.getByLabel("Run details");
    await expect(details.getByText("pending approval").first()).toBeVisible();
    await expect(details.getByText("write_file").first()).toBeVisible();

    await page.getByRole("button", { name: "Approve" }).click();

    await expectRunCompleted(page);
    await expect(details.getByText("wrote").first()).toBeVisible();
    await expect(details.getByText("done").first()).toBeVisible();
  });

  test("answers a request_input tool call from the UI", async ({ page }) => {
    await page.goto("/");

    await startRun(page, {
      task: JSON.stringify({
        tool: "request_input",
        args: { prompt: "Which branch should I use?" },
      }),
      model: "fake-raw",
      steps: "1",
    });

    await expect(page.getByText("Input requested")).toBeVisible();
    const inputCard = page.locator(".input-card").filter({
      hasText: "Which branch should I use?",
    });
    await inputCard.getByRole("textbox").fill("main");
    await inputCard.getByRole("button", { name: "Send" }).click();

    await expectRunCompleted(page);
    await expect(page.getByLabel("Run details").getByText("main").first()).toBeVisible();
  });
});

async function startRun(
  page: import("@playwright/test").Page,
  options: { task: string; model: string; steps: string },
) {
  await page.getByLabel("Task").fill(options.task);
  await page.getByLabel("Model").fill(options.model);
  await page.getByLabel("Steps").fill(options.steps);
  await page.getByRole("button", { name: "Run" }).click();
}

async function expectRunCompleted(page: import("@playwright/test").Page) {
  await expect(
    page.getByLabel("Run summary").getByText("Run completed").first(),
  ).toBeVisible({ timeout: 20_000 });
}
