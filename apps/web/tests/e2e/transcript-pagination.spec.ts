import { expect, test, type Locator, type Page } from "@playwright/test";

import { MAX_MOUNTED } from "../../chat/transcript-window";
import { STREAM_EVENT_CONTRACT_VERSION } from "../../lib/rove-types";
import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  longCompletedTranscript,
} from "./product-api-mock";

/**
 * R1 transcript cursor pagination.
 *
 * The loaded layer is a chain of bounded server pages: the newest page on
 * restore, then one older page per user action. These cases pin the two things
 * a paging implementation gets wrong — a page boundary that loses or repeats a
 * run, and a page arrival that moves what the reader is looking at.
 */

const RUN_COUNT = 300;
/** The client's page ceiling, which is also the API's `limit_runs` maximum. */
const PAGE_RUNS = 64;

async function openLongSession(
  page: Page,
  onTranscriptRequest?: (search: string) => void,
) {
  await page.setViewportSize({ width: 1280, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = longCompletedTranscript(workspace, session, RUN_COUNT);
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  if (onTranscriptRequest) {
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname.endsWith(`/sessions/${session.id}/transcript`)) {
        onTranscriptRequest(url.search);
      }
    });
  }
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const scroller = page.getByLabel("Conversation");
  await expect(scroller.locator("[data-run-ordinal]").first()).toBeVisible();
  return { scroller, workspace, session };
}

function transcriptFetch(page: Page, sessionId: string, query: string) {
  return page.evaluate(
    async ({ id, search }) => {
      const response = await fetch(
        `/api/product/sessions/${id}/transcript${search}`,
      );
      return {
        status: response.status,
        body: (await response.json()) as {
          code?: string;
          has_more?: boolean;
          next_before_ordinal?: number;
          segments?: Array<{ binding: { ordinal: number } }>;
        },
      };
    },
    { id: sessionId, search: query },
  );
}

/** The rendered run ordinals, in document order. */
async function renderedOrdinals(scroller: Locator): Promise<number[]> {
  return scroller
    .locator("[data-run-ordinal]")
    .evaluateAll((nodes) =>
      nodes.map((node) => Number((node as HTMLElement).dataset.runOrdinal)),
    );
}

/**
 * One step of "older history": mount everything already loaded, and only then
 * ask the server for the next page.
 */
async function olderHistoryStep(page: Page): Promise<"grow" | "server" | "done"> {
  const grow = page.locator("button.load-older-turns");
  if (await grow.count()) {
    await grow.click();
    return "grow";
  }
  const server = page.locator("button.load-older-history");
  if (await server.count()) {
    await server.click();
    // A page arrived exactly when the loaded layer holds runs the mounted
    // window has not shown yet, which is what the grow control means.
    await expect(grow).toBeVisible();
    return "server";
  }
  return "done";
}

/**
 * Bring the older-history control back into view, if the session still has one.
 *
 * The control sits above the loaded window, so it leaves the viewport once the
 * reader scrolls into the window. A real wheel gesture both reaches it and keeps
 * follow released; the runner's own scroll-into-view would instead move the
 * reader after the anchored offset was measured.
 */
async function revealOlderControl(
  page: Page,
  scroller: Locator,
): Promise<boolean> {
  const control = page
    .locator("button.load-older-turns, button.load-older-history")
    .first();
  if ((await control.count()) === 0) {
    return false;
  }
  await scroller.hover();
  await page.mouse.wheel(0, -200_000);
  await expect(control).toBeVisible();
  return true;
}

/** The ordinal of the first run below the scroller's top edge. */
async function topmostVisibleOrdinal(scroller: Locator): Promise<number | null> {
  return scroller.evaluate((node) => {
    const top = node.getBoundingClientRect().top + 1;
    const section = [
      ...node.querySelectorAll<HTMLElement>("[data-run-ordinal]"),
    ].find((candidate) => candidate.getBoundingClientRect().top >= top);
    return section ? Number(section.dataset.runOrdinal) : null;
  });
}

