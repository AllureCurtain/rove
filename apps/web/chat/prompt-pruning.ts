import type { PromptPruningFacts } from "../lib/rove-types";
import { formatAttachmentBytes } from "./composer-files";

/**
 * What one turn's bounded compaction request left out, in the reader's terms (R9).
 *
 * The runtime records these facts on the `PromptBuilt` metadata of the turn that
 * ran the probe, and they describe **that request**, not the turn's own prompt:
 * a pruned shrink request means the summarizer saw an excerpt and a statement of
 * what was missing, while the conversation itself kept every byte. The panel
 * therefore says what was left out of the probe's material and never implies the
 * transcript lost anything.
 *
 * Every wire field is omitted while it is zero or empty, so the projection fills
 * the gaps with zeroes and returns `null` when the facts add up to "nothing was
 * pruned at all" — the case the evidence panel must not render, because an empty
 * pruning row would be a claim the server never made.
 */
export interface PromptPruningSummary {
  /** Oversized tool payloads the request carried as a bounded excerpt. */
  elidedToolResults: number;
  /**
   * The bytes the request did not carry: what those payloads occupied before
   * the projection minus what the excerpt occupies. Clamped at zero because a
   * negative "loss" would be a server contradiction, not a number to show.
   */
  elidedBytes: number;
  /** Older messages dropped whole so the request stayed inside its bounds. */
  omittedMessages: number;
  /** What those omitted messages occupied. */
  omittedBytes: number;
  /** The rule the server named, or null when it named none. */
  policy: string | null;
  /**
   * Whether this build can name the rule. An unknown rule is shown as unknown
   * rather than rejected: the server is allowed to revise the policy, and a
   * version this build has never heard of is still a real pruning event.
   */
  policyKnown: boolean;
  /** Newest-first identities of the elided payloads. Identity, not a link. */
  digests: string[];
}

/**
 * The pruning rule this build can name.
 *
 * Kept as a list so a second known rule is one entry, not a new branch: the
 * version string is the server's own identifier and is never translated. The one
 * entry mirrors `PRUNING_POLICY_VERSION` in `runtime/src/context/compaction.rs`;
 * a rename there would show up as "unknown rule" here rather than as a wrong
 * name, which is the safe direction for a display that must not vouch for a rule
 * it does not know.
 */
export const KNOWN_PRUNING_POLICIES: readonly string[] = ["rove.pruning.v1"];

/**
 * The byte formatter is the shell's only one.
 *
 * Pruned payload sizes are byte sizes, so they render through the same function
 * the attachment rows use; a second formatter would let the two drift apart.
 */
export const formatPruningBytes = formatAttachmentBytes;

/**
 * Project one turn's pruning facts, as either shell path delivers them.
 *
 * `pruning` is read as possibly-absent members on purpose. Both arrival paths
 * now decode the wire record — a restored transcript through
 * `product/product-api-types.ts`, a live turn through
 * `lib/rove-state.ts::readPromptBuildMetadata` — so a member is absent only when
 * the server reported nothing for it: the runtime omits every member that is
 * zero or empty (`runtime/src/context/prompt_metadata.rs` uses
 * `skip_serializing_if` on all seven), and a probe that only dropped older
 * messages therefore carries three of them.
 *
 * Absence means the server reported nothing for that member, so it is defaulted
 * here and never rendered as a measurement. This function must not hand back an
 * absent member: the panel reads `digests.length` and the counts arithmetically.
 * The tolerance is a belt, not the mechanism: it is what keeps an older producer
 * or a hand-built fixture from rendering a missing fact as a measured zero.
 */
export function promptPruningSummary(
  pruning: Partial<PromptPruningFacts> | undefined,
): PromptPruningSummary | null {
  if (!pruning) {
    return null;
  }
  const elidedToolResults = pruning.pruned_tool_results ?? 0;
  const omittedMessages = pruning.pruned_omitted_messages ?? 0;
  const digests = pruning.pruned_payload_digests ?? [];
  const policy = pruning.pruning_policy ?? null;
  if (
    elidedToolResults === 0 &&
    omittedMessages === 0 &&
    digests.length === 0 &&
    policy === null
  ) {
    return null;
  }
  return {
    elidedToolResults,
    elidedBytes: Math.max(
      0,
      (pruning.pruned_payload_bytes ?? 0) - (pruning.pruned_excerpt_bytes ?? 0),
    ),
    omittedMessages,
    omittedBytes: pruning.pruned_omitted_bytes ?? 0,
    policy,
    policyKnown: policy !== null && KNOWN_PRUNING_POLICIES.includes(policy),
    digests,
  };
}
