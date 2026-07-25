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

  const waiting = runState.tools.filter((tool) => tool.pendingApproval);
  const tools = runState.tools.slice(0, 12);

  return (
    <aside className="product-inspector" aria-label="Run inspector">
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
            <p style={{ margin: 0, color: "var(--muted)", fontSize: "0.9rem" }}>No plan yet.</p>
          )}
        </section>

        <section className="inspector-section">
          <h3>Approvals</h3>
          {waiting.length === 0 ? (
            <p style={{ margin: 0, color: "var(--muted)", fontSize: "0.9rem" }}>None pending.</p>
          ) : (
            <ul className="tool-list">
              {waiting.map((tool) => (
                <li key={tool.id}>
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
            <p style={{ margin: 0, color: "var(--muted)", fontSize: "0.9rem" }}>No tool calls yet.</p>
          ) : (
            <ul className="tool-list">
              {tools.map((tool: ToolCallView) => (
                <li key={tool.id}>
                  <strong>
                    {tool.name} · {tool.status}
                  </strong>
                  <div style={{ color: "var(--muted)" }}>{truncate(tool.details, 120)}</div>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </aside>
  );
}

function shortId(value: string | null): string {
  return value ? value.slice(0, 10) : "—";
}

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max)}…`;
}
