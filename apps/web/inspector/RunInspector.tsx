"use client";

import { ChevronLeftIcon, ChevronRightIcon, Cross2Icon } from "@radix-ui/react-icons";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import type { ToolCallView, WorkbenchState } from "../lib/rove-state";
import type { TranscriptRestoreState } from "../state/transcript-projection";
import type { SessionUsageState } from "../state/use-session-usage";
import { ArtifactPanel } from "./ArtifactPanel";
import { DiffPanel } from "./DiffPanel";
import { ExportPanel } from "./ExportPanel";
import { FilesPanel } from "./FilesPanel";
import { ReviewPanel } from "./ReviewPanel";
import type {
  ProductReview,
  ProductReviewFindingPageItem,
} from "../product/product-api-types";

export function RunInspector({
  productSessionId,
  workspaceId,
  collapsed,
  onToggle,
  runState,
  restoreState,
  sessionUsage,
  dialogOpen = false,
  reviews = [],
  selectedReviewId = null,
  selectedReview = null,
  reviewFindings = [],
  reviewFindingsCursor = null,
  reviewFindingsLoading = false,
  reviewsLoading = false,
  reviewError = null,
  onSelectReview,
  onRefreshReviews,
  onCancelReview,
  onLoadReviewFindings,
  onOpenReviewFinding,
  fileFocusPath,
  fileFocusLine,
}: {
  productSessionId: string;
  workspaceId?: string;
  collapsed: boolean;
  onToggle: () => void;
  runState: WorkbenchState;
  restoreState?: TranscriptRestoreState;
  sessionUsage?: SessionUsageState;
  dialogOpen?: boolean;
  reviews?: ProductReview[];
  selectedReviewId?: string | null;
  selectedReview?: ProductReview | null;
  reviewFindings?: ProductReviewFindingPageItem[];
  reviewFindingsCursor?: number | null;
  reviewFindingsLoading?: boolean;
  reviewsLoading?: boolean;
  reviewError?: string | null;
  onSelectReview?: (reviewId: string) => void;
  onRefreshReviews?: () => void;
  onCancelReview?: (reviewId: string) => void;
  onLoadReviewFindings?: (reviewId: string, cursor?: number) => void;
  onOpenReviewFinding?: (path: string, line: number) => void;
  fileFocusPath?: string | null;
  fileFocusLine?: number | null;
}) {
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const [tab, setTab] = useState<"run" | "review">("run");

  useEffect(() => {
    if (dialogOpen) {
      closeButtonRef.current?.focus();
    }
  }, [dialogOpen]);

  const activeReviewId = reviews.find(
    (review) => review.status === "queued" || review.status === "running",
  )?.id;
  useEffect(() => {
    if (activeReviewId) {
      setTab("review");
    }
  }, [activeReviewId]);

  if (collapsed) {
    return (
      <aside className="product-inspector" data-collapsed="true" aria-label="Run inspector">
        <div className="inspector-header">
          <button
            type="button"
            className="ghost icon-button"
            onClick={onToggle}
            aria-label="Expand inspector"
          >
            <ChevronLeftIcon />
          </button>
        </div>
      </aside>
    );
  }

  const phase = resolveInspectorPhase(runState);
  const waiting = runState.tools.filter((tool) => tool.pendingApproval);
  const tools = runState.tools.slice(0, 12);
  const mutations = runState.tools.flatMap((tool) =>
    (tool.mutations ?? []).map((mutation) => ({ tool, mutation })),
  );
  const evidenceRefs = uniqueEvidenceRefs(runState);

  return (
    <aside
      className="product-inspector"
      aria-label="Run inspector"
      data-phase={phase}
      data-open={dialogOpen}
      aria-modal={dialogOpen ? true : undefined}
      role={dialogOpen ? "dialog" : undefined}
      onKeyDown={
        dialogOpen
          ? (event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                onToggle();
                return;
              }
              trapFocus(event);
            }
          : undefined
      }
    >
      <div className="inspector-header">
        <h2>Inspector</h2>
        <button
          ref={closeButtonRef}
          type="button"
          className="ghost icon-button"
          onClick={onToggle}
          aria-label={dialogOpen ? "Close run evidence" : "Collapse inspector"}
        >
          {dialogOpen ? <Cross2Icon /> : <ChevronRightIcon />}
        </button>
      </div>
      <div className="inspector-tabs" role="tablist" aria-label="Inspector views">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "run"}
          className={tab === "run" ? "tab-button tab-button--active" : "tab-button"}
          onClick={() => setTab("run")}
        >
          Run
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "review"}
          className={tab === "review" ? "tab-button tab-button--active" : "tab-button"}
          onClick={() => setTab("review")}
        >
          Review{reviews.length > 0 ? ` (${reviews.length})` : ""}
        </button>
      </div>
      <div className="inspector-body">
        {tab === "review" ? (
          <ReviewPanel
            reviews={reviews}
            selectedReviewId={selectedReviewId}
            selectedReview={selectedReview}
            findings={reviewFindings}
            findingsCursor={reviewFindingsCursor}
            findingsLoading={reviewFindingsLoading}
            loading={reviewsLoading}
            error={reviewError}
            onSelect={onSelectReview ?? (() => undefined)}
            onRefresh={onRefreshReviews ?? (() => undefined)}
            onCancel={onCancelReview ?? (() => undefined)}
            onLoadFindings={onLoadReviewFindings ?? (() => undefined)}
            onOpenFinding={(path, line) => {
              setTab("run");
              onOpenReviewFinding?.(path, line);
            }}
          />
        ) : (
          <>
        <ExportPanel sessionId={productSessionId} />
        {phase === "empty" ? (
          <div className="inspector-state" data-tone="empty" role="status">
            <strong>No active run</strong>
            <p>
              Send a message to start a turn. Plan, tools, and approvals for the latest run will
              appear here.
            </p>
          </div>
        ) : null}

        {phase === "loading" ? (
          <div className="inspector-state" data-tone="loading" role="status" aria-live="polite">
            <strong>{runState.statusText || "Working…"}</strong>
            <div className="inspector-skeleton" aria-hidden="true">
              <span />
              <span />
              <span />
            </div>
            <p>Streaming run events. Approvals and tools update as the agent works.</p>
          </div>
        ) : null}

        {phase === "error" ? (
          <div className="inspector-state" data-tone="error" role="alert">
            <strong>Run interrupted</strong>
            <p>{runState.error}</p>
          </div>
        ) : null}

        {phase !== "empty" ? (
          <>
            <section className="inspector-section">
              <div className="inspector-section__heading">
                <h3>Continuity</h3>
                <span data-tone={continuityTone(runState)}>{continuityLabel(runState)}</span>
              </div>
              <div className="inspector-kv">
                <div>
                  <span>product session</span>
                  <strong>{productSessionId}</strong>
                </div>
                <div>
                  <span>status</span>
                  <strong>{runState.statusText}</strong>
                </div>
                <div>
                  <span>turn ordinal</span>
                  <strong>{runState.activeRunOrdinal ?? "Not available"}</strong>
                </div>
                <div>
                  <span>job</span>
                  <strong>{identity(runState.activeJobId)}</strong>
                </div>
                <div>
                  <span>run</span>
                  <strong>{identity(runState.activeRunId)}</strong>
                </div>
                <div>
                  <span>resumed from</span>
                  <strong>{identity(runState.resumedFromRunId)}</strong>
                </div>
                <div>
                  <span>events</span>
                  <strong>{runState.eventCount}</strong>
                </div>
                <div>
                  <span>signal</span>
                  <strong>{runState.lastSignal}</strong>
                </div>
                <div>
                  <span>history</span>
                  <strong>{restoreStatusLabel(restoreState)}</strong>
                </div>
              </div>
            </section>

            <section className="inspector-section">
              <h3>Usage &amp; context</h3>
              <div className="inspector-kv inspector-kv--metrics">
                <div><span>prompt</span><strong>{formatNumber(runState.runUsage.prompt_tokens)}</strong></div>
                <div><span>completion</span><strong>{formatNumber(runState.runUsage.completion_tokens)}</strong></div>
                <div><span>cached</span><strong>{formatNumber(runState.runUsage.cached_tokens ?? 0)}</strong></div>
                <div><span>total</span><strong>{formatNumber(runState.runUsage.total_tokens)}</strong></div>
                <div>
                  <span>session total</span>
                  <strong>
                    {sessionUsage?.status === "ready"
                      ? formatNumber(sessionUsage.data.totals.total_tokens)
                      : sessionUsage?.status === "loading"
                        ? "Loading…"
                        : "Not loaded"}
                  </strong>
                </div>
                <div>
                  <span>context estimate</span>
                  <strong>
                    {sessionUsage?.status === "ready" &&
                    sessionUsage.data.latest_context
                      ? formatContextOccupancy(sessionUsage.data.latest_context)
                      : runState.promptBuild
                        ? `${formatNumber(runState.promptBuild.token_estimate)} tokens`
                        : "Not emitted"}
                  </strong>
                </div>
                <div>
                  <span>cost</span>
                  <strong>{formatSessionCost(sessionUsage)}</strong>
                </div>
              </div>
              {runState.promptBuild ? (
                <dl className="inspector-facts">
                  <div><dt>History</dt><dd>{runState.promptBuild.included_history_messages} included / {runState.promptBuild.dropped_history_messages} dropped</dd></div>
                  <div><dt>Prompt hash</dt><dd><code>{shortId(runState.promptBuild.prompt_hash)}</code></dd></div>
                  <div><dt>Cache key</dt><dd><code>{shortId(runState.promptBuild.prompt_cache_key)}</code></dd></div>
                </dl>
              ) : null}
              {runState.promptCompaction ? (
                <p className="inspector-empty-line">
                  Compaction: {runState.promptCompaction.mode.replaceAll("_", " ")}
                  {runState.promptCompaction.degraded ? ", degraded fallback used" : ""}.
                </p>
              ) : null}
              {sessionUsage?.status === "ready" &&
              sessionUsage.data.latest_context?.compaction_mode ? (
                <p className="inspector-empty-line">
                  Durable compaction: {sessionUsage.data.latest_context.compaction_mode.replaceAll("_", " ")}
                  {sessionUsage.data.latest_context.compaction_auto_triggered ? ", automatic" : ""}
                  {sessionUsage.data.latest_context.compaction_degraded ? ", degraded" : ""}; {formatNumber(sessionUsage.data.latest_context.compacted_history_messages)} messages compacted
                  {sessionUsage.data.latest_context.compaction_source_messages > 0
                    ? ` from ${formatNumber(sessionUsage.data.latest_context.compaction_source_messages)}`
                    : ""}.
                </p>
              ) : null}
              {sessionUsage?.status === "ready" &&
              sessionUsage.data.partial_reasons.length > 0 ? (
                <p className="inspector-empty-line">
                  Partial usage: {sessionUsage.data.partial_reasons[0]}
                  {sessionUsage.data.partial_reasons.length > 1
                    ? ` (+${sessionUsage.data.partial_reasons.length - 1} more)`
                    : ""}
                </p>
              ) : null}
              {sessionUsage?.status === "error" ? (
                <p className="inspector-empty-line">Session usage unavailable: {sessionUsage.message}</p>
              ) : null}
              <p className="inspector-footnote">
                Cost uses the server pricing snapshot frozen per run. Unpriced models stay unavailable; local fake models report explicit zero cost.
              </p>
            </section>

            {workspaceId ? (
              <FilesPanel
                workspaceId={workspaceId}
                focusPath={fileFocusPath}
                focusLine={fileFocusLine}
              />
            ) : null}
            <ArtifactPanel sessionId={productSessionId} />
            <DiffPanel sessionId={productSessionId} />
            <section className="inspector-section">
              <h3>Plan</h3>
              {runState.plan ? (
                <ul className="plan-list">
                  {runState.plan.steps.map((step) => (
                    <li key={step.id} data-done={step.done}>
                      {step.done ? "✓ " : "• "}
                      {step.title}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="inspector-empty-line">
                  {phase === "loading" ? "Waiting for plan…" : "No plan for this run."}
                </p>
              )}
            </section>

            <section className="inspector-section">
              <h3>Approvals</h3>
              {waiting.length === 0 ? (
                <p className="inspector-empty-line">
                  {phase === "loading" ? "No approvals yet." : "None pending."}
                </p>
              ) : (
                <ul className="tool-list">
                  {waiting.map((tool) => (
                    <li key={tool.id} data-tone="waiting">
                      <strong>{tool.name}</strong>
                      <div>{tool.reason ?? tool.details}</div>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="inspector-section">
              <h3>Tools</h3>
              {tools.length === 0 ? (
                <p className="inspector-empty-line">
                  {phase === "loading" ? "Waiting for tool calls…" : "No tool calls yet."}
                </p>
              ) : (
                <ul className="tool-list">
                  {tools.map((tool: ToolCallView) => (
                    <li key={tool.id} data-status={tool.status}>
                      <strong>
                        {tool.name} · {tool.status}
                      </strong>
                      <div className="tool-list__detail">
                        {tool.metadata
                          ? `${tool.metadata.read_only ? "read only" : "mutation capable"}; ${tool.metadata.risk_level} risk`
                          : truncate(tool.details, 120)}
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="inspector-section">
              <h3>Workspace changes</h3>
              {mutations.length === 0 ? (
                <p className="inspector-empty-line">No canonical mutations for this run.</p>
              ) : (
                <ul className="mutation-list">
                  {mutations.map(({ tool, mutation }, index) => (
                    <li key={`${tool.id}-${mutation.path}-${index}`}>
                      <strong>{mutation.path}</strong>
                      <span>{mutation.operation}{mutation.diff ? "; Diff available in timeline" : "; no inline Diff"}</span>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="inspector-section">
              <h3>Evidence references</h3>
              {evidenceRefs.length === 0 ? (
                <p className="inspector-empty-line">No opaque evidence or artifact references were emitted.</p>
              ) : (
                <ul className="evidence-ref-list">
                  {evidenceRefs.map((reference) => (
                    <li key={`${reference.kind}:${reference.value}`}>
                      <span>{reference.kind}</span>
                      <code>{reference.value}</code>
                    </li>
                  ))}
                </ul>
              )}
              <p className="inspector-footnote">Opaque references resolve only through their bound product session.</p>
            </section>

            <section className="inspector-section">
              <h3>Canonical events</h3>
              {runState.trace.length === 0 ? (
                <p className="inspector-empty-line">No projected events yet.</p>
              ) : (
                <ol className="canonical-event-list">
                  {runState.trace.slice(0, 24).map((entry) => (
                    <li key={entry.id}>
                      <code>{entry.label}</code>
                      <span>{truncate(entry.detail, 120)}</span>
                    </li>
                  ))}
                </ol>
              )}
            </section>
          </>
        ) : null}
          </>
        )}
      </div>
    </aside>
  );
}

function trapFocus(event: KeyboardEvent<HTMLElement>) {
  if (event.key !== "Tab") {
    return;
  }
  const focusable = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((element) => element.getClientRects().length > 0);
  const first = focusable[0];
  const last = focusable.at(-1);
  if (!first || !last) {
    return;
  }
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}

export type InspectorPhase = "empty" | "loading" | "error" | "ready";

export function resolveInspectorPhase(runState: WorkbenchState): InspectorPhase {
  if (runState.error) {
    return "error";
  }
  if (runState.busy) {
    return "loading";
  }
  const hasRunIdentity = Boolean(runState.activeJobId || runState.activeRunId);
  const hasRunContent =
    Boolean(runState.plan) ||
    runState.tools.length > 0 ||
    runState.eventCount > 0 ||
    runState.messages.length > 0;
  if (!hasRunIdentity && !hasRunContent) {
    return "empty";
  }
  return "ready";
}

function shortId(value: string | null | undefined): string {
  return value ? value.slice(0, 12) : "Not available";
}

function identity(value: string | null): string {
  return value ?? "Not available";
}

function uniqueEvidenceRefs(runState: WorkbenchState): Array<{
  kind: "artifact" | "evidence";
  value: string;
}> {
  const seen = new Set<string>();
  const references: Array<{ kind: "artifact" | "evidence"; value: string }> = [];
  for (const record of runState.stepRecords) {
    for (const [kind, values] of [
      ["artifact", record.artifact_refs ?? []],
      ["evidence", record.evidence_refs ?? []],
    ] as const) {
      for (const value of values) {
        const key = `${kind}:${value}`;
        if (!seen.has(key)) {
          seen.add(key);
          references.push({ kind, value });
        }
      }
    }
  }
  return references;
}

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max)}…`;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

function continuityLabel(runState: WorkbenchState): string {
  if (runState.error) {
    return "Needs attention";
  }
  if (runState.busy) {
    return "Observed live";
  }
  return runState.activeRunId ? "Durable identity" : "No run";
}

function continuityTone(runState: WorkbenchState): string {
  return runState.error ? "error" : runState.busy ? "working" : "ok";
}

function restoreStatusLabel(state: TranscriptRestoreState | undefined): string {
  if (!state) {
    return "Not available";
  }
  switch (state.status) {
    case "idle":
      return "Not restored";
    case "loading":
      return "Restoring";
    case "complete":
      return "Complete";
    case "partial":
      return `Partial (${state.reasons.length})`;
    case "error":
      return "Restore failed";
  }
}

function formatSessionCost(sessionUsage: SessionUsageState | undefined): string {
  if (!sessionUsage || sessionUsage.status === "idle") {
    return "Not loaded";
  }
  if (sessionUsage.status === "loading") {
    return "Loading…";
  }
  if (sessionUsage.status === "error") {
    return "Unavailable";
  }
  const cost = sessionUsage.data.totals_cost;
  if (!cost) {
    return "Unavailable";
  }
  if (cost.availability === "unpriced" || cost.total_usd === undefined || cost.total_usd === null) {
    return "Unavailable";
  }
  if (cost.availability === "local_zero") {
    return `${cost.currency} 0.00 (local)`;
  }
  const digits = cost.total_usd > 0 && cost.total_usd < 0.01 ? 6 : 2;
  return `${cost.currency} ${cost.total_usd.toFixed(digits)}`;
}

function formatContextOccupancy(context: {
  token_estimate: number;
  context_window?: number;
}): string {
  const used = formatNumber(context.token_estimate);
  if (!context.context_window) {
    return `${used} tokens / window unavailable`;
  }
  const percentage = Math.min(
    999,
    Math.round((context.token_estimate / context.context_window) * 100),
  );
  return `${used} / ${formatNumber(context.context_window)} (${percentage}%)`;
}
