import { afterEach, describe, expect, it, vi } from "vitest";

import { createJob, submitApproval } from "./rove-client";

describe("rove client", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("posts approval decisions to the job approval endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 4,
        pending_approvals: [],
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await submitApproval("job/1", "call/1", "approve");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/jobs/job%2F1/approvals/call%2F1",
      {
        method: "POST",
        headers: {
          "content-type": "application/json",
        },
        body: JSON.stringify({ decision: "approve" }),
      },
    );
  });

  it("sends approval policy when creating a job", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-1",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await createJob({
      message: "write file",
      model: "fake",
      max_steps: 2,
      approval: "ask",
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/jobs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        message: "write file",
        model: "fake",
        max_steps: 2,
        approval: "ask",
      }),
    });
  });
});
