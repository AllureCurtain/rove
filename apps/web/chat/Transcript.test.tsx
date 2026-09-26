import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { TranscriptRunGroup } from "../lib/rove-state";
import type { ProductMessage, ProductMessageStatus } from "../product/product-api-types";
import { Transcript } from "./Transcript";

function renderTranscript(element: React.ReactElement) {
  return renderToStaticMarkup(<CopyProvider>{element}</CopyProvider>);
}

describe("Transcript", () => {
  it("renders canonical items in order and keeps handled input read-only", () => {
    const timeline: TranscriptRunGroup[] = [
      {
        id: "run:run-1",
        runId: "run-1",
        runOrdinal: 1,
        inherited: false,
        sourceSessionId: null,
        items: [
          {
            kind: "message",
            entry: entry("message", "message-1", 1),
            message: {
              id: "message-1",
              role: "assistant",
              content: "Before tool",
              status: "final",
            },
          },
          {
            kind: "tool",
            entry: entry("tool", "run:run-1:tool:call-1", 2),
            tool: {
              id: "call-1",
              timelineId: "run:run-1:tool:call-1",
              name: "read_file",
              status: "done",
              details: "complete",
            },
          },
          {
            kind: "input",
            entry: entry("input", "run:run-1:input:input-1", 3),
            input: {
              id: "input-1",
              timelineId: "run:run-1:input:input-1",
              prompt: "Which format?",
              status: "submitted",
            },
          },
          {
            kind: "message",
            entry: entry("message", "message-2", 4),
            message: {
              id: "message-2",
              role: "assistant",
              content: "After input",
              status: "final",
            },
          },
        ],
      },
    ];

    const html = renderTranscript(
      <Transcript
        timeline={timeline}
        approvalBusy={null}
        inputBusy={null}
        restoreState={{ status: "complete", sessionId: "session-1" }}
        onRetryRestore={vi.fn()}
        onStartNewSession={vi.fn()}
        onApproval={vi.fn()}
        onInputSubmit={vi.fn()}
      />,
    );

    // Consecutive tools and inputs fold into one activity between the answers.
    const activityIndex = html.indexOf("工具调用 × 1");
    expect(html.indexOf("Before tool")).toBeLessThan(activityIndex);
    expect(activityIndex).toBeLessThan(html.indexOf("After input"));
    expect(html).toContain('data-run-ordinal="1"');
    // The folded body stays out of the server-rendered DOM: settled tool and
    // input details only appear once the reader opens the activity.
    expect(html).not.toContain("read_file");
    expect(html).not.toContain("Which format?");
    expect(html).not.toContain("Type your answer");
    expect(html).not.toContain('name="answer"');
  });

  it("keeps a pending approval outside the folded activity body", () => {
    const timeline: TranscriptRunGroup[] = [
      {
        id: "run:run-1",
        runId: "run-1",
        runOrdinal: 1,
        inherited: false,
        sourceSessionId: null,
        items: [
          {
            kind: "tool",
            entry: entry("tool", "run:run-1:tool:call-1", 1),
            tool: {
              id: "call-1",
              timelineId: "run:run-1:tool:call-1",
              name: "write_file",
              status: "waiting",
              details: "needs approval",
              pendingApproval: {
                call_id: "call-1",
                name: "write_file",
                args: { path: "a.txt" },
                reason: "needs approval",
              },
            },
          },
          {
            kind: "tool",
            entry: entry("tool", "run:run-1:tool:call-2", 2),
            tool: {
              id: "call-2",
              timelineId: "run:run-1:tool:call-2",
              name: "read_file",
              status: "done",
              details: "complete",
            },
          },
        ],
      },
    ];

    const html = renderTranscript(
      <Transcript
        timeline={timeline}
        approvalBusy={null}
        inputBusy={null}
        restoreState={{ status: "complete", sessionId: "session-1" }}
        onRetryRestore={vi.fn()}
        onStartNewSession={vi.fn()}
        onApproval={vi.fn()}
        onInputSubmit={vi.fn()}
      />,
    );

    // The approval card is rendered before the fold body — and even when the
    // body is collapsed it stays in the DOM: a blocked run must keep its
    // interaction reachable.
    expect(html).toContain("write_file");
    expect(html.indexOf("approval-card")).toBeLessThan(
      html.indexOf("activity-group__body"),
    );
  });

  it("labels inherited fork history as read-only", () => {
    const timeline: TranscriptRunGroup[] = [
      {
        id: "run:parent-run",
        runId: "parent-run",
        runOrdinal: 1,
        inherited: true,
        sourceSessionId: "parent-product-session",
        items: [
          {
            kind: "message",
            entry: {
              ...entry("message", "parent-message", 1),
              runId: "parent-run",
            },
            message: {
              id: "parent-message",
              role: "assistant",
              content: "Parent answer",
              status: "final",
            },
          },
        ],
      },
    ];

    const html = renderTranscript(
      <Transcript
        timeline={timeline}
        approvalBusy={null}
        inputBusy={null}
        restoreState={{ status: "complete", sessionId: "child-session" }}
        onRetryRestore={vi.fn()}
        onStartNewSession={vi.fn()}
        onApproval={vi.fn()}
        onInputSubmit={vi.fn()}
      />,
    );

    expect(html).toContain("派生会话");
    expect(html).toContain('data-inherited="true"');
  });

  it("renders only actionable ledger messages beside canonical history", () => {
    const statuses: ProductMessageStatus[] = [
      "queued",
      "intervention_requested",
      "applied_current_run",
      "claimed_successor",
      "needs_attention",
      "revoked",
    ];
    const messages: ProductMessage[] = statuses.map((status, index) => ({
      id: `message-${index + 1}`,
      product_session_id: "session-1",
      content: `ledger-${status}`,
      requested_delivery: status === "intervention_requested" || status === "applied_current_run"
        ? "current_run"
        : "successor",
      status,
      seq: index + 1,
      created_at: "2026-08-14T00:00:00Z",
    }));

    const html = renderTranscript(
      <Transcript
        timeline={[]}
        messages={messages}
        approvalBusy={null}
        inputBusy={null}
        restoreState={{ status: "complete", sessionId: "session-1" }}
        onRetryRestore={vi.fn()}
        onStartNewSession={vi.fn()}
        onApproval={vi.fn()}
        onInputSubmit={vi.fn()}
      />,
    );

    expect(html).toContain("ledger-queued");
    expect(html).toContain("ledger-intervention_requested");
    expect(html).toContain("ledger-needs_attention");
    expect(html).not.toContain("ledger-applied_current_run");
    expect(html).not.toContain("ledger-claimed_successor");
    expect(html).not.toContain("ledger-revoked");
  });
});

function entry(
  kind: "message" | "tool" | "input",
  entityId: string,
  eventSeq: number,
) {
  return {
    id: `run:run-1:${kind}:${eventSeq}:${entityId}`,
    kind,
    entityId,
    runId: "run-1",
    runOrdinal: 1,
    eventSeq,
    inherited: false,
    sourceSessionId: null,
  };
}
