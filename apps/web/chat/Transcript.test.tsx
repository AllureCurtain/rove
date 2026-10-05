import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { TranscriptRunGroup } from "../lib/rove-state";
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

  it("offers edit-and-branch for a user turn, and only while branching is available", () => {
    const onBranchMessage = vi.fn();
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
              role: "user",
              content: "original request",
              status: "final",
            },
          },
          {
            kind: "message",
            entry: entry("message", "message-2", 2),
            message: {
              id: "message-2",
              role: "assistant",
              content: "Restored answer",
              status: "final",
            },
          },
        ],
      },
    ];
    const render = (available: boolean) =>
      renderTranscript(
        <Transcript
          timeline={timeline}
          approvalBusy={null}
          inputBusy={null}
          restoreState={{ status: "complete", sessionId: "session-1" }}
          onRetryRestore={vi.fn()}
          onStartNewSession={vi.fn()}
          onApproval={vi.fn()}
          onInputSubmit={vi.fn()}
          onBranchMessage={onBranchMessage}
          branchAvailable={available}
        />,
      );

    const available = render(true);
    expect(available).toContain("编辑并分支到新会话");
    // Only one affordance, on the user turn: an assistant reply has no ledger
    // sequence of its own to cut after.
    expect(available.split("编辑并分支到新会话").length - 1).toBe(1);
    expect(available).toContain("original request");

    // A parent that cannot fork (running turn, or no completed run) offers no
    // branch action at all rather than a control the server would refuse.
    expect(render(false)).not.toContain("编辑并分支到新会话");
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
    // The reserved smart-stop copy is rendered for the marked message, so a
    // restored session explains what happened to the text it kept.
    expect(abortedBubble).toContain(
      "已停止：这轮回复已产生部分内容，已保留在对话中。",
    );
    // Only the salvaged message carries it, and a complete one never does.
    expect(completeBubble).toContain("a complete answer");
    expect(completeBubble).not.toContain("(已中止)");
    expect(completeBubble).not.toContain("已停止");
    expect(html.match(/\(已中止\)/g)).toHaveLength(1);
    expect(html.match(/已停止/g)).toHaveLength(1);
    // The marker never enters the body: the text the model produced stays
    // exactly as it was, next to a separate note element.
    expect(abortedBubble).toContain('class="message-abort-note"');
    expect(completeBubble).not.toContain("message-abort-note");
    expect(html).not.toContain("正在回复");
  });

  it("shows what a pruning compaction request left out, and only when it did", () => {
    const prunedMessage = {
      id: "message-1",
      role: "assistant" as const,
      content: "summarized answer",
      status: "final" as const,
      promptBuild: {
        prompt_hash: "sha256:prompt",
        stable_prefix_hash: "sha256:prefix",
        workspace_fingerprint: "sha256:workspace",
        tool_signature: "sha256:tools",
        token_estimate: 40_000,
        included_history_messages: 6,
        dropped_history_messages: 4,
        prompt_cache_key: "sha256:prompt",
        pruning: {
          pruned_tool_results: 3,
          pruned_payload_bytes: 90_000,
          pruned_excerpt_bytes: 6_000,
          pruned_omitted_messages: 12,
          pruned_omitted_bytes: 44_000,
          pruned_payload_digests: ["call-9:sha256:aaaa"],
          pruning_policy: "rove.pruning.v1",
        },
      },
    };
    const plainMessage = {
      id: "message-2",
      role: "assistant" as const,
      content: "an unpruned answer",
      status: "final" as const,
      promptBuild: {
        ...prunedMessage.promptBuild,
        pruning: undefined,
      },
    };
    const timeline: TranscriptRunGroup[] = [
      {
        id: "run:run-1",
        runId: "run-1",
        runOrdinal: 1,
        inherited: false,
        sourceSessionId: null,
        items: [
          { kind: "message", entry: entry("message", "message-1", 1), message: prunedMessage },
          { kind: "message", entry: entry("message", "message-2", 2), message: plainMessage },
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

    const prunedBubble = html.slice(
      html.indexOf('data-message-id="message-1"'),
      html.indexOf('data-message-id="message-2"'),
    );
    const plainBubble = html.slice(html.indexOf('data-message-id="message-2"'));

    // R9: the loss is measured, not implied — 90 000 B down to 6 000 B.
    expect(prunedBubble).toContain("修剪的工具结果");
    expect(prunedBubble).toContain("3 条 · 少发送 82.0 KiB");
    expect(prunedBubble).toContain("整条未送出的消息");
    expect(prunedBubble).toContain("12 条 · 43.0 KiB");
    expect(prunedBubble).toContain("rove.pruning.v1");
    // The panel says which request the facts belong to, so a reader cannot read
    // them as "the conversation was cut".
    expect(prunedBubble).toContain("仅这次压缩请求；对话本身未删改");
    // Identities are collapsed: eight hashes are noise to a reader not chasing one.
    expect(prunedBubble).toContain("1 条（新到旧）");
    expect(prunedBubble).toContain("call-9:sha256:aaaa");
    // A build that pruned nothing renders no pruning row at all.
    expect(plainBubble).toContain("上下文估计");
    expect(plainBubble).not.toContain("修剪的工具结果");
    expect(plainBubble).not.toContain("仅这次压缩请求");
    expect(html.match(/修剪的工具结果/g)).toHaveLength(1);
  });

  it("names an unknown pruning rule instead of repeating it as known", () => {
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
              content: "answer",
              status: "final",
              promptBuild: {
                prompt_hash: "sha256:prompt",
                stable_prefix_hash: "sha256:prefix",
                workspace_fingerprint: "sha256:workspace",
                tool_signature: "sha256:tools",
                token_estimate: 1_500,
                included_history_messages: 2,
                dropped_history_messages: 1,
                prompt_cache_key: "sha256:prompt",
                pruning: {
                  pruned_tool_results: 1,
                  pruned_payload_bytes: 20_480,
                  pruned_excerpt_bytes: 2_048,
                  pruned_omitted_messages: 0,
                  pruned_omitted_bytes: 0,
                  pruned_payload_digests: [],
                  pruning_policy: "rove.pruning.v9",
                },
              },
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

    // The rule is still reported as a fact about this request; only its name is
    // withheld, because this build cannot vouch for what "v9" does.
    expect(html).toContain("修剪规则");
    expect(html).toContain("未知规则");
    expect(html).not.toContain("rove.pruning.v9");
    // Nothing omitted and no identities: those rows are absent, not zeroed.
    expect(html).not.toContain("整条未送出的消息");
    expect(html).not.toContain("结果标识");
  });

  it("reports a request that only left whole messages out as that, not as zero elisions", () => {
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
              content: "answer",
              status: "final",
              promptBuild: {
                prompt_hash: "sha256:prompt",
                stable_prefix_hash: "sha256:prefix",
                workspace_fingerprint: "sha256:workspace",
                tool_signature: "sha256:tools",
                token_estimate: 1_500,
                included_history_messages: 2,
                dropped_history_messages: 9,
                prompt_cache_key: "sha256:prompt",
                // The shape the wire actually produces when the probe trimmed
                // its older edge and elided no tool payload: the elision fields
                // are absent, so the parser defaults them to zero.
                pruning: {
                  pruned_tool_results: 0,
                  pruned_payload_bytes: 0,
                  pruned_excerpt_bytes: 0,
                  pruned_omitted_messages: 9,
                  pruned_omitted_bytes: 30_720,
                  pruned_payload_digests: [],
                  pruning_policy: "rove.pruning.v1",
                },
              },
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

    expect(html).toContain("整条未送出的消息");
    expect(html).toContain("9 条 · 30.0 KiB");
    // "0 条 · 少发送 0 B" would be a measurement the server never made: those
    // fields are absent on the wire, and absence is not a reported zero.
    expect(html).not.toContain("修剪的工具结果");
    expect(html).not.toContain("0 B");
    expect(html).toContain("仅这次压缩请求");
  });

  it("renders a fact set that names only some of its members", () => {
    // Both arrival paths decode the wire record before it reaches this panel
    // (`readPromptBuildMetadata` for a live frame, the product parser for a
    // restored transcript), so a member is missing here only because a producer
    // older than that decoding sent a sparse record — a probe that dropped older
    // messages carries three keys and no digest list at all. Absence must read as
    // "nothing of that kind was elided": not a crash, and not a measured zero.
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
              content: "answer",
              status: "final",
              promptBuild: {
                prompt_hash: "sha256:prompt",
                stable_prefix_hash: "sha256:prefix",
                workspace_fingerprint: "sha256:workspace",
                tool_signature: "sha256:tools",
                token_estimate: 1_500,
                included_history_messages: 2,
                dropped_history_messages: 9,
                prompt_cache_key: "sha256:prompt",
                pruning: {
                  pruned_tool_results: 0,
                  pruned_payload_bytes: 0,
                  pruned_excerpt_bytes: 0,
                  pruned_omitted_messages: 9,
                  pruned_omitted_bytes: 30_720,
                  pruned_payload_digests: [],
                  pruning_policy: "rove.pruning.v1",
                },
              },
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

    expect(html).toContain("整条未送出的消息");
    expect(html).toContain("9 条 · 30.0 KiB");
    expect(html).toContain("rove.pruning.v1");
    expect(html).toContain("仅这次压缩请求");
    expect(html).not.toContain("修剪的工具结果");
    expect(html).not.toContain("结果标识");
    expect(html).not.toContain("NaN");
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
