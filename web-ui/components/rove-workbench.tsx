"use client";

import {
  ActivityLogIcon,
  ClockIcon,
  CubeIcon,
  DotFilledIcon,
  FileTextIcon,
  PlayIcon,
  ReloadIcon,
  StopIcon,
} from "@radix-ui/react-icons";
import {
  type FormEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";

import { cancelJob, createJob, openJobStream } from "../lib/rove-client";
import { STREAM_EVENT_NAMES, type StreamEvent } from "../lib/rove-types";
import { createWorkbenchState, workbenchReducer } from "../lib/rove-state";

export function RoveWorkbench() {
  const [state, dispatch] = useReducer(
    workbenchReducer,
    undefined,
    createWorkbenchState,
  );
  const [message, setMessage] = useState("inspect this workspace");
  const [model, setModel] = useState("fake");
  const [maxSteps, setMaxSteps] = useState("8");
  const [submitting, setSubmitting] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);

  const isBusy = submitting || state.busy;
  const completedSteps = state.plan?.steps.filter((step) => step.done).length ?? 0;
  const totalSteps = state.plan?.steps.length ?? 0;
  const activeStep = state.plan?.steps[state.plan.current_step]?.title ?? "idle";
  const modelLabel = model.trim() || "fake";
  const stepsLabel = `${maxSteps || "8"} steps`;
  const runMeta = useMemo(() => {
    if (!state.activeJobId) {
      return "no active run";
    }
    return `job ${shortId(state.activeJobId)} / run ${shortId(state.activeRunId)}`;
  }, [state.activeJobId, state.activeRunId]);

  useEffect(() => {
    return () => {
      eventSourceRef.current?.close();
      eventSourceRef.current = null;
    };
  }, []);

  const statusTone = state.error
    ? "error"
    : isBusy
      ? "working"
      : state.activeJobId
        ? "done"
        : "idle";

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = message.trim();
    if (!trimmed || isBusy) {
      return;
    }

    closeStream();
    dispatch({ type: "reset" });
    dispatch({ type: "append_user_message", content: trimmed });
    dispatch({ type: "set_status", statusText: "Submitting job" });
    setSubmitting(true);

    try {
      const job = await createJob({
        message: trimmed,
        model: model.trim() || undefined,
        max_steps: Number(maxSteps) || undefined,
      });

      dispatch({ type: "job_created", jobId: job.job_id, runId: job.run_id });
      attachStream(job.job_id);
    } catch (error) {
      dispatch({ type: "set_error", error: describeError(error) });
    } finally {
      setSubmitting(false);
    }
  }

  async function handleCancel() {
    if (!state.activeJobId || !state.busy) {
      return;
    }

    dispatch({ type: "set_status", statusText: "Cancelling run" });
    try {
      await cancelJob(state.activeJobId);
    } catch (error) {
      dispatch({ type: "set_error", error: describeError(error) });
    } finally {
      closeStream();
      dispatch({ type: "set_busy", busy: false });
      dispatch({ type: "set_status", statusText: "Run cancelled" });
    }
  }

  function attachStream(jobId: string) {
    const source = openJobStream(jobId);
    eventSourceRef.current = source;

    for (const name of STREAM_EVENT_NAMES) {
      source.addEventListener(name, handleEvent as EventListener);
    }
  }

  function handleEvent(event: Event) {
    const payload = JSON.parse((event as MessageEvent<string>).data) as StreamEvent;
    dispatch({ type: "stream_event", event: payload });

    if (payload.type === "run_completed") {
      closeStream();
    }
  }

  function closeStream() {
    eventSourceRef.current?.close();
    eventSourceRef.current = null;
  }

  return (
    <main className="workbench-shell">
      <div className="workbench-shell__ambient" aria-hidden="true" />

      <div className="workbench">
        <header className="hero-band">
          <div className="hero-band__copy">
            <div className="brand-line">
              <h1>rove</h1>
              <div className="eyebrow">
                <ActivityLogIcon />
                <span>runtime console</span>
              </div>
            </div>
            <div className="hero-band__meta">
              <span>{state.activeJobId ? "connected" : "idle"}</span>
              <span>{modelLabel}</span>
              <span>{stepsLabel}</span>
            </div>
          </div>

          <div className="hero-band__status">
            <div className="status-chip" data-tone={statusTone}>
              <DotFilledIcon />
              <span>{state.error ?? state.statusText}</span>
            </div>

            <div className="metric-grid">
              <Metric label="events" value={String(state.eventCount)} icon={<ReloadIcon />} />
              <Metric label="plan" value={`${completedSteps}/${totalSteps}`} icon={<FileTextIcon />} />
              <Metric label="tools" value={String(state.tools.length)} icon={<CubeIcon />} />
              <Metric label="trace" value={String(state.trace.length)} icon={<ActivityLogIcon />} />
            </div>
          </div>
        </header>

        <section className="signal-band" aria-label="Run summary">
          <div className="signal-band__cell">
            <span>workspace</span>
            <strong>{runMeta}</strong>
            <p>{state.lastSignal}</p>
          </div>
          <div className="signal-band__cell">
            <span>active step</span>
            <strong>{activeStep}</strong>
            <p>{state.activeJobId ? state.statusText : "Awaiting a task"}</p>
          </div>
          <div className="signal-band__cell">
            <span>model</span>
            <strong>{modelLabel}</strong>
            <p>{stepsLabel}</p>
          </div>
          <div className="signal-band__cell signal-band__cell--right">
            <span>state</span>
            <strong>{state.error ?? state.statusText}</strong>
            <p>{state.busy ? "streaming" : "idle"}</p>
          </div>
        </section>

        <section className="workspace-grid">
          <div className="workspace-main">
            <form className="composer" onSubmit={handleSubmit}>
              <label className="field field--task">
                <span>Task</span>
                <textarea
                  value={message}
                  onChange={(event) => setMessage(event.target.value)}
                  placeholder="Inspect, plan, or modify this workspace"
                  required
                />
              </label>

              <div className="composer-controls">
                <label className="field">
                  <span>Model</span>
                  <input
                    value={model}
                    onChange={(event) => setModel(event.target.value)}
                    placeholder="fake"
                  />
                </label>

                <label className="field field--steps">
                  <span>Steps</span>
                  <input
                    value={maxSteps}
                    onChange={(event) => setMaxSteps(event.target.value)}
                    type="number"
                    min={1}
                    max={50}
                  />
                </label>

                <div className="action-row">
                  <button type="submit" disabled={isBusy}>
                    <PlayIcon width={15} height={15} />
                    Run
                  </button>
                  <button
                    type="button"
                    className="danger"
                    onClick={handleCancel}
                    disabled={!state.activeJobId || !state.busy}
                  >
                    <StopIcon width={15} height={15} />
                    Stop
                  </button>
                </div>
              </div>
            </form>

            <div className="section-head">
              <div>
                <h2>Conversation</h2>
                <p>{state.messages.length} messages in the current run</p>
              </div>
              <div className="section-head__meta">
                <span>{state.activeJobId ? "connected" : "idle"}</span>
                <span>{state.busy ? "streaming" : "ready"}</span>
              </div>
            </div>

            <div className="message-stream" aria-live="polite" aria-busy={isBusy}>
              {state.messages.length === 0 ? (
                <EmptyStage
                  busy={isBusy}
                  workspace={runMeta}
                  eventCount={state.eventCount}
                  traceCount={state.trace.length}
                />
              ) : (
                state.messages.map((item) => (
                  <article
                    key={item.id}
                    className={`message-card message-card--${item.role} ${
                      item.role === "assistant" && item.status === "streaming"
                        ? "message-card--streaming"
                        : ""
                    }`}
                  >
                    <div className="message-card__meta">
                      <span>{item.role}</span>
                      <span>{item.status}</span>
                    </div>
                    <p>{item.content}</p>
                  </article>
                ))
              )}
            </div>
          </div>

          <aside className="workspace-rail" aria-label="Run details">
            <InspectorSection title="Live" icon={<ClockIcon />}>
              <SummaryRow label="last signal" value={state.lastSignal} />
              <SummaryRow label="status" value={state.statusText} />
              <SummaryRow label="job" value={shortId(state.activeJobId)} />
              <SummaryRow label="run" value={shortId(state.activeRunId)} />
            </InspectorSection>

            <InspectorSection title="Plan" icon={<FileTextIcon />}>
              {state.plan ? (
                <div className="stack-list">
                  {state.plan.steps.map((step, index) => (
                    <article
                      key={step.id}
                      className="stack-row stack-row--plan"
                      data-state={
                        step.done
                          ? "done"
                          : index === state.plan?.current_step
                            ? "running"
                            : "idle"
                      }
                    >
                      <span className="stack-index">{index + 1}</span>
                      <div>
                        <strong>{step.title}</strong>
                        <p>
                          {step.done
                            ? "done"
                            : index === state.plan?.current_step
                              ? "running"
                              : "pending"}
                        </p>
                      </div>
                    </article>
                  ))}
                </div>
              ) : (
                <EmptyBlock label="No plan yet" busy={isBusy} />
              )}
            </InspectorSection>

            <InspectorSection title="Tools" icon={<CubeIcon />}>
              {state.tools.length ? (
                <div className="stack-list">
                  {state.tools.map((tool) => (
                    <article key={tool.id} className="stack-row" data-state={tool.status}>
                      <div className="stack-row__header">
                        <strong>{tool.name}</strong>
                        <span>{tool.status}</span>
                      </div>
                      <p>{tool.details}</p>
                    </article>
                  ))}
                </div>
              ) : (
                <EmptyBlock label="No tool calls yet" busy={isBusy} />
              )}
            </InspectorSection>

            <InspectorSection title="Trace" icon={<ActivityLogIcon />}>
              {state.trace.length ? (
                <div className="stack-list">
                  {state.trace.map((entry) => (
                    <article key={entry.id} className="stack-row">
                      <div className="stack-row__header">
                        <strong>{entry.label}</strong>
                      </div>
                      <p>{entry.detail}</p>
                    </article>
                  ))}
                </div>
              ) : (
                <EmptyBlock label="Trace is empty" busy={isBusy} />
              )}
            </InspectorSection>
          </aside>
        </section>
      </div>
    </main>
  );
}

