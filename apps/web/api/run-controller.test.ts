import { beforeEach, describe, expect, it, vi } from "vitest";

const client = vi.hoisted(() => ({
  cancelJob: vi.fn(),
  createJob: vi.fn(),
  fetchJobState: vi.fn(),
  listProviderModels: vi.fn(),
  openJobStream: vi.fn(),
  submitApproval: vi.fn(),
  submitInput: vi.fn(),
  testProvider: vi.fn(),
}));

vi.mock("../lib/rove-client", () => client);

import {
  createRunController,
  RunControllerInactiveError,
} from "./run-controller";

class FakeEventSource {
  onerror: ((event: Event) => void) | null = null;
  readonly close = vi.fn();
  readonly addEventListener = vi.fn();
}

beforeEach(() => {
  vi.clearAllMocks();
  client.openJobStream.mockImplementation(() => new FakeEventSource());
});

describe("run controller focus generation", () => {
  it("does not attach a stream when a create response arrives after close", async () => {
    const pending = deferred<{
      job_id: string;
      run_id: string;
      resumed_from_run_id: null;
    }>();
    client.createJob.mockReturnValue(pending.promise);
    const dispatch = vi.fn();
    const controller = createRunController(dispatch);

    const start = controller.start({
      message: "hello",
      product_session_id: "session-1",
    });
    controller.close();
    pending.resolve({ job_id: "job-1", run_id: "run-1", resumed_from_run_id: null });

    await expect(start).rejects.toBeInstanceOf(RunControllerInactiveError);
    expect(client.openJobStream).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalled();
  });

  it("drops a stale state resync after focused observation closes", async () => {
    const pending = deferred<{
      job_id: string;
      run_id: string;
      status: "running";
      event_count: number;
      events: never[];
      pending_approvals: never[];
      pending_inputs: never[];
    }>();
    client.fetchJobState.mockReturnValue(pending.promise);
    const dispatch = vi.fn();
    const controller = createRunController(dispatch);

    const attach = controller.attach("job-1");
    controller.close();
    pending.resolve({
      job_id: "job-1",
      run_id: "run-1",
      status: "running",
      event_count: 0,
      events: [],
      pending_approvals: [],
      pending_inputs: [],
    });

    await expect(attach).rejects.toBeInstanceOf(RunControllerInactiveError);
    expect(dispatch).not.toHaveBeenCalled();
    expect(client.openJobStream).toHaveBeenCalledTimes(1);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}
