"use client";

import {
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { Composer } from "../chat/Composer";
import { CopyProvider, useCopy } from "../copy/CopyProvider";
import { Transcript } from "../chat/Transcript";
import { RunInspector } from "../inspector/RunInspector";
import { useWorkPanel } from "../inspector/use-work-panel";
import { useSessionUsage } from "../state/use-session-usage";
import {
  createComposerDraftStore,
  type ComposerDraftStore,
} from "../state/composer-draft-store";
import { selectTranscriptTimeline } from "../lib/rove-state";
import { SettingsShell } from "../settings/SettingsShell";
import { matchKeyboardShortcut } from "../settings/keyboard-settings-model";
import { createSettingsPlatformClient } from "../settings/settings-platform-client";
import { EmptyState } from "../sidebar/EmptyState";
import { WorkspaceTree } from "../sidebar/WorkspaceTree";
import {
  findSession,
  findWorkspace,
  sessionsForWorkspace,
  sortedWorkspaces,
} from "../state/product-catalog";
import { useProductRouteSync } from "../state/use-product-route-sync";
import {
  type ProductBootState,
  useServerProductState,
} from "../state/use-server-product-state";
import { useSessionContinuity } from "../state/use-session-continuity";
import { useProductReviews } from "../state/use-product-reviews";
import type { WorkspaceKind } from "../state/product-types";
import { M1MigrationGate } from "./M1MigrationGate";
import { TopBar } from "./TopBar";
import { UiSkinProvider, useUiSkin, type UiSkin } from "./ui-skin";

export type ProductUiVersion = "v1" | "v2";
export type { UiSkin };

/** Strip Windows long-path prefixes so roots read as ordinary paths. */
function formatDisplayPath(path: string): string {
  if (path.startsWith("\\\\?\\")) {
    return path.slice(4);
  }
  return path;
}

export function ProductApp({
  uiVersion = "v2",
}: {
  uiVersion?: ProductUiVersion;
}) {
  const [draftStore] = useState(createComposerDraftStore);
  return (
    <CopyProvider>
      <UiSkinProvider>
        <ProductFrame uiVersion={uiVersion} draftStore={draftStore} />
      </UiSkinProvider>
    </CopyProvider>
  );
}

function ProductFrame({ uiVersion, draftStore }: {
  uiVersion: ProductUiVersion;
  draftStore: ComposerDraftStore;
}) {
  const { skin } = useUiSkin();
  return (
    <div
      className="product-app-frame"
      data-ui-version={uiVersion}
      data-skin={skin}
    >
      <M1MigrationGate>
        <ServerProductApp uiVersion={uiVersion} draftStore={draftStore} />
      </M1MigrationGate>
    </div>
  );
}

function ServerProductApp({ uiVersion, draftStore }: {
  uiVersion: ProductUiVersion;
  draftStore: ComposerDraftStore;
}) {
  const { t } = useCopy();
  const server = useServerProductState();
  const settingsClient = useMemo(() => createSettingsPlatformClient(), []);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const workspaceButtonRef = useRef<HTMLButtonElement>(null);
  const inspectorButtonRef = useRef<HTMLButtonElement>(null);
  const panel = useWorkPanel(
    server.catalog.active.workspaceId,
    server.catalog.active.sessionId,
    inspectorButtonRef,
  );
  const panelRef = useRef(panel);
  const inspectorCollapsed = panel.collapsed;
  const [mobileLayout, setMobileLayout] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);

  // Keep a stable ref to the panel for the *current* workspace/session. The
  // media-query listener below is registered once, so it must not close over
  // the panel object from the first render (that would drive another
  // session's cached selection instead of the one being viewed).
  useEffect(() => {
    panelRef.current = panel;
  });

  useEffect(() => {
    const narrow = window.matchMedia("(max-width: 960px)");
    const syncInspector = () => {
      setMobileLayout(narrow.matches);
      panelRef.current.setCollapsed(narrow.matches);
      if (!narrow.matches) {
        setWorkspaceOpen(false);
      }
    };
    syncInspector();
    narrow.addEventListener("change", syncInspector);
    return () => narrow.removeEventListener("change", syncInspector);
  }, []);
  const activeWorkspace = findWorkspace(
    server.catalog,
    server.catalog.active.workspaceId,
  );
  const activeSession = findSession(
    server.catalog,
    server.catalog.active.sessionId,
  );
  const continuity = useSessionContinuity({
    productClient: server.productClient,
    catalogRef: server.catalogRef,
    activeSession,
    selection: server.selection,
    profiles: server.profiles,
    markSession: server.markSession,
    refreshSessionStatuses: server.refreshSessionStatuses,
    updateSessionTitle: server.updateSessionTitle,
    setConnection: server.setConnection,
  });
  const reviews = useProductReviews({
    productClient: server.productClient,
    sessionId: activeSession?.id ?? null,
    workspaceKind:
      activeWorkspace?.kind === "repo"
        ? "repo"
        : activeWorkspace
          ? "folder"
          : undefined,
  });
  const routing = useProductRouteSync({
    ready: server.bootState.status === "ready",
    catalog: server.catalog,
    preferences: server.preferences,
    selectCatalogRoute: server.selectCatalogRoute,
    persistActiveRoute: server.persistActiveRoute,
    focusSession: continuity.focusSession,
    prepareSession: continuity.prepareSession,
    leaveSession: continuity.leaveSession,
  });
  const workspaces = sortedWorkspaces(server.catalog);
  const sessionsByWorkspace = useMemo(() => {
    const map: Record<string, ReturnType<typeof sessionsForWorkspace>> = {};
    for (const workspace of server.catalog.workspaces) {
      map[workspace.id] = sessionsForWorkspace(server.catalog, workspace.id);
    }
    return map;
  }, [server.catalog]);

  const connectionLabel =
    server.connection === "ok"
      ? t("connection.ok")
      : server.connection === "error"
        ? t("connection.error")
        : t("connection.checking");
  const connectionTone =
    server.connection === "ok"
      ? "ok"
      : server.connection === "error"
        ? "error"
        : "idle";
  const busy = continuity.runState.busy;
  const sessionUsage = useSessionUsage(activeSession?.id ?? null, busy);
  const awaitingInitialRestore =
    routing.route.kind === "session" &&
    activeSession?.id === routing.route.sessionId &&
    continuity.restoreState.status === "idle";
  const transcriptRestoreState = awaitingInitialRestore
    ? ({ status: "loading", sessionId: activeSession.id } as const)
    : continuity.restoreState;
  const composerPrerequisiteUnavailable =
    awaitingInitialRestore ||
    continuity.restoreState.status === "loading" ||
    continuity.restoreState.status === "error" ||
    server.sessionModelConfigLoading ||
    server.sessionModelConfig === null ||
    routing.routeError !== null;
  const composerDisabled =
    composerPrerequisiteUnavailable;
  const controlAvailable =
    !composerPrerequisiteUnavailable &&
    activeSession !== undefined &&
    activeSession !== null &&
    (busy ||
      activeSession.status === "running" ||
      activeSession.status === "needs_attention");
  const forkAvailable =
    activeSession?.status === "idle" &&
    Boolean(activeSession.activeRunId) &&
    !server.catalogMutationBusy;
  const composerDisabledReason = awaitingInitialRestore
    ? t("chat.disabledRestoring")
    : continuity.restoreState.status === "loading"
      ? t("chat.disabledRestoring")
    : continuity.restoreState.status === "error"
      ? t("chat.disabledRestoreError")
      : server.sessionModelConfigLoading || server.sessionModelConfig === null
        ? t("chat.disabledSettings")
      : routing.routeError
              ? t("chat.disabledRoute")
              : undefined;

  function closeWorkspaceDrawer() {
    setWorkspaceOpen(false);
    window.requestAnimationFrame(() => workspaceButtonRef.current?.focus());
  }

  function closeInspector() {
    panel.close();
  }

  async function handleOpenWorkspace(path: string, kind: WorkspaceKind) {
    const activeBefore = { ...server.catalogRef.current.active };
    const navigationIntent = routing.captureNavigationIntent();
    const target = await server.openWorkspace(path, kind);
    const activeNow = server.catalogRef.current.active;
    if (
      target &&
      routing.isNavigationIntentCurrent(navigationIntent) &&
      activeNow.workspaceId === activeBefore.workspaceId &&
      activeNow.sessionId === activeBefore.sessionId
    ) {
      routing.navigateSession(target.workspaceId, target.sessionId);
      setWorkspaceOpen(false);
    }
  }

  async function handleNewSession(workspaceId: string) {
    const activeBefore = { ...server.catalogRef.current.active };
    const navigationIntent = routing.captureNavigationIntent();
    const session = await server.createSession(workspaceId);
    const activeNow = server.catalogRef.current.active;
    if (
      session &&
      routing.isNavigationIntentCurrent(navigationIntent) &&
      activeNow.workspaceId === activeBefore.workspaceId &&
      activeNow.sessionId === activeBefore.sessionId
    ) {
      routing.navigateSession(workspaceId, session.id);
      setWorkspaceOpen(false);
    }
  }

  async function handleForkSession() {
    if (!activeWorkspace || !activeSession) {
      return;
    }
    const activeBefore = { ...server.catalogRef.current.active };
    const navigationIntent = routing.captureNavigationIntent();
    const child = await server.forkSession(activeSession.id);
    const activeNow = server.catalogRef.current.active;
    if (
      child &&
      routing.isNavigationIntentCurrent(navigationIntent) &&
      activeNow.workspaceId === activeBefore.workspaceId &&
      activeNow.sessionId === activeBefore.sessionId
    ) {
      routing.navigateSession(activeWorkspace.id, child.id);
    }
  }

  async function handleRemoveWorkspace(workspaceId: string) {
    try {
      await removeWorkspaceAndLeaveIfActive(workspaceId);
    } catch {
      // The state hook exposes the failure through catalogError.
    }
  }

  async function removeWorkspaceAndLeaveIfActive(workspaceId: string) {
    const navigationIntent = routing.captureNavigationIntent();
    if (
      (await server.removeWorkspace(workspaceId)) &&
      routing.isNavigationIntentCurrent(navigationIntent)
    ) {
      continuity.leaveSession();
      routing.returnHome();
    }
  }

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent) {
      const shortcut = matchKeyboardShortcut(event);
      if (!shortcut) {
        return;
      }

      let handled = true;
      switch (shortcut.action) {
        case "focus-composer":
          if (routing.viewSettings || composerDisabled || !composerRef.current) {
            handled = false;
          } else {
            composerRef.current.focus();
          }
          break;
        case "new-session":
          if (!activeWorkspace || server.catalogMutationBusy) {
            handled = false;
          } else {
            void handleNewSession(activeWorkspace.id);
          }
          break;
        case "open-settings":
          routing.openSettings("general");
          break;
        case "toggle-inspector":
          if (routing.viewSettings || !activeWorkspace || !activeSession) {
            handled = false;
          } else {
            panel.toggle();
          }
          break;
      }

      if (handled) {
        event.preventDefault();
      }
    }

    document.addEventListener("keydown", handleShortcut);
    return () => document.removeEventListener("keydown", handleShortcut);
  }, [
    activeSession,
    activeWorkspace,
    composerDisabled,
    routing,
    server.catalogMutationBusy,
    panel,
  ]);

  if (server.bootState.status !== "ready") {
    return (
      <div className="product-root" data-presentation={uiVersion}>
        <TopBar
          connectionLabel={connectionLabel}
          connectionTone={connectionTone}
          theme={server.theme}
          onToggleTheme={() =>
            server.changeTheme(server.theme === "dark" ? "light" : "dark")
          }
          onOpenSettings={() => routing.openSettings("general")}
          showSettingsBack={false}
          onBackToChat={routing.returnHome}
        />
        <BootStateView
          state={server.bootState}
          onRetry={() => void server.reload()}
        />
      </div>
    );
  }

  return (
    <div className="product-root" data-presentation={uiVersion}>
      <TopBar
        connectionLabel={connectionLabel}
        connectionTone={
          busy ? "working" : continuity.runState.error ? "error" : connectionTone
        }
        theme={server.theme}
        onToggleTheme={() =>
          server.changeTheme(server.theme === "dark" ? "light" : "dark")
        }
        onOpenSettings={() => routing.openSettings("providers")}
        showSettingsBack={routing.viewSettings}
        onBackToChat={routing.backToChat}
        workspaceButtonRef={routing.viewSettings ? undefined : workspaceButtonRef}
        onToggleWorkspace={
          routing.viewSettings
            ? undefined
            : () => setWorkspaceOpen((value) => !value)
        }
      />

      {routing.viewSettings ? (
        <div className="product-body" data-settings="true">
          <SettingsShell
            section={routing.settingsSection}
            onSectionChange={routing.openSettings}
            settingsClient={settingsClient}
            profiles={server.profiles}
            selection={server.selection}
            defaultApprovalPolicy={
              server.preferences?.default_approval_policy ??
              server.selection.approval
            }
            onCreateProfile={server.createProviderProfile}
            onUpdateProfile={server.updateProviderProfile}
            onDeleteProfile={server.deleteProviderProfile}
            onRefreshProviderProfiles={server.refreshProviderProfiles}
            onSelectionChange={server.changeSelection}
            onDefaultApprovalPolicyChange={
              server.changeDefaultApprovalPolicy
            }
            workspaces={server.catalog.workspaces}
            sessions={server.catalog.sessions}
            activeWorkspaceId={server.catalog.active.workspaceId}
            activeSessionId={server.catalog.active.sessionId}
            onSelectWorkspace={routing.navigateWorkspace}
            onSelectSession={routing.navigateSession}
            onTogglePin={server.togglePin}
            onRemoveWorkspace={removeWorkspaceAndLeaveIfActive}
            onRenameSession={server.updateSessionTitle}
            onDeleteSession={server.deleteSession}
            connectionLabel={connectionLabel}
            theme={server.theme}
            onThemeChange={server.changeTheme}
            error={server.catalogError}
          />
        </div>
      ) : (
        <div
          className="product-body"
          data-workspace-open={workspaceOpen}
          data-inspector-open={mobileLayout && !inspectorCollapsed}
        >
          <WorkspaceTree
            workspaces={workspaces}
            sessionsByWorkspace={sessionsByWorkspace}
            activeWorkspaceId={server.catalog.active.workspaceId}
            activeSessionId={server.catalog.active.sessionId}
            mutationBusy={server.catalogMutationBusy}
            onOpenWorkspace={(path, kind) => void handleOpenWorkspace(path, kind)}
            onSelectWorkspace={routing.navigateWorkspace}
            onSelectSession={(workspaceId, sessionId) => {
              routing.navigateSession(workspaceId, sessionId);
              setWorkspaceOpen(false);
            }}
            onNewSession={(workspaceId) => void handleNewSession(workspaceId)}
            onTogglePin={(workspaceId) =>
              void server.togglePin(workspaceId).catch(() => undefined)
            }
            onRemoveWorkspace={(workspaceId) =>
              void handleRemoveWorkspace(workspaceId)
            }
            mobileOpen={mobileLayout && workspaceOpen}
            onCloseMobile={closeWorkspaceDrawer}
            onOpenSettings={() => {
              setWorkspaceOpen(false);
              routing.openSettings("general");
            }}
          />

          <main
            className="product-main"
            inert={mobileLayout && (workspaceOpen || !inspectorCollapsed) ? true : undefined}
          >
            {server.catalogError ? (
              <div className="shell-alert" role="alert">
                {t("chrome.settingsError")}
              </div>
            ) : null}
            {routing.routeError ? (
              <RouteErrorView
                error={routing.routeError}
                onReturn={routing.returnHome}
              />
            ) : routing.routePending ? (
              <RouteLoadingView />
            ) : !activeWorkspace ? (
              <EmptyState
                recents={workspaces.slice(0, 6)}
                onOpenWorkspace={(path, kind) => void handleOpenWorkspace(path, kind)}
                onOpenRecent={routing.navigateWorkspace}
                onOpenProviders={() => routing.openSettings("providers")}
              />
            ) : !activeSession ? (
              <WorkspaceSessionEmpty
                workspaceName={activeWorkspace.displayName}
                onNewSession={() => void handleNewSession(activeWorkspace.id)}
              />
            ) : (
              <div className="chat-pane">
                <div className="chat-pane__header">
                  <div>
                    <h1>{activeSession.title}</h1>
                    <p>
                      {activeWorkspace.displayName} / {formatDisplayPath(activeWorkspace.rootPath)}
                    </p>
                  </div>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => void handleForkSession()}
                    disabled={!forkAvailable}
                  >
                    Fork
                  </button>
                  <button
                    ref={inspectorButtonRef}
                    type="button"
                    className="secondary"
                    onClick={panel.toggle}
                    aria-label={
                      inspectorCollapsed
                        ? t("nav.expandInspector")
                        : t("nav.collapseInspector")
                    }
                  >
                    {inspectorCollapsed
                      ? t("inspector.tabRun")
                      : t("common.close")}
                  </button>
                </div>
                <Transcript
                  timeline={selectTranscriptTimeline(continuity.runState)}
                  messages={continuity.messages}
                  messageBusy={continuity.controlBusy}
                  canPromote={controlAvailable}
                  approvalBusy={continuity.approvalBusy}
                  approvalError={continuity.approvalError}
                  onApprovalDetail={(tool, trigger) => {
                    const { activeJobId, activeRunId } = continuity.runState;
                    if (activeJobId && activeRunId) panel.open("approval", {
                      kind: "approval", jobId: activeJobId, runId: activeRunId, callId: tool.id,
                    }, trigger);
                  }}
                  inputBusy={continuity.inputBusy}
                  restoreState={transcriptRestoreState}
                  onRetryRestore={() =>
                    continuity.retryRestore(activeWorkspace.id, activeSession.id)
                  }
                  onStartNewSession={() =>
                    void handleNewSession(activeWorkspace.id)
                  }
                  onApproval={continuity.approve}
                  onInputSubmit={continuity.answer}
                  onPromoteMessage={(messageId) => void continuity.promoteMessage(messageId)}
                  onRevokeMessage={(messageId) => void continuity.revokeMessage(messageId)}
                />
                <Composer
                  draftBinding={{
                    store: draftStore,
                    workspaceId: activeWorkspace.id,
                    productSessionId: activeSession.id,
                  }}
                  disabled={composerDisabled}
                  busy={busy}
                  disabledReason={composerDisabledReason}
                  error={continuity.runState.error}
                  profiles={server.profiles}
                  modelConfig={server.sessionModelConfig}
                  modelConfigSaving={server.sessionModelConfigMutationBusy}
                  textareaRef={composerRef}
                  onSend={continuity.send}
                  onCancel={() => void continuity.cancel()}
                  onLoadProviderModels={server.productClient.listProviderModels}
                  onModelConfigChange={server.changeSessionModelConfig}
                  controlError={continuity.controlError}
                  reviewAvailable={activeWorkspace.kind === "repo"}
                  reviewBusy={reviews.creating}
                  reviewError={reviews.error}
                  onCreateReview={async (target) => {
                    const created = await reviews.create(target);
                    if (created) {
                      panel.open("review");
                    }
                    return created;
                  }}
                />
              </div>
            )}
          </main>

          {activeWorkspace &&
          activeSession &&
          !routing.routeError &&
          !routing.routePending ? (
            <RunInspector
              key={`${activeWorkspace.id}:${activeSession.id}`}
              panel={panel}
              approvalBusy={continuity.approvalBusy}
              approvalError={continuity.approvalError}
              onApproval={continuity.approve}
              productSessionId={activeSession.id}
              workspaceId={activeWorkspace.id}
              collapsed={inspectorCollapsed}
              onToggle={panel.toggle}
              runState={continuity.runState}
              restoreState={transcriptRestoreState}
              sessionUsage={sessionUsage}
              dialogOpen={mobileLayout && !inspectorCollapsed}
              reviews={reviews.reviews}
              selectedReviewId={reviews.selectedReviewId}
              selectedReview={reviews.selectedReview}
              reviewFindings={reviews.findings}
              reviewFindingsCursor={reviews.findingsCursor}
              reviewFindingsLoading={reviews.findingsLoading}
              reviewsLoading={reviews.loading}
              reviewError={reviews.error}
              onSelectReview={reviews.select}
              onRefreshReviews={() => void reviews.refresh()}
              onCancelReview={(reviewId) => void reviews.cancel(reviewId)}
              onLoadReviewFindings={(reviewId, cursor) => {
                void reviews.loadFindings(reviewId, cursor);
              }}
              onOpenReviewFinding={(path, line) => {
                panel.setTarget({ kind: "file", path, line });
              }}
              fileFocusPath={panel.target?.kind === "file" ? panel.target.path : undefined}
              fileFocusLine={panel.target?.kind === "file" ? panel.target.line : undefined}
            />
          ) : (
            <div />
          )}
          {mobileLayout && (workspaceOpen || !inspectorCollapsed) ? (
            <button
              type="button"
              className="product-mobile-scrim"
              aria-label={t("common.close")}
              tabIndex={-1}
              onClick={workspaceOpen ? closeWorkspaceDrawer : closeInspector}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}

function BootStateView({
  state,
  onRetry,
}: {
  state: Exclude<ProductBootState, { status: "ready" }>;
  onRetry: () => void;
}) {
  const { t } = useCopy();
  return (
    <main className="boot-state" role={state.status === "error" ? "alert" : "status"}>
      <h1>
        {state.status === "loading"
          ? t("boot.loading")
          : t("errors.bootTitle")}
      </h1>
      <p>
        {state.status === "loading"
          ? t("boot.checking")
          : state.error}
      </p>
      {state.status === "error" ? (
        <button type="button" onClick={onRetry}>
          {t("common.retry")}
        </button>
      ) : null}
    </main>
  );
}

function RouteLoadingView() {
  const { t } = useCopy();
  return (
    <section className="route-state" role="status">
      <h1>{t("boot.loading")}</h1>
      <p>{t("boot.checking")}</p>
    </section>
  );
}

function RouteErrorView({
  error,
  onReturn,
}: {
  error: string;
  onReturn: () => void;
}) {
  const { t } = useCopy();
  return (
    <section className="route-state" role="alert">
      <h1>{t("errors.routeTitle")}</h1>
      <p>{error}</p>
      <p className="error-code">{t("errors.code", { code: "route-unavailable" })}</p>
      <button type="button" onClick={onReturn}>
        {t("nav.backToChat")}
      </button>
    </section>
  );
}

function WorkspaceSessionEmpty({
  workspaceName,
  onNewSession,
}: {
  workspaceName: string;
  onNewSession: () => void;
}) {
  const { t } = useCopy();
  return (
    <section className="route-state">
      <h1>{workspaceName}</h1>
      <p>{t("chat.startNewSession")}</p>
      <button type="button" onClick={onNewSession}>
        {t("nav.newSession")}
      </button>
    </section>
  );
}
