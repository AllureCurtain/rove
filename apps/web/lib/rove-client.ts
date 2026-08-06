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
  return `${API_PREFIX}${path}`;
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
  const response = await fetch(apiUrl("/jobs"), {
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
  const response = await fetch(apiUrl("/providers/test"), {
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
  const response = await fetch(apiUrl("/providers/models"), {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  return parseJson<ProviderModelsResponse>(response);
}

export async function cancelJob(jobId: string): Promise<JobStateResponse> {
  const response = await fetch(apiUrl(`/jobs/${encodeURIComponent(jobId)}/cancel`), {
    method: "POST",
  });
  return parseJson<JobStateResponse>(response);
}

export async function fetchJobState(jobId: string): Promise<JobStateResponse> {
  const response = await fetch(apiUrl(`/jobs/${encodeURIComponent(jobId)}/state`));
  return parseJson<JobStateResponse>(response);
}

export async function listRuns(limit = 50): Promise<ListRunsResponse> {
  const response = await fetch(apiUrl(`/runs?limit=${encodeURIComponent(String(limit))}`));
  return parseJson<ListRunsResponse>(response);
}

export async function fetchRunReport(runId: string): Promise<RunReport> {
  const response = await fetch(apiUrl(`/runs/${encodeURIComponent(runId)}/report`));
  return parseJson<RunReport>(response);
}

export async function submitApproval(
  jobId: string,
  callId: string,
  decision: ApprovalDecision,
): Promise<JobStateResponse> {
  const response = await fetch(
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
  const response = await fetch(
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
  return new EventSource(apiUrl(`/jobs/${encodeURIComponent(jobId)}/events`));
}

// ─── Benchmark API ─────────────────────────────────────────────────────────

export async function listBenchSuites(): Promise<ListBenchSuitesResponse> {
  const response = await fetch(apiUrl("/bench/suites"));
  return parseJson<ListBenchSuitesResponse>(response);
}

export async function startBenchRun(
  payload: StartBenchRunRequest,
): Promise<StartBenchRunResponse> {
  const response = await fetch(apiUrl("/bench/runs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  return parseJson<StartBenchRunResponse>(response);
}

export async function listBenchRuns(): Promise<ListBenchRunsResponse> {
  const response = await fetch(apiUrl("/bench/runs"));
  return parseJson<ListBenchRunsResponse>(response);
}

export async function fetchBenchRun(benchRunId: string): Promise<BenchRunDetail> {
  const response = await fetch(
    apiUrl(`/bench/runs/${encodeURIComponent(benchRunId)}`),
  );
  return parseJson<BenchRunDetail>(response);
}

export async function fetchBenchTask(
  benchRunId: string,
  taskName: string,
): Promise<BenchTaskResult> {
  const response = await fetch(
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
