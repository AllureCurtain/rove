import type {
  ChatMessage,
  SubagentView,
  ToolCallView,
} from "../lib/rove-state";

/**
 * One row of a delegate's mini transcript, in the transcript's own language:
 * the delegated prompt opens the turn as a user row, then the child's own
 * tool calls and assistant messages follow.
 */
export type SubagentTranscriptRow =
  | { kind: "tool"; tool: ToolCallView }
  | { kind: "message"; message: ChatMessage };

/**
 * One user/assistant exchange a delegate ran. `prompt` is the `prompt` arg the
 * parent `task` call carried — it renders as the turn's user row, exactly as a
 * prompt the user typed would.
 */
export interface SubagentTranscriptTurn {
  prompt: string;
  agent?: string;
  rows: SubagentTranscriptRow[];
}

export interface SubagentTranscript {
  turns: SubagentTranscriptTurn[];
}

/** The `task` call args a delegation was opened with, or nothing usable. */
export function taskCallArgs(
  tool: ToolCallView | undefined,
): { prompt: string; agent?: string } {
  const args = tool?.args;
  if (!args || typeof args !== "object" || Array.isArray(args)) {
    return { prompt: "" };
  }
  const prompt = (args as { prompt?: unknown }).prompt;
  const agent = (args as { agent?: unknown }).agent;
  return {
    prompt: typeof prompt === "string" ? prompt : "",
    ...(typeof agent === "string" && agent ? { agent } : {}),
  };
}

/**
 * The wrapped-event order a child message was created at. Child message ids
 * embed the `eventCount` at creation (`message:sub:<call>:assistant:<n>`), the
 * only ordering the split view retains.
 */
export function subagentMessageOrder(message: ChatMessage): number {
  const suffix = message.id.match(/:(\d+)$/u);
  const parsed = suffix ? Number(suffix[1]) : Number.NaN;
  return Number.isSafeInteger(parsed) ? parsed : Number.MAX_SAFE_INTEGER;
}

/**
 * One delegated child's conversation as a single turn.
 *
 * The parent's `task` prompt opens the turn as a user row; the rows after it
 * are the child's own. The projection keeps the child's messages and tool
 * calls in two separately arrival-ordered lists — `tools` is newest-first, so
 * it is reversed here — and the wrapped events between them are gone, so the
 * merge preserves each list's exact order and interleaves by the message ids'
 * embedded event count: a tool sorts by its arrival rank among the tools, a
 * message by its recorded event ordinal, so the common shape (streaming text
 * around a handful of calls) reads in the order it happened.
 *
 * Returns `null` when there is nothing the delegate produced yet — the caller
 * shows its own empty state rather than a blank conversation.
 */
export function buildSubagentTranscript(
  subagent: SubagentView | undefined,
  parentTool: ToolCallView | undefined,
): SubagentTranscript | null {
  if (!subagent) {
    return null;
  }
  const { prompt, agent } = taskCallArgs(parentTool);
  const tools = [...subagent.tools].reverse();
  const rows: SubagentTranscriptRow[] = [
    ...subagent.messages.map(
      (message): { order: number; row: SubagentTranscriptRow } => ({
        order: subagentMessageOrder(message),
        row: { kind: "message", message },
      }),
    ),
    ...tools.map((tool, index): { order: number; row: SubagentTranscriptRow } => ({
      // A tool's wrapped-event ordinal is not retained; its arrival rank among
      // the tools is the honest sort key the projection can offer.
      order: index + 0.5,
      row: { kind: "tool", tool },
    })),
  ]
    .sort((a, b) => a.order - b.order)
    .map((entry) => entry.row);
  return {
    turns: [
      {
        prompt,
        ...(agent ? { agent } : {}),
        rows,
      },
    ],
  };
}
