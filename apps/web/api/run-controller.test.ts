import { beforeEach, describe, expect, it, vi } from "vitest";

const client = vi.hoisted(() => ({
  cancelJob: vi.fn(),
  createJob: vi.fn(),
  fetchJobState: vi.fn(),
  listProviderModels: vi.fn(),
  openJobStream: vi.fn(),
  submitApproval: vi.fn(),
  submitInput: vi.fn(),
  submitPlanApproval: vi.fn(),
  stopSubagent: vi.fn(),
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
  readonly listeners = new Map<string, EventListener[]>();
  readonly addEventListener = vi.fn(
    (name: string, listener: EventListenerOrEventListenerObject) => {
      const eventListener =
        typeof listener === "function"
          ? listener
          : (event: Event) => listener.handleEvent(event);
      this.listeners.set(name, [...(this.listeners.get(name) ?? []), eventListener]);
    },
  );

  emit(name: string, data: unknown, lastEventId: string) {
    const event = {
      data: JSON.stringify(data),
      lastEventId,
    } as MessageEvent<string>;
    for (const listener of this.listeners.get(name) ?? []) {
      listener(event);
    }
  }
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
    expect(client.openJobStream).not.toHaveBeenCalled();
  });

  it("waits for the exact run before streaming a reused job id", async () => {
    const oldState = {
      job_id: "job-1",
      run_id: "run-1",
      status: "done" as const,
      event_count: 8,
      events: [],
      pending_approvals: [],
      pending_inputs: [],
    };
    const nextState = {
      job_id: "job-1",
      run_id: "run-2",
      status: "running" as const,
      event_count: 1,
      events: [],
      pending_approvals: [],
      pending_inputs: [],
    };
    client.fetchJobState
      .mockResolvedValueOnce(oldState)
      .mockResolvedValueOnce(nextState);
    const dispatch = vi.fn();
    const controller = createRunController(dispatch);

    await expect(controller.attach("job-1", "run-2")).resolves.toEqual(nextState);

    expect(client.fetchJobState).toHaveBeenCalledTimes(2);
    expect(client.openJobStream).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith({
      type: "job_state_synced",
      state: nextState,
    });
    expect(dispatch).not.toHaveBeenCalledWith({
      type: "job_state_synced",
      state: oldState,
    });
  });

  it("keeps one live observation after cancellation fails", async () => {
    const source = new FakeEventSource();
    client.openJobStream.mockReturnValue(source);
    client.createJob.mockResolvedValue({
      job_id: "job-1",
      run_id: "run-1",
      resumed_from_run_id: null,
    });
    client.cancelJob.mockRejectedValue(new Error("cancel request failed"));
    const dispatch = vi.fn();
    const controller = createRunController(dispatch);

    await controller.start({
      message: "hello",
      product_session_id: "session-1",
    });
    const listenerCount = source.addEventListener.mock.calls.length;

    await expect(controller.cancel("job-1")).rejects.toThrow(
      "cancel request failed",
    );
    expect(source.close).not.toHaveBeenCalled();
    expect(source.addEventListener).toHaveBeenCalledTimes(listenerCount);

    source.emit("llm_chunk", { type: "llm_chunk", delta: "still observed" }, "2");
    expect(
      dispatch.mock.calls.filter(
        ([action]) => action.type === "stream_event" && action.seq === 2,
      ),
    ).toHaveLength(1);
  });

  it("closes the observation after cancellation reaches a terminal state", async () => {
    const source = new FakeEventSource();
    client.openJobStream.mockReturnValue(source);
    client.createJob.mockResolvedValue({
      job_id: "job-1",
      run_id: "run-1",
      resumed_from_run_id: null,
    });
    client.cancelJob.mockResolvedValue({
      job_id: "job-1",
      run_id: "run-1",
      status: "cancelled",
      event_count: 1,
      events: [],
      pending_approvals: [],
      pending_inputs: [],
    });
    const onTerminal = vi.fn();
    const dispatch = vi.fn();
    const controller = createRunController(dispatch, { onTerminal });

    await controller.start({
      message: "hello",
      product_session_id: "session-1",
    });
    await controller.cancel("job-1");

    expect(source.close).toHaveBeenCalledTimes(1);
    expect(onTerminal).toHaveBeenCalledExactlyOnceWith("cancelled");
    dispatch.mockClear();
    source.onerror?.(new Event("error"));
    source.emit("run_started", { type: "run_started", run_id: "run-1" }, "2");
    source.emit("run_completed", { type: "run_completed", reason: "cancelled" }, "3");
    expect(onTerminal).toHaveBeenCalledTimes(1);
    expect(dispatch).not.toHaveBeenCalled();
  });
});

