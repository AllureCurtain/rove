import type { ChatMessage, TranscriptTimelineItem } from "../lib/rove-state";

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

/**
 * Whether an `aborted` message was a partial abort rather than a complete
 * answer: the runtime's marker *and* content the model had already produced.
 */
export function isPartialAbort(input: {
  aborted: boolean | undefined;
  producedWork: boolean;
}): boolean {
  return input.aborted === true && input.producedWork;
}

/**
 * The boundary a stop takes: the identity of every message that had already
 * settled when the stop was pressed.
 *
 * R2b's `aborted` marker is a fact about *this* stop, not about the session.
 * A restored transcript may already carry an old salvaged partial from an
 * earlier cancelled turn, so "the session contains an `aborted` message" would
 * pop the notice on a refresh or a session switch. Comparing against the
 * settled identities at the moment of the stop is what binds the marker to the
 * run the user actually cancelled.
 */
export function stopBoundary(messages: readonly ChatMessage[]): ReadonlySet<string> {
  return new Set(messages.map((message) => message.id));
}

/**
 * The partial this stop salvaged, or null when it kept nothing.
 *
 * This is the single judgment behind the composer notice: only a message that
 * arrived *after* the boundary and carries both the marker and text counts, so
 * a stop that produced nothing keeps behaving exactly as it did before R2b.
 */
export function salvagedPartialAfter(
  messages: readonly ChatMessage[],
  settledBefore: ReadonlySet<string>,
): ChatMessage | null {
  for (const message of messages) {
    if (settledBefore.has(message.id)) {
      continue;
    }
    if (message.role !== "assistant") {
      continue;
    }
    if (
      isPartialAbort({
        aborted: message.aborted,
        producedWork: message.content.length > 0,
      })
    ) {
      return message;
    }
  }
  return null;
}
