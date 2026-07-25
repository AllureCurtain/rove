"use client";

import { ChevronLeftIcon, ChevronRightIcon } from "@radix-ui/react-icons";

import type { ToolCallView, WorkbenchState } from "../lib/rove-state";

export function RunInspector({
  collapsed,
  onToggle,
  runState,
}: {
  collapsed: boolean;
  onToggle: () => void;
  runState: WorkbenchState;
}) {
  if (collapsed) {
    return (
      <aside className="product-inspector" data-collapsed="true" aria-label="Run inspector">
        <div className="inspector-header">
          <button
            type="button"
            className="ghost icon-button"
            onClick={onToggle}
            aria-label="Expand inspector"
          >
            <ChevronLeftIcon />
          </button>
        </div>
      </aside>
    );
  }

  const phase = resolveInspectorPhase(runState);
  const waiting = runState.tools.filter((tool) => tool.pendingApproval);
  const tools = runState.tools.slice(0, 12);

  return (
    <aside className="product-inspector" aria-label="Run inspector" data-phase={phase}>
      <div className="inspector-header">
        <h2>Inspector</h2>
        <button
          type="button"
          className="ghost icon-button"
          onClick={onToggle}
          aria-label="Collapse inspector"
        >
          <ChevronRightIcon />
        </button>
      </div>
      <div className="inspector-body">
        {phase === "empty" ? (
          <div className="inspector-state" data-tone="empty" role="status">
            <strong>No active run</strong>
            <p>
              Send a message to start a turn. Plan, tools, and approvals for the latest run will
              appear here.
            </p>
          </div>
        ) : null}

        {phase === "loading" ? (
          <div className="inspector-state" data-tone="loading" role="status" aria-live="polite">
            <strong>{runState.statusText || "Working…"}</strong>
            <div className="inspector-skeleton" aria-hidden="true">
              <span />
              <span />
              <span />
            </div>
            <p>Streaming run events. Approvals and tools update as the agent works.</p>
          </div>
        ) : null}

        {phase === "error" ? (
          <div className="inspector-state" data-tone="error" role="alert">
            <strong>Run interrupted</strong>
            <p>{runState.error}</p>
          </div>
        ) : null}

        {phase !== "empty" ? (
          <>
            <section className="inspector-section">
              <h3>Run</h3>
              <div className="inspector-kv">
                <div>
                  <span>status</span>
                  <strong>{runState.statusText}</strong>
                </div>
                <div>
                  <span>job</span>
                  <strong>{shortId(runState.activeJobId)}</strong>
                </div>
                <div>
                  <span>run</span>
                  <strong>{shortId(runState.activeRunId)}</strong>
                </div>
                <div>
                  <span>resumed from</span>
                  <strong>{shortId(runState.resumedFromRunId)}</strong>
                </div>
                <div>
                  <span>events</span>
                  <strong>{runState.eventCount}</strong>
                </div>
                <div>
                  <span>signal</span>
                  <strong>{runState.lastSignal}</strong>
                </div>
              </div>
            </section>

            <section className="inspector-section">
              <h3>Plan</h3>
              {runState.plan ? (
                <ul className="plan-list">
                  {runState.plan.steps.map((step) => (
                    <li key={step.id} data-done={step.done}>
                      {step.done ? "✓ " : "• "}
                      {step.title}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="inspector-empty-line">
                  {phase === "loading" ? "Waiting for plan…" : "No plan for this run."}
                </p>
              )}
            </section>

            <section className="inspector-section">
              <h3>Approvals</h3>
              {waiting.length === 0 ? (
                <p className="inspector-empty-line">
                  {phase === "loading" ? "No approvals yet." : "None pending."}
                </p>
              ) : (
                <ul className="tool-list">
                  {waiting.map((tool) => (
                    <li key={tool.id} data-tone="waiting">
                      <strong>{tool.name}</strong>
                      <div>{tool.reason ?? tool.details}</div>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="inspector-section">
              <h3>Tools</h3>
              {tools.length === 0 ? (
                <p className="inspector-empty-line">
                  {phase === "loading" ? "Waiting for tool calls…" : "No tool calls yet."}
                </p>
              ) : (
                <ul className="tool-list">
                  {tools.map((tool: ToolCallView) => (
                    <li key={tool.id} data-status={tool.status}>
                      <strong>
                        {tool.name} · {tool.status}
                      </strong>
                      <div className="tool-list__detail">{truncate(tool.details, 120)}</div>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </>
        ) : null}
      </div>
    </aside>
  );
}

export type InspectorPhase = "empty" | "loading" | "error" | "ready";

export function resolveInspectorPhase(runState: WorkbenchState): InspectorPhase {
  if (runState.error) {
    return "error";
  }
  if (runState.busy) {
    return "loading";
  }
  const hasRunIdentity = Boolean(runState.activeJobId || runState.activeRunId);
  const hasRunContent =
    Boolean(runState.plan) ||
    runState.tools.length > 0 ||
    runState.eventCount > 0 ||
    runState.messages.length > 0;
  if (!hasRunIdentity && !hasRunContent) {
    return "empty";
  }
  return "ready";
}

function shortId(value: string | null): string {
  return value ? value.slice(0, 10) : "—";
}

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max)}…`;
}
