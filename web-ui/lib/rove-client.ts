import type {
  CreateJobRequest,
  CreateJobResponse,
  JobStateResponse,
} from "./rove-types";

const API_PREFIX = "/api";

function apiUrl(path: string): string {
  return `${API_PREFIX}${path}`;
}

async function parseJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    throw new Error(await response.text());
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

export function openJobStream(jobId: string): EventSource {
  return new EventSource(apiUrl(`/jobs/${encodeURIComponent(jobId)}/events`));
}
