import type { TranscriptTimelineItem } from "../lib/rove-state";

/**
 * Decide whether stopping should put the last message back into the composer.
 *
 * A stop restores the draft only when the model has produced nothing yet:
 * no assistant text, no tool call, no input request after the last user
 * message. Once any of those exist the turn has started and the message
 * stays in the transcript. Inherited history never qualifies, because it
 * belongs to the parent session.
 */
export function shouldRestoreOnStop(items: readonly TranscriptTimelineItem[]): boolean {
  return lastRestorableUserMessage(items) !== null;
}

/** The message a stop would restore, or null when the turn already produced work. */
export function lastRestorableUserMessage(
  items: readonly TranscriptTimelineItem[],
): Extract<TranscriptTimelineItem, { kind: "message" }> | null {
  let lastUserIndex = -1;
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index]!;
    if (item.kind === "message" && item.message.role === "user") {
      lastUserIndex = index;
      break;
    }
  }
  if (lastUserIndex < 0) {
    return null;
  }
  const lastUser = items[lastUserIndex]!;
  if (lastUser.kind !== "message" || lastUser.entry.inherited) {
    return null;
  }
  const producedWork = items
    .slice(lastUserIndex + 1)
    .some((item) => item.kind !== "message" || item.message.role !== "user");
  if (producedWork) {
    return null;
  }
  return lastUser;
}
