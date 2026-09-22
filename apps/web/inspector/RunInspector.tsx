"use client";

import {
  ChevronLeftIcon,
  ChevronRightIcon,
  Cross2Icon,
  PlusIcon,
} from "@radix-ui/react-icons";
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
import { SessionAuthorizationPanel } from "./SessionAuthorizationPanel";
import { FilesPanel } from "./FilesPanel";
import { ReviewPanel } from "./ReviewPanel";
import type {
  ProductReview,
  ProductReviewFindingPageItem,
} from "../product/product-api-types";

import { ApprovalCard } from "../chat/Transcript";
import { type WorkPanelLayout } from "./work-panel-layout";
import { usePanelResize } from "./use-panel-resize";
import {
  WORK_PANEL_LAUNCHER_KINDS,
  activateWorkPanelTab,
  closeWorkPanelTab,
  defaultWorkPanelTabs,
  openWorkPanelTab,
  replaceWorkPanelTab,
  workPanelTabStep,
  type WorkPanelTabKind,
  type WorkPanelTabsState,
} from "./work-panel-tabs";
import { type WorkPanelState } from "./use-work-panel";
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
  panel,
  panelLayout,
  uiVersion = "v2",
  approvalBusy = null,
  approvalError = null,
  onApproval,
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
  panel?: WorkPanelState;
  /** Live width budget for the panel separator (design §5.0). */
  panelLayout?: WorkPanelLayout;
  /** v2 sizes the panel from its grid track; v1 keeps an inline width. */
  uiVersion?: "v1" | "v2";
  approvalBusy?: string | null;
  approvalError?: string | null;
  onApproval?: (tool: ToolCallView, decision: "approve" | "reject") => void;
}) {
  const { t } = useCopy();
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  // The tab strip is a dynamic set (PI-Desktop `WorkPanel`), so a shell that
  // hosts the panel owns the state and a standalone render keeps its own.
  const [fallbackTabs, setFallbackTabs] = useState<WorkPanelTabsState>(() =>
    defaultWorkPanelTabs(),
  );
  // `activeKind` may legitimately be null (every tab closed), so the panel must
  // be selected by presence, not with a nullish fallback.
  const tabs = panel ? panel.tabs : fallbackTabs.tabs;
  const activeKind = panel ? panel.activeKind : fallbackTabs.activeKind;
  const openTab = (kind: WorkPanelTabKind) =>
    panel ? panel.openTab(kind) : setFallbackTabs((state) => openWorkPanelTab(state, kind));
  const activateTab = (kind: WorkPanelTabKind) =>
    panel ? panel.activateTab(kind) : setFallbackTabs((state) => activateWorkPanelTab(state, kind));
  const closeTab = (kind: WorkPanelTabKind) =>
    panel ? panel.closeTab(kind) : setFallbackTabs((state) => closeWorkPanelTab(state, kind));
  const replaceTab = (from: WorkPanelTabKind, kind: WorkPanelTabKind) =>
    panel
      ? panel.replaceTab(from, kind)
      : setFallbackTabs((state) => replaceWorkPanelTab(state, from, kind));

  const tabButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const plusButtonRef = useRef<HTMLButtonElement>(null);

  // Keep the active tab in view inside a scrollable strip (PI-Desktop parity).
  useEffect(() => {
    if (!activeKind) {
      return;
    }
    tabButtonRefs.current[activeKind]?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
    });
  }, [activeKind, tabs.length]);

  const closeTabAndFocus = (kind: WorkPanelTabKind) => {
    const index = tabs.findIndex((item) => item.kind === kind);
    const next = index >= 0 ? tabs[index + 1] ?? tabs[index - 1] : undefined;
    closeTab(kind);
    requestAnimationFrame(() => {
      if (next) {
        tabButtonRefs.current[next.kind]?.focus();
      } else {
        plusButtonRef.current?.focus();
      }
    });
  };

  const onTabKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    kind: WorkPanelTabKind,
  ) => {
    const index = tabs.findIndex((item) => item.kind === kind);
    if (index < 0) {
      return;
    }
    // Delete/Backspace closes, like the reference tab strip.
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      closeTabAndFocus(kind);
      return;
    }
    const step = workPanelTabStep(event, index, tabs.length);
    if (step === null) {
      return;
    }
    event.preventDefault();
    const next = tabs[step];
    if (!next) {
      return;
    }
    activateTab(next.kind);
    requestAnimationFrame(() => tabButtonRefs.current[next.kind]?.focus());
  };

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
      openTab("review");
    }
    // Only the active review identity may open the tab; re-running on every
    // render would fight the user's own tab choice.
  }, [activeReviewId]);

  const phase = resolveInspectorPhase(runState);
  const waiting = runState.tools.filter((tool) => tool.pendingApproval);
  // The separator owns the pointer/keyboard resize; the budget owns the cap.
  const panelResize = usePanelResize({
    renderedWidth: panelLayout?.panelWidth ?? panel?.width ?? 0,
    maxPanelWidth: panelLayout?.maxPanelWidth ?? panel?.width ?? 0,
    setWidth: panel?.setWidth ?? (() => undefined),
    resizeByKeyboard: panel?.resizeByKeyboard ?? (() => undefined),
  });
  const target = panel?.target;
  const selectedApproval = target?.kind === "approval" &&
    target.jobId === runState.activeJobId && target.runId === runState.activeRunId
    ? waiting.find((tool) => tool.id === target.callId) : undefined;
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
      style={
        // The v2 shell sizes the panel from its grid track (design §5.0 shared
        // width budget), so an inline width would make it unshrinkable. The v1
        // skin has no such track and keeps the direct width.
        panel && !dialogOpen && uiVersion === "v1"
          ? { width: `min(${panel.width}px, 45vw)` }
          : undefined
      }
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
      {panel && !dialogOpen ? (
        <div
          className="inspector-resize"
          role="separator"
          aria-orientation="vertical"
          aria-label={t("inspector.panelWidth")}
          aria-valuemin={panelResize.minimumWidth}
          aria-valuemax={panelResize.maximumWidth}
          aria-valuenow={Math.round(panelResize.value)}
          aria-valuetext={t("inspector.panelWidthValue", {
            width: Math.round(panelResize.value),
          })}
          tabIndex={0}
          onPointerDown={panelResize.onPointerDown}
          onPointerMove={panelResize.onPointerMove}
          onPointerUp={panelResize.onPointerUp}
          onPointerCancel={panelResize.onPointerCancel}
          onLostPointerCapture={panelResize.onPointerCancel}
          onKeyDown={panelResize.onKeyDown}
        />
      ) : null}
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
      <div className="inspector-tabs-row">
        <div
          className="inspector-tabs"
          // An empty tablist owns no tabs, which is an ARIA violation, so the
          // role only exists while the strip has content.
          role={tabs.length > 0 ? "tablist" : undefined}
          aria-label={tabs.length > 0 ? t("inspector.tabsLabel") : undefined}
        >
        {tabs.map((item) => {
          const selected = item.kind === activeKind;
          const label = workPanelTabLabel(item.kind, t, {
            waiting: waiting.length,
            reviews: reviews.length,
          });
          return (
            <div className="inspector-tab" key={item.kind}>
              <button
                ref={(node) => {
                  tabButtonRefs.current[item.kind] = node;
                }}
                type="button"
                role="tab"
                id={`inspector-tab-${item.kind}`}
                aria-selected={selected}
                aria-controls={`inspector-surface-${item.kind}`}
                tabIndex={selected ? 0 : -1}
                className={selected ? "tab-button tab-button--active" : "tab-button"}
                onClick={() => activateTab(item.kind)}
                onAuxClick={(event) => {
                  // Middle-click closes, like the reference panel's tab strip.
                  if (event.button !== 1) return;
                  event.preventDefault();
                  closeTabAndFocus(item.kind);
                }}
                onKeyDown={(event) => onTabKeyDown(event, item.kind)}
              >
                {label}
              </button>
              <button
                type="button"
                className="inspector-tab-close"
                aria-label={t("inspector.closeTab", { name: label })}
                title={t("inspector.closeTab", { name: label })}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={() => closeTabAndFocus(item.kind)}
              >
                <Cross2Icon />
              </button>
            </div>
          );
        })}
        </div>
        {/* The launcher sits at the end of the strip (PI-Desktop parity) and
            outside the tablist, because only tabs belong in a tablist. */}
        <button
          ref={plusButtonRef}
          type="button"
          className="ghost icon-button inspector-open-tab"
          onClick={() => openTab("new")}
          aria-label={t("inspector.openTab")}
          title={t("inspector.openTab")}
        >
          <PlusIcon />
        </button>
      </div>
      <div
        className="inspector-body"
        id={activeKind ? `inspector-surface-${activeKind}` : undefined}
        // Without an active tab there is no tab to label the surface, and an
        // unlabelled tabpanel is not worth exposing.
        role={activeKind ? "tabpanel" : undefined}
        aria-labelledby={activeKind ? `inspector-tab-${activeKind}` : undefined}
      >
        {activeKind === "new" ? (
          <section
            className="inspector-section"
            aria-label={t("inspector.launcherTitle")}
          >
            <h3>{t("inspector.launcherTitle")}</h3>
            <ul className="inspector-launcher">
              {WORK_PANEL_LAUNCHER_KINDS.map((kind) => (
                <li key={kind}>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => replaceTab("new", kind)}
                  >
                    {workPanelTabLabel(kind, t, {
                      waiting: waiting.length,
                      reviews: reviews.length,
                    })}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        {activeKind === null ? (
          <p className="inspector-empty-line">{t("inspector.noTabs")}</p>
        ) : null}
        {activeKind === "pending" ? (
          <section className="approval-detail" aria-label={t("inspector.approvalDetail")}>
            {approvalError ? <p role="alert">{approvalError}</p> : null}
            {waiting.map((tool) => <button key={tool.id} type="button" className="secondary"
              onClick={() => panel?.setTarget({ kind: "approval", jobId: runState.activeJobId!,
                runId: runState.activeRunId!, callId: tool.id })}>
              {tool.name} · {tool.id}
            </button>)}
            {selectedApproval && onApproval ? <>
              <p className="approval-detail__identity">{runState.activeRunId} / {selectedApproval.id}</p>
              <ApprovalCard tool={selectedApproval} busy={approvalBusy === selectedApproval.id}
                onApproval={onApproval} />
            </> : <p role="status">{t("inspector.approvalUnavailable")}</p>}
            {/* Plan P4: the durable authorization history for this session. */}
            {workspaceId ? (
              <SessionAuthorizationPanel
                sessionId={productSessionId}
                workspaceId={workspaceId}
              />
            ) : null}
          </section>
        ) : activeKind === "review" ? (
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
              // The finding's file lives in the files tab, so the panel opens it
              // rather than only switching the suspended run tab.
              openTab("files");
              onOpenReviewFinding?.(path, line);
            }}
          />
        ) : activeKind === "status" ? (
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

            {phase !== "empty" || activeKind !== "status" ? (
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

              </>
            ) : null}
          </>
        ) : null}

        {activeKind === "files" ? (
          workspaceId ? (
            <FilesPanel
              workspaceId={workspaceId}
              focusPath={fileFocusPath}
              focusLine={fileFocusLine}
            />
          ) : null
        ) : null}

        {activeKind === "changes" ? (
          <>
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
          </>
        ) : null}

        {activeKind === "status" && phase !== "empty" ? (
          <>

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

          </>
        ) : null}

        {activeKind === "pending" ? (
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
        ) : null}

        {activeKind === "status" && phase !== "empty" ? (
          <>

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
      </div>
    </aside>
  );
}

/**
 * Tab label, including the badge counts the previous fixed tabs carried.
 * Every kind is host-owned in rove, so a kind maps to one localized label.
 */
export function workPanelTabLabel(
  kind: WorkPanelTabKind,
  t: (path: string, params?: Record<string, string | number>) => string,
  counts: { waiting: number; reviews: number },
): string {
  switch (kind) {
    case "new":
      return t("inspector.tabNew");
    case "pending":
      return counts.waiting > 0
        ? `${t("inspector.tabPending")} (${counts.waiting})`
        : t("inspector.tabPending");
    case "files":
      return t("inspector.tabFiles");
    case "changes":
      return t("inspector.tabChanges");
    case "review":
      return counts.reviews > 0
        ? `${t("inspector.tabReview")} (${counts.reviews})`
        : t("inspector.tabReview");
    case "status":
      return t("inspector.tabStatus");
  }
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
