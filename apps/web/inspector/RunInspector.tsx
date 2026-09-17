"use client";

import { ChevronLeftIcon, ChevronRightIcon, Cross2Icon } from "@radix-ui/react-icons";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  isTerminalRunCanceled,
  type ToolCallView,
  type WorkbenchState,
} from "../lib/rove-state";
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
  const { t } = useCopy();
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

  const phase = resolveInspectorPhase(runState);
  const waiting = runState.tools.filter((tool) => tool.pendingApproval);
  const mutations = useMemo(
    () =>
      runState.tools.flatMap((tool) =>
        (tool.mutations ?? []).map((mutation) => ({ tool, mutation })),
      ),
    [runState.tools],
  );
  const timeline = useMemo(
    () => buildActivityTimeline(runState, waiting.length > 0, t),
    [runState, waiting.length, t],
  );

  if (collapsed) {
    return (
      <aside className="product-inspector" data-collapsed="true" aria-label={t("inspector.title")}>
        <div className="inspector-header">
          <button
            type="button"
            className="ghost icon-button"
            onClick={onToggle}
            aria-label={t("nav.expandInspector")}
          >
            <ChevronLeftIcon />
          </button>
        </div>
      </aside>
    );
  }

  return (
    <aside
      className="product-inspector"
      aria-label={t("inspector.title")}
      data-phase={phase}
      data-open={dialogOpen}
      aria-modal={dialogOpen ? true : undefined}
      role={dialogOpen ? "dialog" : undefined}
      onKeyDown={
        dialogOpen
          ? (event: KeyboardEvent<HTMLElement>) => {
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
        <h2>{t("inspector.title")}</h2>
        <button
          ref={closeButtonRef}
          type="button"
          className="ghost icon-button"
          onClick={onToggle}
          aria-label={dialogOpen ? t("nav.closeInspector") : t("nav.collapseInspector")}
        >
          {dialogOpen ? <Cross2Icon /> : <ChevronRightIcon />}
        </button>
      </div>
      <div className="inspector-tabs" role="tablist" aria-label={t("inspector.title")}>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "run"}
          className={tab === "run" ? "tab-button tab-button--active" : "tab-button"}
          onClick={() => setTab("run")}
        >
          {t("inspector.tabRun")}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "review"}
          className={tab === "review" ? "tab-button tab-button--active" : "tab-button"}
          onClick={() => setTab("review")}
        >
          {t("inspector.tabReview")}
          {reviews.length > 0 ? ` (${reviews.length})` : ""}
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
                <strong>{t("inspector.emptyTitle")}</strong>
                <p>{t("inspector.emptyBody")}</p>
              </div>
            ) : null}

            {phase === "loading" ? (
              <div
                className="inspector-state"
                data-tone="loading"
                role="status"
                aria-live="polite"
              >
                <strong>{t("inspector.working")}</strong>
                <div className="inspector-skeleton" aria-hidden="true">
                  <span />
                  <span />
                  <span />
                </div>
                <p>{t("inspector.streaming")}</p>
              </div>
            ) : null}

            {phase === "error" ? (
              <div className="inspector-state" data-tone="error" role="alert">
                <strong>{t("inspector.errorTitle")}</strong>
                <p>{runState.error}</p>
                <p className="error-code">
                  {t("errors.code", { code: "run-interrupted" })}
                </p>
              </div>
            ) : null}

            {phase !== "empty" ? (
              <>
                <section className="inspector-section">
                  <div className="inspector-section__heading">
                    <h3>{t("inspector.summaryTitle")}</h3>
                    <span data-tone={statusTone(runState)}>
                      {statusLabel(runState, waiting.length > 0, t)}
                    </span>
                  </div>
                  <div className="inspector-kv inspector-kv--metrics">
                    <div>
                      <span>{t("inspector.usage")}</span>
                      <strong>
                        {t("inspector.tokens", {
                          count: formatNumber(runState.runUsage.total_tokens),
                        })}
                      </strong>
                    </div>
                    <div>
                      <span>{t("inspector.cost")}</span>
                      <strong>{formatSessionCost(sessionUsage, t)}</strong>
                    </div>
                    <div>
                      <span>{t("inspector.sessionTotal")}</span>
                      <strong>
                        {sessionUsage?.status === "ready"
                          ? t("inspector.tokens", {
                              count: formatNumber(
                                sessionUsage.data.totals.total_tokens,
                              ),
                            })
                          : sessionUsage?.status === "loading"
                            ? t("common.loading")
                            : t("inspector.notLoaded")}
                      </strong>
                    </div>
                    {restoreState?.status === "partial" ||
                    restoreState?.status === "error" ? (
                      <div className="inspector-empty-line">
                        {t("inspector.restorePartial")}
                      </div>
                    ) : null}
                    {runState.promptCompaction?.degraded ? (
                      <div className="inspector-empty-line">
                        {t("inspector.compactedNotice")}
                      </div>
                    ) : null}
                  </div>
                </section>

                <section className="inspector-section">
                  <h3>{t("inspector.timeline")}</h3>
                  {timeline.length === 0 ? (
                    <p className="inspector-empty-line">{t("inspector.streaming")}</p>
                  ) : (
                    <ol className="activity-timeline">
                      {timeline.map((item) => (
                        <li key={item.id} data-tone={item.tone}>
                          <strong>{item.label}</strong>
                          {item.detail ? <div>{item.detail}</div> : null}
                        </li>
                      ))}
                    </ol>
                  )}
                </section>

                {workspaceId ? (
                  <FilesPanel
                    workspaceId={workspaceId}
                    focusPath={fileFocusPath}
                    focusLine={fileFocusLine}
                  />
                ) : null}

                <section className="inspector-section">
                  <h3>{t("inspector.filesTitle")}</h3>
                  {mutations.length === 0 ? (
                    <p className="inspector-empty-line">{t("inspector.filesNone")}</p>
                  ) : (
                    <ul className="mutation-list">
                      {mutations.map(({ tool, mutation }, index) => (
                        <li key={`${tool.id}-${mutation.path}-${index}`}>
                          <strong>{mutation.path}</strong>
                          <span>{mutation.operation}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </section>

                <ArtifactPanel sessionId={productSessionId} />
                <DiffPanel sessionId={productSessionId} />

                <section className="inspector-section">
                  <h3>{t("inspector.planTitle")}</h3>
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
                      {phase === "loading"
                        ? t("inspector.planWaiting")
                        : t("inspector.planNone")}
                    </p>
                  )}
                </section>

                <section className="inspector-section">
                  <h3>{t("inspector.approvalsTitle")}</h3>
                  {waiting.length === 0 ? (
                    <p className="inspector-empty-line">
                      {t("inspector.approvalsNone")}
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
                  <h3>{t("inspector.toolsTitle")}</h3>
                  {runState.tools.length === 0 ? (
                    <p className="inspector-empty-line">
                      {phase === "loading"
                        ? t("inspector.toolsWaiting")
                        : t("inspector.toolsNone")}
                    </p>
                  ) : (
                    <ul className="tool-list">
                      {runState.tools.slice(0, 12).map((tool: ToolCallView) => (
                        <li key={tool.id} data-status={tool.status}>
                          <strong>{tool.name}</strong>
                        </li>
                      ))}
                    </ul>
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

function statusTone(runState: WorkbenchState): string {
  if (runState.error) {
    return "error";
  }
  if (isTerminalRunCanceled(runState)) {
    return "canceled";
  }
  if (runState.busy) {
    return "working";
  }
  return "ok";
}

function statusLabel(
  runState: WorkbenchState,
  hasWaitingApproval: boolean,
  t: (path: string) => string,
): string {
  if (runState.error) {
    return t("inspector.statusFailed");
  }
  if (hasWaitingApproval) {
    return t("inspector.statusWaitingApproval");
  }
  if (runState.busy) {
    return t("inspector.statusRunning");
  }
  if (isTerminalRunCanceled(runState)) {
    return t("inspector.statusCanceled");
  }
  if (runState.activeRunId) {
    return t("inspector.statusCompleted");
  }
  return t("inspector.statusQueued");
}

type ActivityItem = {
  id: string;
  label: string;
  detail?: string;
  tone: "accent" | "default" | "ok" | "error";
};

function buildActivityTimeline(
  runState: WorkbenchState,
  hasWaitingApproval: boolean,
  t: (path: string, params?: Record<string, string | number>) => string,
): ActivityItem[] {
  if (!runState.activeRunId && runState.tools.length === 0 && !runState.plan) {
    return [];
  }
  const items: ActivityItem[] = [
    {
      id: "start",
      label: t("inspector.timelineStart"),
      tone: "accent",
    },
  ];

  const toolCount = runState.tools.length;
  if (toolCount > 0) {
    items.push({
      id: "tools",
      label: t("inspector.timelineTools", { count: toolCount }),
      detail: runState.tools
        .slice(0, 6)
        .map((tool) => tool.name)
        .join(", "),
      tone: "default",
    });
  }

  if (hasWaitingApproval) {
    items.push({
      id: "approval",
      label: t("inspector.timelineApproval"),
      tone: "accent",
    });
  }

  if (runState.error) {
    items.push({
      id: "failed",
      label: t("inspector.timelineFailed"),
      detail: runState.error,
      tone: "error",
    });
  } else if (isTerminalRunCanceled(runState)) {
    items.push({
      id: "canceled",
      label: t("inspector.timelineCanceled"),
      tone: "default",
    });
  } else if (!runState.busy && runState.activeRunId) {
    items.push({
      id: "complete",
      label: t("inspector.timelineComplete"),
      tone: "ok",
    });
  }

  return items;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

function formatSessionCost(
  sessionUsage: SessionUsageState | undefined,
  t: (path: string) => string,
): string {
  if (!sessionUsage || sessionUsage.status === "idle") {
    return t("inspector.notLoaded");
  }
  if (sessionUsage.status === "loading") {
    return t("common.loading");
  }
  if (sessionUsage.status === "error") {
    return t("inspector.notAvailable");
  }
  const cost = sessionUsage.data.totals_cost;
  if (!cost) {
    return t("inspector.notAvailable");
  }
  if (
    cost.availability === "unpriced" ||
    cost.total_usd === undefined ||
    cost.total_usd === null
  ) {
    return t("inspector.notAvailable");
  }
  if (cost.availability === "local_zero") {
    return `${cost.currency} 0.00`;
  }
  const digits = cost.total_usd > 0 && cost.total_usd < 0.01 ? 6 : 2;
  return `${cost.currency} ${cost.total_usd.toFixed(digits)}`;
}
