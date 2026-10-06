import { expect, test, type Page } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  longCompletedTranscript,
} from "./product-api-mock";

/**
 * Main-thread budgets for the two frontend items whose evidence used to be a
 * manual Performance-panel screenshot (design §4.4 F3, §10 F9).
 *
 * The screenshot was never captured, so neither claim had machine-checkable
 * evidence: "streaming a long markdown reply does not produce long tasks" and
 * "a rail collapse no longer animates the grid track" were recorded prose. This
 * file measures both in the browser instead — Chrome's own long-task entries and
 * a `devtools.timeline` trace — and asserts a stated budget rather than a
 * screenshot:
 *
 * - `longtask` entries come from `PerformanceObserver`, whose 50ms threshold is
 *   Chrome's definition of a long task, not this file's.
 * - Layout and style-recalculation counts and durations come from the CDP trace,
 *   which is the same feed the Performance panel draws from.
 *
 * Each case logs what it measured with a `PERF` prefix, so the recorded numbers
 * in the design documents can be compared against a fresh run. The budgets are
 * not the measured values: they are the ceiling that separates "flat" from the
 * failure modes those documents describe, and they are loose enough for a busy
 * machine.
 */

const VIEWPORT = { width: 1280, height: 720 };
const COMPOSER = /输入消息/u;
const COLLAPSE = { name: "收起工作区列表", exact: true } as const;
const EXPAND = { name: "展开工作区列表", exact: true } as const;

interface LongTask {
  start: number;
  duration: number;
}

interface TraceEvent {
  name: string;
  dur: number;
}

interface TraceMeasurements {
  layoutPasses: number;
  layoutMs: number;
  styleRecalculations: number;
  styleMs: number;
  taskCount: number;
  longestTaskMs: number;
  /** Layout milliseconds split into arrival thirds. */
  layoutThirds: [number, number, number];
  /** Style-recalculation milliseconds split into arrival thirds. */
  styleThirds: [number, number, number];
}

function thirds(values: number[]): [number, number, number] {
  const size = Math.max(1, Math.floor(values.length / 3));
  const sum = (slice: number[]) => slice.reduce((total, value) => total + value, 0) / 1_000;
  return [
    sum(values.slice(0, size)),
    sum(values.slice(size, size * 2)),
    sum(values.slice(size * 2)),
  ];
}

/**
 * The CDP surface this file needs, untyped on purpose.
 *
 * Playwright's generated protocol types do not cover the `Tracing` domain in this
 * version (`Tracing.stop` is not a known command and `Tracing.dataCollected` is
 * typed as a bag of strings), so the session is narrowed to the four calls used
 * here instead of casting every call site.
 */
interface RawCdpSession {
  send(method: string, params?: Record<string, unknown>): Promise<unknown>;
  on(event: string, listener: (payload: unknown) => void): void;
  once(event: string, listener: (payload: unknown) => void): void;
  detach(): Promise<void>;
}

/**
 * Start collecting a `devtools.timeline` trace through CDP.
 *
 * Only the three event kinds these budgets are about are kept: `Layout` and
 * `UpdateLayoutTree` are the passes a transition on `grid-template-columns`
 * multiplies, and `RunTask` gives the main thread's task durations, the same
 * measurement a `longtask` entry reports (that API is unavailable to a
 * trace-only reader).
 */
async function startTrace(page: Page) {
  const browser = page.context().browser();
  if (!browser) {
    throw new Error("the trace needs a browser-scoped CDP session");
  }
  // Tracing is a browser-domain command in this Chromium: a page-scoped session
  // answers `'Tracing.stop' wasn't found`.
  const session = (await browser.newBrowserCDPSession()) as unknown as RawCdpSession;
  const events: TraceEvent[] = [];
  session.on("Tracing.dataCollected", (payload) => {
    const collected = (payload as { value?: Array<{ name?: string; dur?: number }> }).value ?? [];
    for (const event of collected) {
      if (
        event.name === "Layout" ||
        event.name === "UpdateLayoutTree" ||
        event.name === "RunTask"
      ) {
        events.push({ name: event.name, dur: event.dur ?? 0 });
      }
    }
  });
  await session.send("Tracing.start", {
    categories: "devtools.timeline",
    transferMode: "ReportEvents",
  });
  return {
    async stop(): Promise<TraceMeasurements> {
      const complete = new Promise<void>((resolve) => {
        session.once("Tracing.tracingComplete", () => resolve());
      });
      await session.send("Tracing.end");
      await complete;
      await session.detach();

      const of = (name: string) => events.filter((event) => event.name === name);
      const sum = (list: TraceEvent[]) =>
        list.reduce((total, event) => total + event.dur, 0) / 1_000;
      const layouts = of("Layout");
      const styles = of("UpdateLayoutTree");
      const tasks = of("RunTask");
      return {
        layoutPasses: layouts.length,
        layoutMs: sum(layouts),
        styleRecalculations: styles.length,
        styleMs: sum(styles),
        taskCount: tasks.length,
        longestTaskMs: tasks.reduce((longest, event) => Math.max(longest, event.dur), 0) / 1_000,
        layoutThirds: thirds(layouts.map((event) => event.dur)),
        styleThirds: thirds(styles.map((event) => event.dur)),
      };
    },
  };
}

