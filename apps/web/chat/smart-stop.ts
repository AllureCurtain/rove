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

/**
 * The runtime's partial-abort marker, as of runtime document R2b.
 *
 * R2b now emits the fact: a stop that cut a turn short after the model had
 * already produced content ends with an `llm_message` event carrying
 * `aborted: true`, and the transcript renders it as a `(aborted)` suffix beside
 * the partial text. This smart-stop branch is a different question — whether to
 * show a stop notice *instead of* restoring the message — and the stop handler
 * runs while the salvage window is still open, before that terminal message
 * exists. Deriving it from the settled run is the registered F6 follow-up, so
 * the marker stays `undefined` and the branch below is never taken: a stop that
 * produced work keeps behaving exactly as it does today.
 */
export const PARTIAL_ABORT_MARKER: boolean | undefined = undefined;

/**
 * Whether a stop was a partial abort: the run carries the R2b marker *and* the
 * cancelled turn had already produced content. Reserved for the copy branch in
 * design F6 — with no marker this is always false.
 */
export function isPartialAbort(input: {
  aborted: boolean | undefined;
  producedWork: boolean;
}): boolean {
  return input.aborted === true && input.producedWork;
}
