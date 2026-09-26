import type {
  ApprovalDecision,
  BenchRunDetail,
  BenchTaskResult,
  CreateJobRequest,
  CreateJobResponse,
  JobStateResponse,
  ListBenchRunsResponse,
  ListBenchSuitesResponse,
  ListRunsResponse,
  ProviderModelsRequest,
  ProviderModelsResponse,
  ProviderTestRequest,
  ProviderTestResponse,
  RunReport,
  StartBenchRunRequest,
  StartBenchRunResponse,
} from "./rove-types";
import {
  desktopTransport,
  withDesktopAuthorization,
} from "../platform/desktop-transport";

const API_PREFIX = "/api";

export class RoveApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "RoveApiError";
    this.status = status;
    this.code = code;
  }
}

function apiUrl(path: string): string {
  return `${desktopTransport()?.apiPrefix ?? API_PREFIX}${path}`;
}

function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const desktop = desktopTransport();
  const fetchImpl = withDesktopAuthorization(globalThis.fetch, desktop?.token);
  return init === undefined ? fetchImpl(input) : fetchImpl(input, init);
}

async function parseJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    let code = "http_error";
    let message = `API request failed with status ${response.status}`;
    try {
      const payload: unknown = JSON.parse(text);
      if (
        typeof payload === "object" &&
        payload !== null &&
        "code" in payload &&
        "error" in payload &&
        typeof payload.code === "string" &&
        typeof payload.error === "string"
      ) {
        code = payload.code;
        message = payload.error;
      }
    } catch {
      // Non-JSON error bodies are not safe UI diagnostics.
    }
    throw new RoveApiError(response.status, code, message);
  }
  return (await response.json()) as T;
}

