import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { TranscriptRunGroup } from "../lib/rove-state";
import { Transcript } from "./Transcript";

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

    const html = renderToStaticMarkup(
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

    expect(html.indexOf("Before tool")).toBeLessThan(html.indexOf("read_file"));
    expect(html.indexOf("read_file")).toBeLessThan(html.indexOf("Input submitted"));
    expect(html.indexOf("Input submitted")).toBeLessThan(html.indexOf("After input"));
    expect(html).toContain('data-run-ordinal="1"');
    expect(html).not.toContain("Type your answer");
    expect(html).not.toContain('name="answer"');
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

    const html = renderToStaticMarkup(
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

    expect(html).toContain("Read-only inherited history");
    expect(html).toContain('data-inherited="true"');
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
