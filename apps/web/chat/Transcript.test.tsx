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

  it("offers the older server page only while the cursor exists", () => {
    const html = renderTranscript(
      <Transcript
        timeline={singleRunTimeline()}
        approvalBusy={null}
        inputBusy={null}
        restoreState={{ status: "complete", sessionId: "session-1" }}
        olderHistory={{ hasMore: true, cursor: 5, loading: false, error: null }}
        onLoadOlderHistory={vi.fn()}
        onRetryRestore={vi.fn()}
        onStartNewSession={vi.fn()}
        onApproval={vi.fn()}
        onInputSubmit={vi.fn()}
      />,
    );
    expect(html).toContain("load-older-history");
    expect(html).toContain("加载更早的历史");

    // `has_more: false`, a legacy response without the fields, and a cursor-less
    // page all leave only the local mount window.
    for (const olderHistory of [
      { hasMore: false, cursor: null, loading: false, error: null },
      { hasMore: true, cursor: null, loading: false, error: null },
    ]) {
      const withoutPage = renderTranscript(
        <Transcript
          timeline={singleRunTimeline()}
          approvalBusy={null}
          inputBusy={null}
          restoreState={{ status: "complete", sessionId: "session-1" }}
          olderHistory={olderHistory}
          onLoadOlderHistory={vi.fn()}
          onRetryRestore={vi.fn()}
          onStartNewSession={vi.fn()}
          onApproval={vi.fn()}
          onInputSubmit={vi.fn()}
        />,
      );
      expect(withoutPage).not.toContain("load-older-history");
    }
  });

  it("shows a truthful, retryable older-page state", () => {
    const render = (olderHistory: {
      hasMore: boolean;
      cursor: number | null;
      loading: boolean;
      error: string | null;
    }) =>
      renderTranscript(
        <Transcript
          timeline={singleRunTimeline()}
          approvalBusy={null}
          inputBusy={null}
          restoreState={{ status: "complete", sessionId: "session-1" }}
          olderHistory={olderHistory}
          onLoadOlderHistory={vi.fn()}
          onRetryRestore={vi.fn()}
          onStartNewSession={vi.fn()}
          onApproval={vi.fn()}
          onInputSubmit={vi.fn()}
        />,
      );

    const loading = render({
      hasMore: true,
      cursor: 5,
      loading: true,
      error: null,
    });
    expect(loading).toContain("正在加载更早的历史");
    expect(loading).toMatch(/load-older-history[^>]*disabled/);

    const failed = render({
      hasMore: true,
      cursor: 5,
      loading: false,
      error: "Could not load older history: transcript store unavailable",
    });
    // The failure is visible and the same page can be requested again.
    expect(failed).toContain('role="alert"');
    expect(failed).toContain("transcript store unavailable");
    expect(failed).toContain("load-older-history");
    expect(failed).toContain("重试");
  });

  it("marks a salvaged assistant message as aborted beside its text", () => {
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
              content: "partial worth keeping",
              status: "final",
              aborted: true,
            },
          },
          {
            kind: "message",
            entry: entry("message", "message-2", 2),
            message: {
              id: "message-2",
              role: "assistant",
              content: "a complete answer",
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

    // R2b: the salvaged text stays exactly as the model produced it, and the
    // marker is an additive suffix that says the turn was cut short.
    const abortedBubble = html.slice(
      html.indexOf('data-message-id="message-1"'),
      html.indexOf('data-message-id="message-2"'),
    );
    const completeBubble = html.slice(
      html.indexOf('data-message-id="message-2"'),
    );
    expect(abortedBubble).toContain("partial worth keeping");
    expect(abortedBubble).toContain("(已中止)");
    // Only the salvaged message carries it, and a complete one never does.
    expect(completeBubble).toContain("a complete answer");
    expect(completeBubble).not.toContain("(已中止)");
    expect(html.match(/\(已中止\)/g)).toHaveLength(1);
    expect(html).not.toContain("正在回复");
  });
});

function singleRunTimeline(): TranscriptRunGroup[] {
  return [
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
            content: "Restored answer",
            status: "final",
          },
        },
      ],
    },
  ];
}

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
