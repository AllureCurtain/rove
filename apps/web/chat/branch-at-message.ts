import type { ProductMessage } from "../product/product-api-types";

/**
 * The ledger message a transcript turn was triggered by, or null when the
 * match is not unique (R6).
 *
 * Edit-and-resend sends the *ledger* `seq` of the user message being replaced;
 * the transcript itself is a projection of canonical events and carries no
 * ledger identity. The ledger publishes both `content` and the run the message
 * was delivered to, so the pair is what identifies one turn's trigger message
 * without inventing an identity. An ambiguous or absent match returns null and
 * the caller refuses the branch instead of guessing a seq — forking from the
 * wrong message would silently cut a different prefix.
 */
export function ledgerMessageForTurn(input: {
  messages: readonly ProductMessage[];
  content: string;
  runId: string | null;
}): ProductMessage | null {
  if (!input.runId || input.content.length === 0) {
    return null;
  }
  const matches = input.messages.filter(
    (message) =>
      message.run_id === input.runId && message.content === input.content,
  );
  return matches.length === 1 ? matches[0]! : null;
}

/**
 * Whether the branch affordance can be offered for one turn.
 *
 * Only the session's latest actionable turn is a valid truncation point in
 * practice: the server additionally requires the parent to be idle and the run
 * to be terminal, and it refuses anything else with a typed error. The client
 * checks the two facts it owns — an open fork target and a unique ledger match
 * — so the action is never a fake entrance.
 */
export function branchPointForTurn(input: {
  messages: readonly ProductMessage[];
  content: string;
  runId: string | null;
  forkAvailable: boolean;
}): ProductMessage | null {
  if (!input.forkAvailable) {
    return null;
  }
  return ledgerMessageForTurn(input);
}
