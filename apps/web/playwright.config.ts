import { defineConfig, devices } from "@playwright/test";

const webPort = process.env.ROVE_WEB_PORT || "13043";
const baseURL = process.env.PLAYWRIGHT_BASE_URL || `http://127.0.0.1:${webPort}`;
const reuseExistingServer =
  process.env.PLAYWRIGHT_REUSE_EXISTING_SERVER === "1" ||
  Boolean(process.env.PLAYWRIGHT_BASE_URL);

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL,
    trace: "retain-on-failure",
  },
  webServer: {
    command: `pnpm exec next dev --port ${webPort}`,
    url: baseURL,
    reuseExistingServer,
    timeout: 120_000,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