test("the transcript endpoint enforces the cursor contract", async ({ page }) => {
  const { session } = await openLongSession(page);

  const legacy = await transcriptFetch(page, session.id, "");
  expect(legacy.status).toBe(200);
  expect(legacy.body.has_more).toBeUndefined();
  expect(legacy.body.next_before_ordinal).toBeUndefined();
  expect(legacy.body.segments).toHaveLength(RUN_COUNT);

  for (const invalid of [
    "?before_ordinal=0",
    "?limit_runs=0",
    "?limit_runs=65",
    "?before_ordinal=abc",
  ]) {
    const response = await transcriptFetch(page, session.id, invalid);
    expect(response.status, invalid).toBe(400);
    expect(response.body.code, invalid).toBe("product_invalid_input");
  }

  const outOfRange = await transcriptFetch(
    page,
    session.id,
    `?before_ordinal=${RUN_COUNT + 1}`,
  );
  expect(outOfRange.status).toBe(200);
  expect(outOfRange.body.segments).toEqual([]);
  expect(outOfRange.body.has_more).toBe(false);
  expect(outOfRange.body.next_before_ordinal).toBeUndefined();

  const newest = await transcriptFetch(
    page,
    session.id,
    `?limit_runs=${PAGE_RUNS}`,
  );
  expect(newest.body.has_more).toBe(true);
  expect(newest.body.next_before_ordinal).toBe(RUN_COUNT - PAGE_RUNS + 1);
  const newestOrdinals = newest.body.segments!.map((segment) => segment.binding.ordinal);
  expect(newestOrdinals).toHaveLength(PAGE_RUNS);
  // Whole runs, ascending inside the page, newest page first.
  expect(newestOrdinals).toEqual(
    Array.from({ length: PAGE_RUNS }, (_value, index) => RUN_COUNT - PAGE_RUNS + 1 + index),
  );

  const oldest = await transcriptFetch(page, session.id, "?before_ordinal=45");
  expect(oldest.body.has_more).toBe(false);
  expect(oldest.body.next_before_ordinal).toBeUndefined();
  expect(oldest.body.segments!.map((segment) => segment.binding.ordinal)).toEqual(
    Array.from({ length: 44 }, (_value, index) => index + 1),
  );
});

test("load older history walks every cursor page without losing a run", async ({
  page,
}) => {
  test.setTimeout(180_000);
  const requested: string[] = [];
  const { scroller } = await openLongSession(page, (search) => {
    requested.push(search);
  });

  // The reader leaves the newest turn before any older page is fetched.
  await scroller.hover();
  await page.mouse.wheel(0, -700);
  const returnToLatest = page.locator("button.return-to-latest");
  await expect(returnToLatest).toBeVisible();

  let sawServerPage = false;
  let peakRendered = 0;
  for (let step = 0; step < 80; step += 1) {
    if (!(await revealOlderControl(page, scroller))) {
      break;
    }
    // Remember where the reader is by the ordinal of the topmost visible run.
    const anchorOrdinal = await topmostVisibleOrdinal(scroller);
    const offsetBefore =
      anchorOrdinal === null ? null : await anchorOffset(scroller, anchorOrdinal);

    const outcome = await olderHistoryStep(page);
    // Mounting older runs and fetching an older page both add content above the
    // reader: the anchored run must stay at the same viewport offset, and the
    // correction must not pull the viewport back to the newest turn.
    if (outcome === "server") {
      sawServerPage = true;
    }
    if (anchorOrdinal !== null && offsetBefore !== null) {
      await expect
        .poll(async () => {
          const after = await anchorOffset(scroller, anchorOrdinal);
          return after === null ? null : Math.abs(after - offsetBefore);
        })
        .toBeLessThanOrEqual(1);
    }
    // Bounded lazy rendering: the loaded layer grows toward the whole session
    // while the DOM keeps holding one window of runs.
    const rendered = await scroller.locator("[data-run-ordinal]").count();
    peakRendered = Math.max(peakRendered, rendered);
    expect(rendered).toBeLessThanOrEqual(MAX_MOUNTED);
  }

  expect(sawServerPage).toBe(true);
  // The restore asked for one bounded cursor page, which is the only reason an
  // older page can be requested at all, and every request declared the
  // canonical-event contract this bundle can decode.
  expect(requested.filter((search) => !search.includes("before_ordinal"))).toEqual([
    `?event_contract=${STREAM_EVENT_CONTRACT_VERSION}&limit_runs=${PAGE_RUNS}`,
  ]);
  // Every older request used the cursor of the smallest loaded ordinal, and
  // every page was bounded.
  const cursors = requested
    .filter((search) => search.includes("before_ordinal"))
    .map((search) => {
      const match = /before_ordinal=(\d+)/u.exec(search);
      return match ? Number(match[1]) : null;
    });
  expect(cursors).toEqual([237, 173, 109, 45]);
  for (const search of requested) {
    expect(search).toContain(`event_contract=${STREAM_EVENT_CONTRACT_VERSION}`);
    expect(search).toContain(`limit_runs=${PAGE_RUNS}`);
  }

  // The server page action is gone once `has_more` is false; the mount window
  // is fully drained, so no local control is left either.
  await expect(page.locator("button.load-older-history")).toHaveCount(0);
  await expect(page.locator("button.load-older-turns")).toHaveCount(0);

  // Bounded, not "growing with the loaded layer": the walk reached the oldest
  // run of the session, so the DOM holds the oldest MAX_MOUNTED runs — one
  // contiguous, unique, ascending window — and the newest run the server
  // returned has been released. Every run still passed through the window, which
  // is what the cursor chain above and the oldest ordinal below pin.
  const ordinals = await renderedOrdinals(scroller);
  expect(peakRendered).toBe(MAX_MOUNTED);
  expect(ordinals).toEqual(
    Array.from({ length: MAX_MOUNTED }, (_value, index) => index + 1),
  );
  await expect(scroller.locator(`[data-run-ordinal="${RUN_COUNT}"]`)).toHaveCount(0);
  await expect(scroller).toContainText("Restored question 1");
  // The reader never asked to return to the newest turn: the page arrivals did
  // not scroll there.
  await expect(returnToLatest).toBeVisible();
});