function Metric({
  label,
  value,
  icon,
}: {
  label: string;
  value: string;
  icon: ReactNode;
}) {
  return (
    <div className="metric">
      <span className="metric__icon">{icon}</span>
      <div>
        <span>{label}</span>
        <strong>{value}</strong>
      </div>
    </div>
  );
}

function InspectorSection({
  title,
  icon,
  children,
}: {
  title: string;
  icon: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="rail-section">
      <div className="rail-section__head">
        <h3>
          <span className="rail-section__icon">{icon}</span>
          {title}
        </h3>
      </div>
      {children}
    </section>
  );
}

function SummaryRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="summary-row">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function EmptyStage({
  busy,
  workspace,
  eventCount,
  traceCount,
}: {
  busy: boolean;
  workspace: string;
  eventCount: number;
  traceCount: number;
}) {
  if (busy) {
    return (
      <div className="empty-stage" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>
    );
  }

  return (
    <div className="empty-state">
      <div className="empty-state__row">
        <span>workspace</span>
        <strong>{workspace}</strong>
      </div>
      <div className="empty-state__row">
        <span>events</span>
        <strong>{eventCount}</strong>
      </div>
      <div className="empty-state__row">
        <span>trace</span>
        <strong>{traceCount}</strong>
      </div>
    </div>
  );
}

function EmptyBlock({ label, busy }: { label: string; busy: boolean }) {
  if (busy) {
    return (
      <div className="empty-block empty-block--loading" aria-hidden="true">
        <span />
        <span />
      </div>
    );
  }

  return <div className="empty-block">{label}</div>;
}

function shortId(value: string | null): string {
  return value ? value.slice(0, 10) : "—";
}

function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}
