"use client";

import {
  ActivityLogIcon,
  ChevronRightIcon,
  CommitIcon,
  Component2Icon,
  Cross2Icon,
  EnterFullScreenIcon,
  ExclamationTriangleIcon,
  ExitFullScreenIcon,
  FileIcon,
  FileTextIcon,
  MagnifyingGlassIcon,
  PlusIcon,
} from "@radix-ui/react-icons";
import {
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

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
import { FileViewerPane } from "./FileViewerPane";
import { InspectorEmpty } from "./InspectorEmpty";
import { MessageEvidence } from "./MessageEvidence";
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
  moveWorkPanelTab,
  openWorkPanelFileTab,
  openWorkPanelTab,
  replaceWorkPanelTab,
  workPanelTabStep,
  type WorkPanelTab,
  type WorkPanelTabKind,
  type WorkPanelTabsState,
} from "./work-panel-tabs";
import type { WorkPanelTarget } from "./use-work-panel";
import { type WorkPanelState } from "./use-work-panel";

/** Drag distance before a tab press becomes a reorder (design §8). */
const TAB_DRAG_THRESHOLD_PX = 4;

export function RunInspector({
  productSessionId,
  workspaceId,
  workspaceRootPath,
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
  initialTabs,
  approvalBusy = null,
  approvalError = null,
  onApproval,
}: {
  productSessionId: string;
  workspaceId?: string;
  /** Absolute workspace root, used by the file viewer's OS reveal (Desktop). */
  workspaceRootPath?: string | null;
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
  /**
   * Seed for the standalone tab strip — the shell owns tab state when `panel`
   * is supplied, so this only applies to renders without a host (tests,
   * workbench previews).
   */
  initialTabs?: WorkPanelTabsState;
  approvalBusy?: string | null;
  approvalError?: string | null;
  onApproval?: (tool: ToolCallView, decision: "approve" | "reject") => void;
}) {
  const { t } = useCopy();
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  // The tab strip is a dynamic set, so a shell that
  // hosts the panel owns the state and a standalone render keeps its own.
  const [fallbackTabs, setFallbackTabs] = useState<WorkPanelTabsState>(
    () => initialTabs ?? defaultWorkPanelTabs(),
  );
  const [fallbackTarget, setFallbackTarget] = useState<WorkPanelTarget>(null);
  // `activeId` may legitimately be null (every tab closed), so the panel must
  // be selected by presence, not with a nullish fallback.
  const tabs = panel ? panel.tabs : fallbackTabs.tabs;
  const activeId = panel ? panel.activeId : fallbackTabs.activeId;
  const maximized = panel?.maximized ?? false;
  const openTab = (kind: WorkPanelTabKind) =>
    panel ? panel.openTab(kind) : setFallbackTabs((state) => openWorkPanelTab(state, kind));
  const activateTab = (id: string) =>
    panel ? panel.activateTab(id) : setFallbackTabs((state) => activateWorkPanelTab(state, id));
  const closeTab = (id: string) =>
    panel ? panel.closeTab(id) : setFallbackTabs((state) => closeWorkPanelTab(state, id));
  const replaceTab = (from: string, kind: WorkPanelTabKind) =>
    panel
      ? panel.replaceTab(from, kind)
      : setFallbackTabs((state) => replaceWorkPanelTab(state, from, kind));
  const moveTab = (id: string, toIndex: number) =>
    panel
      ? panel.moveTab(id, toIndex)
      : setFallbackTabs((state) => moveWorkPanelTab(state, id, toIndex));
  const openFile = (path: string, line?: number | null) => {
    if (panel) {
      panel.openFile(path, line ?? 1);
    } else {
      setFallbackTabs((state) => openWorkPanelFileTab(state, path));
      setFallbackTarget({ kind: "file", path, line: line ?? 1 });
    }
  };

  const tabButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const plusButtonRef = useRef<HTMLButtonElement>(null);
  const stripRef = useRef<HTMLDivElement>(null);
  // Pointer-drag reorder bookkeeping lives outside React state so pointermove
  // never re-renders the strip; only the indicator position renders.
  const dragRef = useRef<{
    pointerId: number;
    id: string;
    startClientX: number;
    dragging: boolean;
    insertIndex: number;
  } | null>(null);
  const [dragIndicator, setDragIndicator] = useState<{
    id: string;
    insertIndex: number;
  } | null>(null);
  // A finished drag is followed by a click on the same press; it must not also
  // activate the tab.
  const suppressClickRef = useRef(false);

  // Keep the active tab in view inside a scrollable strip.
  useEffect(() => {
    if (!activeId) {
      return;
    }
    tabButtonRefs.current[activeId]?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
    });
  }, [activeId, tabs.length]);

  const closeTabAndFocus = (id: string) => {
    const index = tabs.findIndex((item) => item.id === id);
    const next = index >= 0 ? tabs[index + 1] ?? tabs[index - 1] : undefined;
    closeTab(id);
    requestAnimationFrame(() => {
      if (next) {
        tabButtonRefs.current[next.id]?.focus();
      } else {
        plusButtonRef.current?.focus();
      }
    });
  };

  const onTabKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    id: string,
  ) => {
    const index = tabs.findIndex((item) => item.id === id);
    if (index < 0) {
      return;
    }
    // Delete/Backspace closes the tab.
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      closeTabAndFocus(id);
      return;
    }
    // Alt+Left/Alt+Right reorders the focused tab (design §8); the moved tab
    // keeps focus so the reorder reads as a move, not a jump.
    if (event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
      event.preventDefault();
      const toIndex = event.key === "ArrowLeft" ? index - 1 : index + 1;
      moveTab(id, toIndex);
      requestAnimationFrame(() => tabButtonRefs.current[id]?.focus());
      return;
    }
    if (event.altKey) {
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
    activateTab(next.id);
    requestAnimationFrame(() => tabButtonRefs.current[next.id]?.focus());
  };

  function onTabPointerDown(
    event: ReactPointerEvent<HTMLDivElement>,
    tab: WorkPanelTab,
    index: number,
  ) {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    // A cancelled drag never produces a click; drop any stale suppression.
    suppressClickRef.current = false;
    dragRef.current = {
      pointerId: event.pointerId,
      id: tab.id,
      startClientX: event.clientX,
      dragging: false,
      insertIndex: index,
    };
    // Capture is deferred until the press actually becomes a drag: capturing
    // on pointerdown retargets the click to this wrapper and the tab's own
    // activation handler would never see it.
  }

  function onTabPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) {
      return;
    }
    if (
      !drag.dragging &&
      Math.abs(event.clientX - drag.startClientX) < TAB_DRAG_THRESHOLD_PX
    ) {
      return;
    }
    if (!drag.dragging) {
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    drag.dragging = true;
    // The drop position is the first cell whose midpoint is right of the
    // pointer; the 2px edge indicator renders on that cell's leading edge.
    const cells = stripRef.current
      ? Array.from(stripRef.current.querySelectorAll<HTMLElement>(".inspector-tab"))
      : [];
    let insertIndex = cells.length;
    for (let i = 0; i < cells.length; i += 1) {
      const rect = cells[i]!.getBoundingClientRect();
      if (event.clientX < rect.left + rect.width / 2) {
        insertIndex = i;
        break;
      }
    }
    if (insertIndex !== drag.insertIndex || !dragIndicator) {
      drag.insertIndex = insertIndex;
      setDragIndicator({ id: drag.id, insertIndex });
    }
  }

  function finishTabDrag(event: ReactPointerEvent<HTMLDivElement>, cancel: boolean) {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) {
      return;
    }
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (drag.dragging) {
      // The press ends in a click on the same element; swallow it.
      suppressClickRef.current = true;
      if (!cancel) {
        const fromIndex = tabs.findIndex((item) => item.id === drag.id);
        // Removing the dragged tab first shifts every later slot down one.
        const toIndex =
          drag.insertIndex > fromIndex ? drag.insertIndex - 1 : drag.insertIndex;
        if (fromIndex >= 0) {
          moveTab(drag.id, toIndex);
        }
      }
    }
    setDragIndicator(null);
  }

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
  // The drag preview bypasses React entirely (--work-panel-preview), so the
  // committed width is also what the ARIA value reports. Maximize disables the
  // handle entirely (design §8).
  const renderedPanelWidth = panelLayout?.panelWidth ?? panel?.width ?? 0;
  const panelResize = usePanelResize({
    renderedWidth: renderedPanelWidth,
    maxPanelWidth: panelLayout?.maxPanelWidth ?? panel?.width ?? 0,
    setWidth: panel?.setWidth ?? (() => undefined),
    resizeByKeyboard: panel?.resizeByKeyboard ?? (() => undefined),
  });
  const target = panel ? panel.target : fallbackTarget;
  const focusFilePath =
    target?.kind === "file" ? target.path : (fileFocusPath ?? null);
  const focusFileLine =
    target?.kind === "file" ? target.line : (fileFocusLine ?? null);
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
  const evidenceMessages = useMemo(
    () =>
      runState.messages.filter(
        (message) =>
          message.role === "assistant" &&
          (message.usage || message.promptBuild || message.promptCompaction),
      ),
    [runState.messages],
  );

  function renderPane(tab: WorkPanelTab): ReactNode {
    switch (tab.kind) {
      case "new":
        return (
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
                    onClick={() => replaceTab(tab.id, kind)}
                  >
                    {workPanelTabIcon(kind)}
                    {workPanelTabLabel(
                      { id: kind, kind },
                      t,
                      { waiting: waiting.length, reviews: reviews.length },
                    )}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        );
      case "pending":
        return (
          <>
            <section className="approval-detail" aria-label={t("inspector.approvalDetail")}>
              {approvalError ? <p role="alert">{approvalError}</p> : null}
              {waiting.map((tool) => (
                <button
                  key={tool.id}
                  type="button"
                  className="secondary"
                  onClick={() =>
                    (panel?.setTarget ?? setFallbackTarget)({
                      kind: "approval",
                      jobId: runState.activeJobId!,
                      runId: runState.activeRunId!,
                      callId: tool.id,
                    })
                  }
                >
                  {tool.name} · {tool.id}
                </button>
              ))}
              {selectedApproval && onApproval ? (
                <>
                  <p className="approval-detail__identity">
                    {runState.activeRunId} / {selectedApproval.id}
                  </p>
                  <ApprovalCard
                    tool={selectedApproval}
                    busy={approvalBusy === selectedApproval.id}
                    onApproval={onApproval}
                  />
                </>
              ) : waiting.length > 0 || target?.kind === "approval" ? (
                // A picked call that left the pending list gets the honest
                // message — never imply this window granted it.
                <p role="status">{t("inspector.approvalUnavailable")}</p>
              ) : (
                <InspectorEmpty
                  icon={<ExclamationTriangleIcon />}
                  title={t("inspector.approvalsNone")}
                  body={t("inspector.approvalsNoneBody")}
                />
              )}
            </section>
            {/* The durable authorization history for this session. */}
            {workspaceId ? (
              <SessionAuthorizationPanel
                sessionId={productSessionId}
                workspaceId={workspaceId}
              />
            ) : null}
          </>
        );
      case "files":
        return workspaceId ? (
          <FilesPanel
            workspaceId={workspaceId}
            onOpenFile={(path) => {
              openFile(path, null);
              onOpenReviewFinding?.(path, 1);
            }}
          />
        ) : (
          <InspectorEmpty
            icon={<FileIcon />}
            title={t("inspector.filesEmpty")}
            body={t("inspector.filesNoWorkspace")}
          />
        );
      case "file":
        return workspaceId && tab.path ? (
          <FileViewerPane
            workspaceId={workspaceId}
            workspaceRootPath={workspaceRootPath}
            path={tab.path}
            focusLine={tab.path === focusFilePath ? focusFileLine : null}
          />
        ) : (
          <InspectorEmpty
            icon={<FileTextIcon />}
            title={t("inspector.fileUnavailable")}
            body={t("inspector.fileUnavailableBody")}
          />
        );
      case "changes":
        return (
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
        );
      case "review":
        return (
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
              // The finding's file lives in its own viewer tab, so the panel
              // opens it rather than only switching the suspended run tab.
              openFile(path, line);
              onOpenReviewFinding?.(path, line);
            }}
          />
        );
      case "subagents":
        return (
          <InspectorEmpty
            icon={<Component2Icon />}
            title={t("inspector.subagentsNone")}
            body={t("inspector.subagentsNoneBody")}
          />
        );
      case "status":
        return (
          <>
            {phase === "empty" ? (
              <InspectorEmpty
                title={t("inspector.emptyTitle")}
                body={t("inspector.emptyBody")}
              />
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
                  {evidenceMessages.length > 0 ? (
                    // §2.2: measured per-turn detail is evidence for the panel,
                    // not furniture for the conversation. One dl per turn that
                    // published usage, a prompt build, or pruning facts.
                    <div className="inspector-evidence">
                      {evidenceMessages.map((message) => (
                        <MessageEvidence key={message.id} message={message} />
                      ))}
                    </div>
                  ) : null}
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
        );
      default:
        return null;
    }
  }

  if (collapsed) {
    // The closed panel keeps its subtree mounted so the allocated-width
    // collapse can animate, but at zero width it must be inert and out of the
    // accessibility tree; the floating toggle reopens it (design §5.0).
    return (
      <aside
        className="product-inspector"
        data-collapsed="true"
        aria-label={t("inspector.title")}
        aria-hidden="true"
        inert
      />
    );
  }

  return (
    <aside
      className="product-inspector"
      aria-label={t("inspector.title")}
      data-collapsed="false"
      data-phase={phase}
      data-open={dialogOpen}
      data-maximized={maximized || undefined}
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
      {panel && !dialogOpen ? (
        <div
          className="inspector-resize"
          role="separator"
          aria-orientation="vertical"
          aria-label={t("inspector.panelWidth")}
          // The resize handle is disabled while the panel is maximized
          // (design §8): the conversation floor no longer applies.
          aria-disabled={maximized || undefined}
          aria-valuemin={maximized ? undefined : panelResize.minimumWidth}
          aria-valuemax={maximized ? undefined : panelResize.maximumWidth}
          aria-valuenow={Math.round(renderedPanelWidth)}
          aria-valuetext={t("inspector.panelWidthValue", {
            width: Math.round(renderedPanelWidth),
          })}
          tabIndex={maximized ? -1 : 0}
          onPointerDown={maximized ? undefined : panelResize.onPointerDown}
          onPointerMove={maximized ? undefined : panelResize.onPointerMove}
          onPointerUp={maximized ? undefined : panelResize.onPointerUp}
          onPointerCancel={maximized ? undefined : panelResize.onPointerCancel}
          onLostPointerCapture={maximized ? undefined : panelResize.onPointerCancel}
          onKeyDown={maximized ? undefined : panelResize.onKeyDown}
        />
      ) : null}
      {/* The 46px dock header carries the tab strip and the strip's own
          actions; the surface's title lives on the aside's aria-label. */}
      <div className="inspector-header">
        <div
          ref={stripRef}
          className="inspector-tabs"
          data-dragging={dragIndicator ? "true" : undefined}
          // An empty tablist owns no tabs, which is an ARIA violation, so the
          // role only exists while the strip has content.
          role={tabs.length > 0 ? "tablist" : undefined}
          aria-label={tabs.length > 0 ? t("inspector.tabsLabel") : undefined}
        >
          {tabs.map((item, index) => {
            const selected = item.id === activeId;
            const label = workPanelTabLabel(item, t, {
              waiting: waiting.length,
              reviews: reviews.length,
            });
            const edge =
              dragIndicator === null
                ? undefined
                : dragIndicator.insertIndex === index
                  ? "before"
                  : dragIndicator.insertIndex === tabs.length &&
                      index === tabs.length - 1
                    ? "after"
                    : undefined;
            return (
              // The cell is layout only: marking it presentational keeps the tab
              // the tablist's owned child instead of the wrapper. Its close button
              // stays a real, named control inside the tab list, which ARIA does
              // not allow as a child of `tablist`; that one deviation is accepted,
              // asserted in both directions, and explained in
              // `tests/e2e/accessibility.spec.ts`.
              <div
                className="inspector-tab"
                key={item.id}
                role="presentation"
                data-drag-edge={edge}
                data-drag-source={
                  dragIndicator?.id === item.id ? "true" : undefined
                }
                onPointerDown={(event) => onTabPointerDown(event, item, index)}
                onPointerMove={onTabPointerMove}
                onPointerUp={(event) => finishTabDrag(event, false)}
                onPointerCancel={(event) => finishTabDrag(event, true)}
              >
                <button
                  ref={(node) => {
                    tabButtonRefs.current[item.id] = node;
                  }}
                  type="button"
                  role="tab"
                  id={`inspector-tab-${item.id}`}
                  aria-selected={selected}
                  aria-controls={`inspector-surface-${item.id}`}
                  tabIndex={selected ? 0 : -1}
                  className={selected ? "tab-button tab-button--active" : "tab-button"}
                  onClick={() => {
                    if (suppressClickRef.current) {
                      suppressClickRef.current = false;
                      return;
                    }
                    activateTab(item.id);
                  }}
                  onMouseDown={(event) => {
                    // Keep the middle press from latching autoscroll on a
                    // scrollable ancestor; that gesture swallows auxclick, so
                    // the close itself rides on pointerup below.
                    if (event.button === 1) {
                      event.preventDefault();
                    }
                  }}
                  onPointerUp={(event) => {
                    // Middle-click closes the tab (design §8). pointerup —
                    // not auxclick — because autoscroll suppresses auxclick.
                    if (event.button !== 1) return;
                    event.preventDefault();
                    closeTabAndFocus(item.id);
                  }}
                  onKeyDown={(event) => onTabKeyDown(event, item.id)}
                >
                  {workPanelTabIcon(item.kind)}
                  <span className="inspector-tab__label">{label}</span>
                </button>
                <button
                  type="button"
                  className="inspector-tab-close"
                  aria-label={t("inspector.closeTab", { name: label })}
                  title={t("inspector.closeTab", { name: label })}
                  onPointerDown={(event) => event.stopPropagation()}
                  onClick={() => closeTabAndFocus(item.id)}
                >
                  <Cross2Icon />
                </button>
              </div>
            );
          })}
        </div>
        <div className="inspector-actions">
          {/* The launcher sits at the end of the strip and
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
          {panel && !dialogOpen ? (
            <button
              type="button"
              className="ghost icon-button inspector-maximize"
              onClick={panel.toggleMaximize}
              aria-pressed={maximized}
              aria-label={maximized ? t("inspector.unmaximize") : t("inspector.maximize")}
              title={maximized ? t("inspector.unmaximize") : t("inspector.maximize")}
            >
              {maximized ? <ExitFullScreenIcon /> : <EnterFullScreenIcon />}
            </button>
          ) : null}
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
      </div>
      <div className="inspector-body">
        {tabs.map((tab) => (
          // Every open tab stays mounted so hidden panes keep their state
          // (design §8); `hidden` + `inert` keep them out of the accessibility
          // tree and the tab order until activated.
          <div
            key={tab.id}
            className="inspector-pane"
            role="tabpanel"
            id={`inspector-surface-${tab.id}`}
            aria-labelledby={`inspector-tab-${tab.id}`}
            hidden={tab.id !== activeId}
            inert={tab.id !== activeId ? true : undefined}
          >
            {renderPane(tab)}
          </div>
        ))}
        {tabs.length === 0 ? (
          <InspectorEmpty
            title={t("inspector.noTabsTitle")}
            body={t("inspector.noTabs")}
          />
        ) : null}
      </div>
    </aside>
  );
}

/**
 * Tab label, including the badge counts the previous fixed tabs carried.
 * A file tab is labelled by its file's basename so two opened files read
 * distinctly in the strip.
 */
export function workPanelTabLabel(
  tab: WorkPanelTab,
  t: (path: string, params?: Record<string, string | number>) => string,
  counts: { waiting: number; reviews: number },
): string {
  switch (tab.kind) {
    case "new":
      return t("inspector.tabNew");
    case "pending":
      return counts.waiting > 0
        ? `${t("inspector.tabPending")} (${counts.waiting})`
        : t("inspector.tabPending");
    case "files":
      return t("inspector.tabFiles");
    case "file":
      return tab.path?.split("/").filter(Boolean).pop() ?? t("inspector.tabFile");
    case "changes":
      return t("inspector.tabChanges");
    case "review":
      return counts.reviews > 0
        ? `${t("inspector.tabReview")} (${counts.reviews})`
        : t("inspector.tabReview");
    case "subagents":
      return t("inspector.tabSubagents");
    case "status":
      return t("inspector.tabStatus");
  }
}

/** One icon per tab kind so the strip reads at a glance (design §8). */
function workPanelTabIcon(kind: WorkPanelTabKind): ReactNode {
  switch (kind) {
    case "new":
      return <PlusIcon aria-hidden="true" />;
    case "pending":
      return <ExclamationTriangleIcon aria-hidden="true" />;
    case "status":
      return <ActivityLogIcon aria-hidden="true" />;
    case "files":
      return <FileIcon aria-hidden="true" />;
    case "file":
      return <FileTextIcon aria-hidden="true" />;
    case "changes":
      return <CommitIcon aria-hidden="true" />;
    case "review":
      return <MagnifyingGlassIcon aria-hidden="true" />;
    case "subagents":
      return <Component2Icon aria-hidden="true" />;
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
