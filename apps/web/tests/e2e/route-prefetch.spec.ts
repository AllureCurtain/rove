import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Idle route prefetching, ported from open-vetta's `useIdleRoutePrefetch`.
 *
 * Next only prefetches route payloads in a production build, so these cases are
 * meaningful against `next start`:
 *
 *   pnpm build
 *   ROVE_API_BASE=... pnpm exec next start --port 3002
 *   ROVE_E2E_PROD=1 PLAYWRIGHT_BASE_URL=http://localhost:3002 \
 *     pnpm exec playwright test tests/e2e/route-prefetch.spec.ts
 *
 * Against `next dev` no request is issued, so a dev run could only assert a
 * false negative. The policy itself (bounded targets, the reference's idle
 * timeout and timer fallback) is covered by `shell/route-prefetch.test.ts`, and
 * the production behavior behind this gate is recorded in the plan document.
 */
const PRODUCTION_SERVER = process.env.ROVE_E2E_PROD === "1";

test.skip(
  !PRODUCTION_SERVER,
  "route prefetching is a production-only behaviour in Next; see the file comment",
);

async function openSession(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Prefetch question", "Prefetch answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Prefetch answer", { exact: true })).toBeVisible();
  return { workspace, session };
}

/** A prefetch, not an incidental load: these carry the RSC prefetch marker. */
function prefetches(requests: string[], fragment: string): string[] {
  return requests.filter(
    (url) => url.includes(fragment) && url.includes("_rsc="),
  );
}

test("the shell warms the routes one click away", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));

  const { workspace } = await openSession(page);

  await expect
    .poll(() => prefetches(requests, "/settings/providers").length, {
      message: "the settings section the header gear opens is warmed",
      timeout: 15_000,
    })
    .toBeGreaterThan(0);
  expect(
    prefetches(requests, "/settings/general").length,
    "the other settings entry point is warmed too",
  ).toBeGreaterThan(0);
  // The workspace target only exists once the catalog has loaded, so it lands in
  // a later idle callback than the settings pair: poll it rather than assuming
  // the first callback covered it.
  await expect
    .poll(() => prefetches(requests, `/w/${workspace.id}`).length, {
      message: "the active workspace is warmed as the way back",
      timeout: 15_000,
    })
    .toBeGreaterThan(0);
});

test("a browser without requestIdleCallback still warms the routes", async ({
  page,
}) => {
  // The reference keeps a timer fallback for engines without the API.
  await page.addInitScript(() => {
    // @ts-expect-error - deliberately removed for this case.
    delete window.requestIdleCallback;
  });
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  await openSession(page);

  await expect
    .poll(() => prefetches(requests, "/settings/").length, {
      message: "the timer fallback still prefetches",
      timeout: 15_000,
    })
    .toBeGreaterThan(0);
});

