import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { ChatMessage } from "../lib/rove-state";
import { MessageEvidence } from "./MessageEvidence";

function renderEvidence(message: ChatMessage) {
  return renderToStaticMarkup(
    <CopyProvider>
      <MessageEvidence message={message} />
    </CopyProvider>,
  );
}

const PROMPT_BUILD = {
  prompt_hash: "sha256:prompt",
  stable_prefix_hash: "sha256:prefix",
  workspace_fingerprint: "sha256:workspace",
  tool_signature: "sha256:tools",
  token_estimate: 40_000,
  included_history_messages: 6,
  dropped_history_messages: 4,
  prompt_cache_key: "sha256:prompt",
};

describe("MessageEvidence", () => {
  it("shows what a pruning compaction request left out, and only when it did", () => {
    const pruned = renderEvidence({
      id: "message-1",
      role: "assistant",
      content: "summarized answer",
      status: "final",
      promptBuild: {
        ...PROMPT_BUILD,
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
    });
    const plain = renderEvidence({
      id: "message-2",
      role: "assistant",
      content: "an unpruned answer",
      status: "final",
      promptBuild: { ...PROMPT_BUILD, pruning: undefined },
    });

    // R9: the loss is measured, not implied — 90 000 B down to 6 000 B.
    expect(pruned).toContain("修剪的工具结果");
    expect(pruned).toContain("3 条 · 少发送 82.0 KiB");
    expect(pruned).toContain("整条未送出的消息");
    expect(pruned).toContain("12 条 · 43.0 KiB");
    expect(pruned).toContain("rove.pruning.v1");
    // The panel says which request the facts belong to, so a reader cannot read
    // them as "the conversation was cut".
    expect(pruned).toContain("仅这次压缩请求；对话本身未删改");
    // Identities are collapsed: eight hashes are noise to a reader not chasing one.
    expect(pruned).toContain("1 条（新到旧）");
    expect(pruned).toContain("call-9:sha256:aaaa");
    // A build that pruned nothing renders no pruning row at all.
    expect(plain).toContain("上下文估计");
    expect(plain).not.toContain("修剪的工具结果");
    expect(plain).not.toContain("仅这次压缩请求");
    expect(pruned.match(/修剪的工具结果/g)).toHaveLength(1);
  });

  it("names an unknown pruning rule instead of repeating it as known", () => {
    const html = renderEvidence({
      id: "message-1",
      role: "assistant",
      content: "answer",
      status: "final",
      promptBuild: {
        ...PROMPT_BUILD,
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
    });

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
    const html = renderEvidence({
      id: "message-1",
      role: "assistant",
      content: "answer",
      status: "final",
      promptBuild: {
        ...PROMPT_BUILD,
        pruning: {
          // The shape the wire actually produces when the probe trimmed
          // its older edge and elided no tool payload: the elision fields
          // are absent, so the parser defaults them to zero.
          pruned_tool_results: 0,
          pruned_payload_bytes: 0,
          pruned_excerpt_bytes: 0,
          pruned_omitted_messages: 9,
          pruned_omitted_bytes: 30_720,
          pruned_payload_digests: [],
          pruning_policy: "rove.pruning.v1",
        },
      },
    });

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
    const html = renderEvidence({
      id: "message-1",
      role: "assistant",
      content: "answer",
      status: "final",
      promptBuild: {
        ...PROMPT_BUILD,
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
    });

    expect(html).toContain("整条未送出的消息");
    expect(html).toContain("9 条 · 30.0 KiB");
    expect(html).toContain("rove.pruning.v1");
    expect(html).toContain("仅这次压缩请求");
    expect(html).not.toContain("修剪的工具结果");
    expect(html).not.toContain("结果标识");
    expect(html).not.toContain("NaN");
  });

  it("renders nothing for a message that published no measured facts", () => {
    const html = renderEvidence({
      id: "message-1",
      role: "assistant",
      content: "plain answer",
      status: "final",
    });
    expect(html).toBe("");
  });
});
