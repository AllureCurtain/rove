import { defineConfig, devices } from "@playwright/test";

const webPort = process.env.ROVE_WEB_PORT || "13043";
const baseURL = process.env.PLAYWRIGHT_BASE_URL || `http://127.0.0.1:${webPort}`;
const reuseExistingServer =
  process.env.PLAYWRIGHT_REUSE_EXISTING_SERVER === "1" ||
  Boolean(process.env.PLAYWRIGHT_BASE_URL);

/**
 * Production profile: serve the suite from `next start` instead of `next dev`.
 *
 * Next only prefetches route payloads in a production build, and the `/dev/*` preview
 * routes are prerendered behind `ROVE_ENABLE_DEV_ROUTES` (`app/dev/layout.tsx`). That gate
 * has to be present while the profiled build runs and while its server serves, so the
 * profile sets it for the spawned server as well. `pnpm test:e2e:prod` builds first.
 */
const productionBuild = process.env.ROVE_E2E_PROD === "1";
if (productionBuild) {
  process.env.ROVE_ENABLE_DEV_ROUTES = "1";
}

/**
 * Worker count. Playwright's default fan-out is right on a normal machine, but the
 * acceptance runner's browser step has to fit the dev server plus one browser per worker
 * in memory; an over-committed machine turns correct assertions into visibility timeouts.
 * `ROVE_E2E_WORKERS=<n>` bounds the fan-out without changing any assertion.
 */
const workers = process.env.ROVE_E2E_WORKERS ? Number(process.env.ROVE_E2E_WORKERS) : undefined;

export default defineConfig({
  testDir: "./tests/e2e",
  workers,
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL,
    trace: "retain-on-failure",
  },
  webServer: {
    command: productionBuild
      ? `pnpm exec next start --port ${webPort}`
      : `pnpm exec next dev --port ${webPort}`,
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