describe("approval authority", () => {
  const snapshot = (status = "running", event_count = 1) => ({
    job_id: "job-1", run_id: "run-1", status, event_count,
    events: [], pending_approvals: [], pending_inputs: [],
  });
  async function observed() {
    const source = new FakeEventSource();
    client.openJobStream.mockReturnValue(source);
    client.fetchJobState.mockResolvedValue(snapshot());
    const dispatch = vi.fn();
    const controller = createRunController(dispatch);
    await controller.attach("job-1", "run-1");
    dispatch.mockClear();
    return { controller, dispatch, source };
  }
  it.each(["approve", "reject"] as const)("projects %s only from the server snapshot and synchronously deduplicates", async (decision) => {
    const { controller, dispatch } = await observed();
    const pending = deferred<ReturnType<typeof snapshot>>();
    client.submitApproval.mockReturnValue(pending.promise);
    const first = controller.approve("job-1", "call-1", decision, "run-1");
    await controller.approve("job-1", "call-1", decision, "run-1");
    expect(client.submitApproval).toHaveBeenCalledTimes(1);
    expect(dispatch).not.toHaveBeenCalled();
    pending.resolve(snapshot("running", 2));
    await first;
    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual(["job_state_synced"]);
  });
  it("does not roll terminal canonical state back when the POST responds late", async () => {
    const { controller, dispatch, source } = await observed();
    const pending = deferred<ReturnType<typeof snapshot>>();
    client.submitApproval.mockReturnValue(pending.promise);
    const request = controller.approve("job-1", "call-1", "approve");
    source.emit("run_completed", { type: "run_completed", run_id: "run-1" }, "5");
    pending.resolve(snapshot("running", 2));
    await request;
    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual(["stream_event"]);
  });
  it("ignores a pending reconnect failure after cancellation", async () => {
    const { controller, dispatch, source } = await observed();
    let reject!: (error: Error) => void;
    client.fetchJobState.mockReturnValue(new Promise((_resolve, fail) => { reject = fail; }));
    source.onerror?.(new Event("error"));
    client.cancelJob.mockResolvedValue(snapshot("cancelled", 3));
    await controller.cancel("job-1");
    dispatch.mockClear();
    reject(new Error("job evicted"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(dispatch).not.toHaveBeenCalled();
  });
  it.each(["answer", "stopSubagent"] as const)("ignores a late %s response after cancellation", async (action) => {
    const { controller, dispatch } = await observed();
    const pending = deferred<ReturnType<typeof snapshot>>();
    client.submitInput.mockReturnValue(pending.promise);
    client.stopSubagent.mockReturnValue(pending.promise);
    const request = action === "answer"
      ? controller.answer("job-1", "input-1", "reply")
      : controller.stopSubagent("job-1", "child-1");
    client.cancelJob.mockResolvedValue(snapshot("cancelled", 3));
    await controller.cancel("job-1");
    dispatch.mockClear();
    pending.resolve(snapshot("running", 3));
    await request;
    expect(dispatch).not.toHaveBeenCalled();
  });
  it.each(["404", "409", "Failed to fetch"])("re-reads %s without resending or claiming approval", async (failure) => {
    const { controller, dispatch } = await observed();
    client.submitApproval.mockRejectedValue(new Error(failure));
    await expect(controller.approve("job-1", "call-1", "reject")).rejects.toThrow("outcome not confirmed");
    expect(client.submitApproval).toHaveBeenCalledTimes(1);
    expect(client.fetchJobState).toHaveBeenCalledTimes(2);
    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual(["job_state_synced"]);
  });
  it("bounds failed authoritative reads", async () => {
    const { controller } = await observed();
    client.submitApproval.mockRejectedValue(new Error("409"));
    client.fetchJobState.mockRejectedValue(new Error("offline"));
    await expect(controller.approve("job-1", "call-1", "approve")).rejects.toThrow("No automatic resend");
    expect(client.fetchJobState).toHaveBeenCalledTimes(4);
    expect(client.submitApproval).toHaveBeenCalledTimes(1);
  });
  it("drops a previous session's response and rejects a mismatched run before POST", async () => {
    const { controller, dispatch } = await observed();
    await expect(controller.approve("job-1", "call-1", "approve", "run-other")).rejects.toThrow("no longer current");
    expect(client.submitApproval).not.toHaveBeenCalled();
    const pending = deferred<ReturnType<typeof snapshot>>();
    client.submitApproval.mockReturnValue(pending.promise);
    const request = controller.approve("job-1", "call-1", "approve");
    controller.close();
    pending.resolve(snapshot());
    await expect(request).rejects.toBeInstanceOf(RunControllerInactiveError);
    expect(dispatch).not.toHaveBeenCalled();
  });
});

describe("plan approval authority", () => {
  const snapshot = (status = "running", event_count = 1) => ({
    job_id: "job-1", run_id: "run-1", status, event_count,
    events: [], pending_approvals: [], pending_inputs: [],
  });
  async function observed() {
    const source = new FakeEventSource();
    client.openJobStream.mockReturnValue(source);
    client.fetchJobState.mockResolvedValue(snapshot());
    const dispatch = vi.fn();
    const controller = createRunController(dispatch);
    await controller.attach("job-1", "run-1");
    dispatch.mockClear();
    return { controller, dispatch, source };
  }

  it("projects the decision only from the server snapshot and deduplicates", async () => {
    const { controller, dispatch } = await observed();
    const pending = deferred<ReturnType<typeof snapshot>>();
    client.submitPlanApproval.mockReturnValue(pending.promise);
    const first = controller.approvePlan("job-1", "approval-1", "auto", "run-1");
    await controller.approvePlan("job-1", "approval-1", "auto", "run-1");
    expect(client.submitPlanApproval).toHaveBeenCalledTimes(1);
    expect(client.submitPlanApproval).toHaveBeenCalledWith("job-1", "approval-1", "auto");
    expect(dispatch).not.toHaveBeenCalled();
    pending.resolve(snapshot("running", 2));
    await first;
    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual(["job_state_synced"]);
  });

  it("rejects a stale run before POST and drops a late response after close", async () => {
    const { controller, dispatch } = await observed();
    await expect(
      controller.approvePlan("job-1", "approval-1", "reject", "run-other"),
    ).rejects.toThrow("no longer current");
    expect(client.submitPlanApproval).not.toHaveBeenCalled();
    const pending = deferred<ReturnType<typeof snapshot>>();
    client.submitPlanApproval.mockReturnValue(pending.promise);
    const request = controller.approvePlan("job-1", "approval-1", "ask");
    controller.close();
    pending.resolve(snapshot());
    await expect(request).rejects.toBeInstanceOf(RunControllerInactiveError);
    expect(dispatch).not.toHaveBeenCalled();
  });

  it("re-reads after a failed POST without resending", async () => {
    const { controller, dispatch } = await observed();
    client.submitPlanApproval.mockRejectedValue(new Error("409"));
    await expect(
      controller.approvePlan("job-1", "approval-1", "auto"),
    ).rejects.toThrow("not confirmed");
    expect(client.submitPlanApproval).toHaveBeenCalledTimes(1);
    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual(["job_state_synced"]);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}