/** Chrome's own long-task entries, which exist only for tasks over 50ms. */
async function watchLongTasks(page: Page) {
  await page.evaluate(() => {
    const target = window as unknown as { __roveLongTasks?: LongTask[] };
    target.__roveLongTasks = [];
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        target.__roveLongTasks!.push({
          start: entry.startTime,
          duration: entry.duration,
        });
      }
    });
    observer.observe({ entryTypes: ["longtask"] });
  });
  return {
    async read(): Promise<LongTask[]> {
      // The observer callback runs in its own task: let the last entries arrive
      // before reading them.
      await page.waitForTimeout(250);
      return page.evaluate(
        () => (window as unknown as { __roveLongTasks?: LongTask[] }).__roveLongTasks ?? [],
      );
    },
  };
}

function report(label: string, values: Record<string, number | string>) {
  const rendered = Object.entries(values)
    .map(([key, value]) =>
      typeof value === "number" ? `${key}=${value.toFixed(2)}` : `${key}=${value}`,
    )
    .join(" ");
  // eslint-disable-next-line no-console
  console.log(`PERF ${label} ${rendered}`);
}

/**
 * The v2 shell animates allocated width on collapse (design §4.4: flex-basis,
 * width, opacity and an 8px translate together — never transform-only), so one
 * layout pass per animation frame is the contract, not a regression. What this
 * budget still guards is the per-frame cost staying small over the whole 30-run
 * fixture, and the collapse never occupying the main thread for a long task.
 */
test("a rail collapse stays inside its layout budget", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: longCompletedTranscript(workspace, session, 30) },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  // The transcript has to be laid out for the collapse to have anything to
  // re-lay-out: this is the 30-run fixture the recorded numbers used.
  await expect(page.getByText("Restored answer 30", { exact: true })).toBeVisible();

  const body = page.locator(".product-body");
  const longTasks = await watchLongTasks(page);

  // Each collapse is traced on its own, so the number is comparable with the
  // recorded pre-F9 measurement of one collapse rather than mixing a collapse
  // with the reopen that follows it.
  const collapses = 6;
  const collapseWindows: Array<[number, number]> = [];
  const totals: TraceMeasurements = {
    layoutPasses: 0,
    layoutMs: 0,
    styleRecalculations: 0,
    styleMs: 0,
    taskCount: 0,
    longestTaskMs: 0,
    layoutThirds: [0, 0, 0],
    styleThirds: [0, 0, 0],
  };
  for (let index = 0; index < collapses; index += 1) {
    const trace = await startTrace(page);
    const windowStart = await page.evaluate(() => performance.now());
    await page.getByRole("button", COLLAPSE).click();
    await expect(body).toHaveAttribute("data-nav-collapsed", "true");
    // The rail slides for 240ms; the reopen must not overlap that slide, or the
    // next collapse is measured while two animations are running.
    await page.waitForTimeout(300);
    collapseWindows.push([windowStart, await page.evaluate(() => performance.now())]);
    const measured = await trace.stop();
    totals.layoutPasses += measured.layoutPasses;
    totals.layoutMs += measured.layoutMs;
    totals.styleRecalculations += measured.styleRecalculations;
    totals.styleMs += measured.styleMs;
    totals.taskCount += measured.taskCount;
    totals.longestTaskMs = Math.max(totals.longestTaskMs, measured.longestTaskMs);

    await page.getByRole("button", EXPAND).click();
    await expect(body).toHaveAttribute("data-nav-collapsed", "false");
    await page.waitForTimeout(300);
  }

  // Only tasks that started inside a measured collapse window count — ambient
  // work elsewhere in the test (route compiles, hydration) is not the collapse's
  // cost, but it shares the same `longtask` observer.
  const tasks = await longTasks.read();
  const longestLongTask = tasks
    .filter((task) =>
      collapseWindows.some(([start, end]) => task.start >= start - 50 && task.start <= end),
    )
    .reduce((longest, task) => Math.max(longest, task.duration), 0);
  const perCollapse = {
    layoutPasses: totals.layoutPasses / collapses,
    layoutMs: totals.layoutMs / collapses,
    styleRecalculations: totals.styleRecalculations / collapses,
    styleMs: totals.styleMs / collapses,
    longestTaskMs: totals.longestTaskMs,
    longestLongTaskMs: longestLongTask,
  };
  report("rail-collapse per-collapse", perCollapse);

  // Measured on the allocated-width collapse (same viewport, 30-run fixture):
  // ~18 layout passes, ~8ms layout, ~37 style recalculations, ~20ms style per
  // collapse — roughly one pass per animation frame, each pass cheap. The budget
  // keeps headroom for slower machines while still failing if a single pass gets
  // expensive or the count balloons past what a 240ms transition can draw.
  expect(perCollapse.layoutPasses).toBeLessThanOrEqual(40);
  expect(perCollapse.layoutMs).toBeLessThanOrEqual(30);
  expect(perCollapse.styleRecalculations).toBeLessThanOrEqual(80);
  expect(perCollapse.styleMs).toBeLessThanOrEqual(60);
  // A collapse is a state change plus a cheap per-frame layout; no single frame
  // may occupy the main thread for a long task. The ceiling sits well above the
  // measured cost (no collapse-owned task at all in quiet runs) but below the
  // point where a transition could monopolise the main thread — headroom also
  // absorbs ambient `longtask` entries that share the measured window when the
  // suite runs under load.
  expect(longestLongTask).toBeLessThanOrEqual(150);
});

