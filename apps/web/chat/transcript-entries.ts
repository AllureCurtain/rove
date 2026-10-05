import type { ChatMessage, ToolCallView, TranscriptInputView, TranscriptTimelineItem } from "../lib/rove-state";

/**
 * One rendered row of a transcript.
 *
 * A run is a flat list of messages, tool calls and inputs. Rendering that
 * list directly splits one answer into as many bubbles as there were tool
 * calls. This projection folds consecutive tools and inputs into a single
 * activity, so the answer reads as one turn.
 */

export type TranscriptEntry =
  | { kind: "turn"; id: string; message: ChatMessage }
  | { kind: "activity"; id: string; tools: ToolCallView[]; inputs: TranscriptInputView[] }
  | { kind: "answer"; id: string; message: ChatMessage; streaming: boolean };

export function buildTranscriptEntries(items: TranscriptTimelineItem[]): TranscriptEntry[] {
  const entries: TranscriptEntry[] = [];
  let activity: Extract<TranscriptEntry, { kind: "activity" }> | null = null;

  const flush = () => {
    if (activity && (activity.tools.length > 0 || activity.inputs.length > 0)) {
      entries.push(activity);
    }
    activity = null;
  };

  items.forEach((item, index) => {
    if (item.kind === "message" && item.message.role === "user") {
      flush();
      entries.push({ kind: "turn", id: item.entry.id, message: item.message });
      return;
    }
    if (item.kind === "message") {
      flush();
      const last = index === items.length - 1;
      entries.push({
        kind: "answer",
        id: item.entry.id,
        message: item.message,
        streaming: last && item.message.status === "streaming",
      });
      return;
    }
    if (!activity) {
      activity = { kind: "activity", id: item.entry.id, tools: [], inputs: [] };
    }
    if (item.kind === "tool") {
      activity.tools.push(item.tool);
    } else {
      activity.inputs.push(item.input);
    }
  });
  flush();
  return entries;
}

/** An activity wants to be open while any of its work is still in flight. */
export function activityWantsOpen(entry: Extract<TranscriptEntry, { kind: "activity" }>): boolean {
  return (
    entry.tools.some((tool) => tool.status === "running" || tool.status === "waiting") ||
    entry.inputs.some((input) => input.status === "waiting")
  );
}