export async function createJob(
  payload: CreateJobRequest,
): Promise<CreateJobResponse> {
  const response = await apiFetch(apiUrl("/jobs"), {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  return parseJson<CreateJobResponse>(response);
}

export async function testProvider(
  payload: ProviderTestRequest,
): Promise<ProviderTestResponse> {
  const response = await apiFetch(apiUrl("/providers/test"), {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  return parseJson<ProviderTestResponse>(response);
}

/** List models available on a provider endpoint (requires base URL + key env). */
export async function listProviderModels(
  payload: ProviderModelsRequest,
): Promise<ProviderModelsResponse> {
  const response = await apiFetch(apiUrl("/providers/models"), {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  return parseJson<ProviderModelsResponse>(response);
}

export async function cancelJob(jobId: string): Promise<JobStateResponse> {
  const response = await apiFetch(apiUrl(`/jobs/${encodeURIComponent(jobId)}/cancel`), {
    method: "POST",
  });
  return parseJson<JobStateResponse>(response);
}

export async function fetchJobState(jobId: string): Promise<JobStateResponse> {
  const response = await apiFetch(apiUrl(`/jobs/${encodeURIComponent(jobId)}/state`));
  return parseJson<JobStateResponse>(response);
}

export async function listRuns(limit = 50): Promise<ListRunsResponse> {
  const response = await apiFetch(apiUrl(`/runs?limit=${encodeURIComponent(String(limit))}`));
  return parseJson<ListRunsResponse>(response);
}

export async function fetchRunReport(runId: string): Promise<RunReport> {
  const response = await apiFetch(apiUrl(`/runs/${encodeURIComponent(runId)}/report`));
  return parseJson<RunReport>(response);
}

export async function submitApproval(
  jobId: string,
  callId: string,
  decision: ApprovalDecision,
): Promise<JobStateResponse> {
  const response = await apiFetch(
    apiUrl(`/jobs/${encodeURIComponent(jobId)}/approvals/${encodeURIComponent(callId)}`),
    {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({ decision }),
    },
  );
  return parseJson<JobStateResponse>(response);
}

export async function submitInput(
  jobId: string,
  inputId: string,
  answer: string,
): Promise<JobStateResponse> {
  const response = await apiFetch(
    apiUrl(`/jobs/${encodeURIComponent(jobId)}/inputs/${encodeURIComponent(inputId)}`),
    {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({ answer }),
    },
  );
  return parseJson<JobStateResponse>(response);
}

export function openJobStream(jobId: string): EventSource {
  return openEventStream(apiUrl(`/jobs/${encodeURIComponent(jobId)}/events`));
}

/**
 * The product directory stream: one long-lived connection carrying every
 * workspace/session/preferences/control fact the server records.
 *
 * It is a change signal, not a transcript of the change: consumers refresh the
 * catalog they already read instead of applying frames, so a frame kind this
 * client does not know about costs nothing beyond the refresh it already owes.
 *
 * The Desktop transport gets the fetch-backed source, which can see a 409 and
 * resynchronize from it. A plain browser keeps the native `EventSource` used by
 * the job stream: it cannot observe a status, so an expired cursor there ends
 * the connection and the catalog poll takes over until the page reloads.
 */
export function openProductEventStream(): EventSource {
  return openEventStream(apiUrl("/product/events"));
}

export { PRODUCT_EVENT_KINDS } from "./rove-types";
export type { ProductEvent, ProductEventKind } from "./rove-types";

function openEventStream(url: string): EventSource {
  const desktop = desktopTransport();
  if (!desktop) {
    return new EventSource(url);
  }
  return new AuthorizedEventSource(url, desktop.token) as unknown as EventSource;
}

// ─── Benchmark API ─────────────────────────────────────────────────────────

export async function listBenchSuites(): Promise<ListBenchSuitesResponse> {
  const response = await apiFetch(apiUrl("/bench/suites"));
  return parseJson<ListBenchSuitesResponse>(response);
}

export async function startBenchRun(
  payload: StartBenchRunRequest,
): Promise<StartBenchRunResponse> {
  const response = await apiFetch(apiUrl("/bench/runs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  return parseJson<StartBenchRunResponse>(response);
}

export async function listBenchRuns(): Promise<ListBenchRunsResponse> {
  const response = await apiFetch(apiUrl("/bench/runs"));
  return parseJson<ListBenchRunsResponse>(response);
}

export async function fetchBenchRun(benchRunId: string): Promise<BenchRunDetail> {
  const response = await apiFetch(
    apiUrl(`/bench/runs/${encodeURIComponent(benchRunId)}`),
  );
  return parseJson<BenchRunDetail>(response);
}

export async function fetchBenchTask(
  benchRunId: string,
  taskName: string,
): Promise<BenchTaskResult> {
  const response = await apiFetch(
    apiUrl(
      `/bench/runs/${encodeURIComponent(benchRunId)}/tasks/${encodeURIComponent(taskName)}`,
    ),
  );
  return parseJson<BenchTaskResult>(response);
}

export function benchEvidenceUrl(benchRunId: string, path: string): string {
  return apiUrl(
    `/bench/runs/${encodeURIComponent(benchRunId)}/evidence/${path}`,
  );
}

/**
 * A stream failure that knows the HTTP status the server answered with, when
 * there was one. The native `EventSource` error event carries no status, so
 * without this a consumer cannot tell "the resume cursor is stale" from "the
 * transport is down".
 */
export class AuthorizedStreamErrorEvent extends Event {
  readonly status: number | null;

  constructor(status: number | null) {
    super("error");
    this.status = status;
  }
}

/**
 * True when a stream error means the resume cursor fell out of the server's
 * retained window (`409 product_events_expired`). The stream has already
 * dropped the cursor, so the caller owes only a catalog refetch: the reconnect
 * that follows carries no `after=` and cannot replay the same 409.
 */
export function isStreamCursorExpired(event: Event): boolean {
  return event instanceof AuthorizedStreamErrorEvent && event.status === 409;
}

class AuthorizedEventSource extends EventTarget {
  onerror: ((event: Event) => void) | null = null;
  private readonly controller = new AbortController();
  private lastEventId = "";
  private closed = false;

  constructor(
    private readonly streamUrl: string,
    private readonly token: string,
  ) {
    super();
    void this.connect();
  }

  close(): void {
    this.closed = true;
    this.controller.abort();
  }

  private async connect(): Promise<void> {
    while (!this.closed) {
      try {
        const url = new URL(this.streamUrl);
        if (this.lastEventId) {
          url.searchParams.set("after", this.lastEventId);
        }
        const response = await withDesktopAuthorization(globalThis.fetch, this.token)(url, {
          headers: { accept: "text/event-stream" },
          cache: "no-store",
          signal: this.controller.signal,
        });
        if (!response.ok || !response.body) {
          // A cursor below the retained window can never be served, so keeping
          // it would turn every reconnect into the same 409. Dropping it makes
          // the next attempt follow from the newest retained fact; the catalog
          // refetch the caller performs alongside this error closes the gap.
          if (response.status === 409) {
            this.lastEventId = "";
          }
          throw new AuthorizedStreamErrorEvent(
            response.ok ? null : response.status,
          );
        }
        // "Connected" and "quiet" are different states: a long-lived directory
        // stream is healthy while it delivers nothing, so the consumer needs
        // the open signal to know it may rely on the stream.
        this.dispatchEvent(new Event("open"));
        await this.consume(response.body);
      } catch (error) {
        if (this.closed || (error instanceof DOMException && error.name === "AbortError")) {
          return;
        }
        const event =
          error instanceof AuthorizedStreamErrorEvent
            ? error
            : new AuthorizedStreamErrorEvent(null);
        this.dispatchEvent(event);
        this.onerror?.(event);
      }
      if (!this.closed) {
        await new Promise((resolve) => setTimeout(resolve, 1_000));
      }
    }
  }

  private async consume(body: ReadableStream<Uint8Array>): Promise<void> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let eventName = "message";
    let data: string[] = [];

    const dispatch = () => {
      if (data.length === 0) {
        eventName = "message";
        return;
      }
      this.dispatchEvent(
        new MessageEvent(eventName, {
          data: data.join("\n"),
          lastEventId: this.lastEventId,
        }),
      );
      eventName = "message";
      data = [];
    };

    while (!this.closed) {
      const { done, value } = await reader.read();
      buffer += decoder.decode(value, { stream: !done });
      let newline = buffer.indexOf("\n");
      while (newline >= 0) {
        const line = buffer.slice(0, newline).replace(/\r$/, "");
        buffer = buffer.slice(newline + 1);
        if (line === "") {
          dispatch();
        } else if (!line.startsWith(":")) {
          const separator = line.indexOf(":");
          const field = separator < 0 ? line : line.slice(0, separator);
          const value = separator < 0 ? "" : line.slice(separator + 1).replace(/^ /, "");
          if (field === "event") eventName = value;
          if (field === "data") data.push(value);
          if (field === "id" && !value.includes("\0")) this.lastEventId = value;
        }
        newline = buffer.indexOf("\n");
      }
      if (done) {
        dispatch();
        return;
      }
    }
  }
}