test("the window releases the newest runs and returning to the latest brings them back", async ({
  page,
}) => {
  test.setTimeout(120_000);
  const { scroller } = await openLongSession(page);

  // The reader leaves the newest turn before the first older page is fetched.
  await scroller.hover();
  await page.mouse.wheel(0, -700);
  const returnToLatest = page.locator("button.return-to-latest");
  await expect(returnToLatest).toBeVisible();

  // Four growth steps reveal the whole first page (64 runs). The page that
  // follows makes the next growth step the one that has to release runs.
  for (let step = 0; step < 4; step += 1) {
    expect(await revealOlderControl(page, scroller)).toBe(true);
    expect(await olderHistoryStep(page)).toBe("grow");
  }
  expect(await revealOlderControl(page, scroller)).toBe(true);
  expect(await olderHistoryStep(page)).toBe("server");
  expect(await revealOlderControl(page, scroller)).toBe(true);
  expect(await olderHistoryStep(page)).toBe("grow");

  // Revealing past the cap released the newest runs, so the live tail is not on
  // the DOM while the reader is deep in older history.
  const walked = await renderedOrdinals(scroller);
  expect(walked).toHaveLength(MAX_MOUNTED);
  expect(await scroller.locator(`[data-run-ordinal="${RUN_COUNT}"]`).count()).toBe(0);

  // "Return to latest" means the newest run, so the window comes home and the
  // newest turn is on screen again instead of the middle of older history.
  await returnToLatest.click();
  await expect(scroller.locator(`[data-run-ordinal="${RUN_COUNT}"]`)).toHaveCount(1);
  const returned = await renderedOrdinals(scroller);
  expect(returned).toHaveLength(MAX_MOUNTED);
  expect(returned[returned.length - 1]).toBe(RUN_COUNT);
  await expect(
    page.getByText(`Restored answer ${RUN_COUNT}`, { exact: true }),
  ).toBeVisible();
  await expect(page.locator("button.return-to-latest")).toHaveCount(0);
});

async function anchorOffset(
  scroller: Locator,
  ordinal: number,
): Promise<number | null> {
  return scroller.evaluate((node, value) => {
    const section = node.querySelector<HTMLElement>(`[data-run-ordinal="${value}"]`);
    return section
      ? section.getBoundingClientRect().top - node.getBoundingClientRect().top
      : null;
  }, ordinal);
}