/**
 * F3 (design §4): the markdown segments in front of a growing tail are memoized
 * by their text, so appending a delta only re-parses the tail. The deterministic
 * half of that contract — the parse count does not grow with the delta count — is
 * a vitest case; what it cannot show is the consequence on the main thread while
 * a long reply streams in, which is what the missing Performance-panel screenshot
 * was for.
 *
 * The stream is delivered through the app's own `EventSource` seam, because the
 * product API mock answers an events request with every frame at once and so
 * cannot emulate a reply arriving delta by delta. The fake below hands the shell
 * named SSE messages on a timer, which is the same path a real stream takes:
 * `api/run-controller.ts` parses `MessageEvent.data` and `lastEventId` and
 * dispatches one reducer event per frame.
 */
test("streaming a long markdown reply keeps its per-delta work flat", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);

  const deltas = 240;
  const prose =
    "The runtime keeps canonical events as the single lifecycle contract, so every consumer reads the same facts and no interface grows a private event loop. ";
  const frames: Array<{ seq: number; event: Record<string, unknown> }> = [];
  let accumulated = "";
  // The job's stored snapshot already occupies seq 1 (`waiting_model` seeds a
  // `run_started` there), and `applyJobState` marks those seqs as seen before
  // the stream attaches — a scripted frame reusing seq 1 would be deduped.
  let seq = 2;
  for (let index = 0; index < deltas; index += 1) {
    let delta = `Paragraph ${index}. ${prose}`;
    if (index % 20 === 19) {
      // A closed fence, so the segmenter gets a real boundary to memoize behind.
      delta += "\n```rust\nfn bounded(value: usize) -> usize { value.min(64) }\n```\n";
    }
    if (index === deltas - 1) {
      delta += "\nSTREAM-END-MARKER\n";
    }
    accumulated += delta;
    frames.push({ seq, event: { type: "llm_chunk", delta } });
    seq += 1;
  }
  frames.push({
    seq,
    event: {
      type: "llm_message",
      full: accumulated,
      usage: { prompt_tokens: 1_024, completion_tokens: 4_096, total_tokens: 5_120 },
    },
  });
  // No terminal `run_completed`: the scripted stream bypasses the mock server,
  // so terminal reconciliation would refetch the canned transcript and wipe the
  // fake reply a few seconds later — a fixture race, not product behaviour. The
  // run staying live is faithful to `waiting_model`, which never completes.

  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    mode: "waiting_model",
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.addInitScript(
    ({ scriptedFrames }: { scriptedFrames: Array<{ seq: number; event: Record<string, unknown> }> }) => {
      const RealEventSource = window.EventSource;
      const target = window as unknown as {
        __roveScriptedStream?: {
          frames: Array<{ seq: number; event: Record<string, unknown> }>;
        };
      };
      const isJobStream = (url: string) =>
        /\/api\/jobs\/[^/]+\/events$/u.test(url);

      // One class for both roles: the scripted job stream, and a thin forwarder
      // for every other stream (the directory feed), so nothing else in the shell
      // notices the substitution.
      class SubstitutedEventSource {
        onerror: ((event: Event) => void) | null = null;
        onopen: ((event: Event) => void) | null = null;
        onmessage: ((event: MessageEvent) => void) | null = null;
        private readonly real: EventSource | null = null;
        private readonly listeners = new Map<string, Set<EventListener>>();
        private closed = false;

        constructor(readonly url: string) {
          const script = target.__roveScriptedStream;
          if (script && isJobStream(url)) {
            void this.play(script);
            return;
          }
          const real = new RealEventSource(url);
          this.real = real;
          real.onerror = (event) => this.onerror?.(event);
          real.onopen = (event) => this.onopen?.(event);
          real.onmessage = (event) => this.onmessage?.(event);
        }

        private async play(script: NonNullable<typeof target.__roveScriptedStream>) {
          for (const frame of script.frames) {
            if (this.closed) {
              return;
            }
            // A frame every 12ms: fast enough to be a stream, slow enough that
            // React commits each delta as its own render.
            await new Promise((resolve) => setTimeout(resolve, 12));
            const type = String(frame.event.type);
            const message = new MessageEvent(type, {
              data: JSON.stringify(frame.event),
              lastEventId: String(frame.seq),
            });
            for (const listener of this.listeners.get(type) ?? []) {
              listener(message);
            }
          }
        }

        addEventListener(type: string, listener: EventListener) {
          if (this.real) {
            this.real.addEventListener(type, listener);
            return;
          }
          const set = this.listeners.get(type) ?? new Set<EventListener>();
          set.add(listener);
          this.listeners.set(type, set);
        }

        removeEventListener(type: string, listener: EventListener) {
          if (this.real) {
            this.real.removeEventListener(type, listener);
            return;
          }
          this.listeners.get(type)?.delete(listener);
        }

        close() {
          this.closed = true;
          this.real?.close();
        }
      }

      target.__roveScriptedStream = { frames: scriptedFrames };
      (window as unknown as { EventSource: unknown }).EventSource = SubstitutedEventSource;
    },
    { scriptedFrames: frames },
  );

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const composer = page.getByRole("textbox", { name: COMPOSER });
  await expect(composer).toBeEnabled();
  await composer.fill("stream a long answer");
  await composer.press("Control+Enter");
  await expect(page.getByRole("button", { name: "停止", exact: true })).toBeVisible();

  const longTasks = await watchLongTasks(page);
  const trace = await startTrace(page);

  // The stream is over when its last delta is on screen; the transcript follows
  // the tail while it grows, which is the work being measured.
  const transcript = page.getByTestId("conversation-log");
  await expect(page.getByText("STREAM-END-MARKER", { exact: false })).toBeVisible({
    timeout: 60_000,
  });
  // Every delta arrived and the whole reply is in the DOM: a stream that stopped
  // early would measure less work and pass the budgets below for the wrong
  // reason. These are DOM (textContent) assertions — `content-visibility:auto`
  // legitimately skips rendering the reply's top while the tail is pinned to the
  // viewport — and they run before `trace.stop()` so the trace window covers the
  // stream itself rather than the post-run idle.
  await expect(transcript).toContainText("Paragraph 0.");
  await expect(transcript).toContainText(`Paragraph ${deltas - 1}.`);
  await page.waitForTimeout(400);

  const measurements = await trace.stop();
  const tasks = await longTasks.read();
  const longestLongTask = tasks.reduce((longest, task) => Math.max(longest, task.duration), 0);
  const totalLongTaskMs = tasks.reduce((total, task) => total + task.duration, 0);
  report(`stream ${deltas} deltas`, {
    layoutPasses: measurements.layoutPasses,
    layoutMs: measurements.layoutMs,
    styleRecalculations: measurements.styleRecalculations,
    styleMs: measurements.styleMs,
    tasks: measurements.taskCount,
    longestTaskMs: measurements.longestTaskMs,
    longestLongTaskMs: longestLongTask,
    totalLongTaskMs,
    layoutThirds: measurements.layoutThirds.map((value) => value.toFixed(1)).join("/"),
    styleThirds: measurements.styleThirds.map((value) => value.toFixed(1)).join("/"),
  });

  // The budget: no single delta may occupy the main thread for a long task, and
  // the stream as a whole must not pin it.
  expect(longestLongTask).toBeLessThanOrEqual(200);
  expect(measurements.longestTaskMs).toBeLessThanOrEqual(200);
  expect(totalLongTaskMs).toBeLessThanOrEqual(1_500);
  // Flat per-delta work: with every closed segment memoized, the end of the
  // stream costs no more than its start. Style recalculation fires at least once
  // per commit, so ordering those passes by arrival orders them by delta and no
  // clock alignment between the trace and the page is needed. Re-parsing the whole
  // accumulated reply per delta (the behaviour F3 removed) grows with the text, so
  // the last third would be many times the first.
  expect(measurements.styleRecalculations).toBeGreaterThan(deltas / 2);
  const [firstStyle, , lastStyle] = measurements.styleThirds;
  expect(lastStyle).toBeLessThanOrEqual(3 * firstStyle + 50);
  const [firstLayout, , lastLayout] = measurements.layoutThirds;
  expect(lastLayout).toBeLessThanOrEqual(3 * firstLayout + 50);
});
