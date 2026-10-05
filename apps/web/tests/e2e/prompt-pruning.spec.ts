import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockSession,
  type MockTranscript,
  type MockWorkspace,
} from "./product-api-mock";

/**
 * R9: what a bounded compaction request left out, on the turn that made it.
 *
 * The runtime records the pruning facts on the same `PromptBuilt` metadata the
 * turn already publishes, so this is a rendering of facts rather than a new
 * contract: the panel has to state the measured loss, name the rule, and stay
 * silent on a turn that pruned nothing — including the case a reader would
 * misread, that the pruning belongs to the compaction request and not to the
 * conversation.
 */

const PRUNING_FACTS = {
  pruned_tool_results: 3,
  pruned_payload_bytes: 90_000,
  pruned_excerpt_bytes: 6_000,
  pruned_omitted_messages: 12,
  pruned_omitted_bytes: 44_000,
  pruned_payload_digests: ["call-9:sha256:aaaa", "call-4:sha256:bbbb"],
  pruning_policy: "rove.pruning.v1",
};

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

/**
 * The restored transcript of a run with a `PromptBuilt` event.
 *
 * `facts` is spliced onto the metadata the turn always publishes, or omitted
 * entirely when null — the two shapes the panel has to tell apart. The event is
 * spliced into a real completed segment and every later seq is shifted, so the
 * durable position the segment reports stays consistent with the events it
 * carries; a transcript whose last event seq disagreed with its
 * `observed_through_seq` would be served as a partial restore instead.
 */
function builtTranscript(
  workspace: MockWorkspace,
  session: MockSession,
  facts: Record<string, unknown> | null,
): MockTranscript {
  const transcript = completedTranscript(
    workspace,
    session,
    "Summarize the build log",
    "The log was loud.",
  );
  const segment = transcript.segments[0]!;
  const events = segment.events as Array<{
    seq: number;
    event: Record<string, unknown>;
  }>;
  const spliced = [
    events[0]!,
    {
      seq: 2,
      event: {
        type: "prompt_built",
        metadata: { ...PROMPT_BUILD, ...(facts ?? {}) },
      },
    },
    ...events.slice(1).map((entry) => ({ ...entry, seq: entry.seq + 1 })),
  ];
  return {
    ...transcript,
    segments: [
      {
        ...segment,
        observed_through_seq: spliced.at(-1)!.seq,
        last_event_seq: spliced.at(-1)!.seq,
        events: spliced,
      },
    ],
  };
}

async function openShell(page: Page, facts: Record<string, unknown> | null) {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-pruning", workspace.id, "Pruning session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: builtTranscript(workspace, session, facts) },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("The log was loud.", { exact: true })).toBeVisible();
  return { workspace, session, panel: page.locator(".message-evidence") };
}

test("the panel reports the loss the compaction request measured", async ({ page }) => {
  const { panel } = await openShell(page, PRUNING_FACTS);

  await expect(panel).toBeVisible();
  // The bytes are the request's own measurement: 90 000 in, 6 000 out.
  await expect(panel).toContainText("修剪的工具结果");
  await expect(panel).toContainText("3 条 · 少发送 82.0 KiB");
  await expect(panel).toContainText("整条未送出的消息");
  await expect(panel).toContainText("12 条 · 43.0 KiB");
  await expect(panel).toContainText("修剪规则");
  await expect(panel).toContainText("rove.pruning.v1");
  // The scope line is what stops a reader concluding the conversation lost text.
  await expect(panel).toContainText("仅这次压缩请求；对话本身未删改");
  // The context estimate the turn always published is still there beside it.
  await expect(panel).toContainText("上下文估计");
  await expect(panel).toContainText("40,000");

  // Identities are collapsed and stay collapsed until a reader asks for them.
  const digests = panel.locator("details.message-evidence__digests");
  await expect(digests.locator("summary")).toHaveText("2 条（新到旧）");
  await expect(digests.getByText("call-9:sha256:aaaa")).toBeHidden();
  await digests.locator("summary").click();
  await expect(digests.getByText("call-9:sha256:aaaa")).toBeVisible();
  await expect(digests.getByText("call-4:sha256:bbbb")).toBeVisible();
});

test("a turn that pruned nothing says nothing about pruning", async ({ page }) => {
  // The metadata is present — this is a real `PromptBuilt` — but it carries no
  // pruning facts, so the panel shows the context estimate and no pruning row.
  const { panel } = await openShell(page, null);

  await expect(panel).toBeVisible();
  await expect(panel).toContainText("上下文估计");
  await expect(panel).not.toContainText("修剪的工具结果");
  await expect(panel).not.toContainText("修剪规则");
  await expect(panel).not.toContainText("仅这次压缩请求");
});

test("a rule this build cannot name is reported as a fact, not as a name", async ({
  page,
}) => {
  // A real wire shape: the probe elided payloads and dropped no whole messages,
  // so the counts and identities are carried and the omitted-message keys are
  // absent instead of zero.
  const { panel } = await openShell(page, {
    pruned_tool_results: 3,
    pruned_payload_bytes: 90_000,
    pruned_excerpt_bytes: 6_000,
    pruned_payload_digests: ["call-9:sha256:aaaa", "call-4:sha256:bbbb"],
    pruning_policy: "rove.pruning.v9",
  });

  await expect(panel).toContainText("修剪规则");
  await expect(panel).toContainText("未知规则");
  // The version string is not echoed as if this build understood it.
  await expect(panel).not.toContainText("rove.pruning.v9");
  // The facts themselves are still reported: only the rule's name is withheld.
  await expect(panel).toContainText("3 条 · 少发送 82.0 KiB");
  await expect(panel).toContainText("2 条（新到旧）");
  // The omission class was not on the wire, so it has no row.
  await expect(panel).not.toContainText("整条未送出的消息");
});

test("a request that only trimmed its older edge reports that, not a zero elision", async ({
  page,
}) => {
  // The shape the wire produces when the probe dropped whole messages and
  // elided no tool payload: the elision fields are absent, so a row built from
  // them would report "0 条 · 少发送 0 B" — a measurement the server never made.
  const { panel } = await openShell(page, {
    pruned_omitted_messages: 9,
    pruned_omitted_bytes: 30_720,
    pruning_policy: "rove.pruning.v1",
  });

  await expect(panel).toContainText("整条未送出的消息");
  await expect(panel).toContainText("9 条 · 30.0 KiB");
  await expect(panel).not.toContainText("修剪的工具结果");
  await expect(panel).not.toContainText("结果标识");
  await expect(panel).not.toContainText("0 B");
  await expect(panel).toContainText("仅这次压缩请求");
});
