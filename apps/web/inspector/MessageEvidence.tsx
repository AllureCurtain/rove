"use client";

import { useCopy } from "../copy/CopyProvider";
import type { ChatMessage } from "../lib/rove-state";
import { formatPruningBytes, promptPruningSummary } from "../chat/prompt-pruning";

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

/**
 * §8 evidence surface: per-turn usage, context estimate, compaction, and
 * pruning facts. The transcript stays clean prose; the measured detail lives
 * here, in the work panel.
 */
export function MessageEvidence({ message }: { message: ChatMessage }) {
  const { t } = useCopy();
  const pruning = promptPruningSummary(message.promptBuild?.pruning);
  if (
    !message.usage &&
    !message.promptBuild &&
    !message.promptCompaction &&
    !pruning
  ) {
    return null;
  }
  return (
    <dl className="message-evidence" aria-label={t("inspector.usageEvidence")}>
      {message.usage ? (
        <>
          <div><dt>{t("inspector.total")}</dt><dd>{formatNumber(message.usage.total_tokens)}</dd></div>
          <div><dt>{t("inspector.prompt")}</dt><dd>{formatNumber(message.usage.prompt_tokens)}</dd></div>
          <div><dt>{t("inspector.completion")}</dt><dd>{formatNumber(message.usage.completion_tokens)}</dd></div>
          {message.usage.cached_tokens !== undefined ? (
            <div><dt>{t("inspector.cached")}</dt><dd>{formatNumber(message.usage.cached_tokens)}</dd></div>
          ) : null}
        </>
      ) : null}
      {message.promptBuild ? (
        <div>
          <dt>{t("inspector.contextEstimate")}</dt>
          <dd>{formatNumber(message.promptBuild.token_estimate)}</dd>
        </div>
      ) : null}
      {message.promptCompaction ? (
        <div><dt>{t("inspector.compacted")}</dt><dd>{message.promptCompaction.mode.replaceAll("_", " ")}</dd></div>
      ) : null}
      {/* The pruning facts belong to the compaction request this turn made, so
          they sit next to the compaction row and say so. A build that pruned
          nothing carries no fields at all and renders nothing here. */}
      {pruning ? (
        <>
          {/* Rows are per fact class, not per object: a request that only left
              out whole messages carries no tool-elision count at all, and a row
              reading "0 条" would report a measurement the server never made.
              Each row is therefore gated on its own count — the byte widths are
              the sizes of those counted facts, so they never justify a row on
              their own. */}
          {pruning.elidedToolResults > 0 ? (
            <div>
              <dt>{t("inspector.prunedTools")}</dt>
              <dd>
                {t("inspector.prunedToolsValue", {
                  count: formatNumber(pruning.elidedToolResults),
                  size: formatPruningBytes(pruning.elidedBytes),
                })}
              </dd>
            </div>
          ) : null}
          {pruning.omittedMessages > 0 ? (
            <div>
              <dt>{t("inspector.prunedOmitted")}</dt>
              <dd>
                {t("inspector.prunedOmittedValue", {
                  count: formatNumber(pruning.omittedMessages),
                  size: formatPruningBytes(pruning.omittedBytes),
                })}
              </dd>
            </div>
          ) : null}
          {pruning.policy !== null ? (
            <div>
              <dt>{t("inspector.pruningPolicy")}</dt>
              <dd>
                {pruning.policyKnown
                  ? pruning.policy
                  : t("inspector.pruningPolicyUnknown")}
              </dd>
            </div>
          ) : null}
          {pruning.digests.length > 0 ? (
            <div>
              <dt>{t("inspector.pruningDigests")}</dt>
              <dd>
                {/* Identities, not links: they correlate with the trace and
                    open nothing. Collapsed because eight hashes are noise to
                    a reader who is not chasing one. */}
                <details className="message-evidence__digests">
                  <summary>
                    {t("inspector.pruningDigestsValue", {
                      count: formatNumber(pruning.digests.length),
                    })}
                  </summary>
                  <ul>
                    {pruning.digests.map((digest) => (
                      <li key={digest}>
                        <code>{digest}</code>
                      </li>
                    ))}
                  </ul>
                </details>
              </dd>
            </div>
          ) : null}
          <div>
            <dt>{t("inspector.pruningScope")}</dt>
            <dd>{t("inspector.pruningScopeValue")}</dd>
          </div>
        </>
      ) : null}
    </dl>
  );
}
