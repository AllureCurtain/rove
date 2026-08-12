"use client";

import { useEffect, useMemo, useRef, useState } from "react";

import { Composer } from "../chat/Composer";
import { Transcript } from "../chat/Transcript";
import { RunInspector } from "../inspector/RunInspector";
import { useSessionUsage } from "../state/use-session-usage";
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
import type { WorkspaceKind } from "../state/product-types";
import { M1MigrationGate } from "./M1MigrationGate";
import { TopBar } from "./TopBar";

export type ProductUiVersion = "v1" | "v2";

export function ProductApp({
  uiVersion = "v2",
}: {
  uiVersion?: ProductUiVersion;
}) {
  return (
    <div className="product-app-frame" data-ui-version={uiVersion}>
      <M1MigrationGate>
        <ServerProductApp uiVersion={uiVersion} />
      </M1MigrationGate>
    </div>
  );
}

function ServerProductApp({ uiVersion }: { uiVersion: ProductUiVersion }) {
  const server = useServerProductState();
  const settingsClient = useMemo(() => createSettingsPlatformClient(), []);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const workspaceButtonRef = useRef<HTMLButtonElement>(null);
  const inspectorButtonRef = useRef<HTMLButtonElement>(null);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [mobileLayout, setMobileLayout] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);

  useEffect(() => {
    const narrow = window.matchMedia("(max-width: 960px)");
    const syncInspector = () => {
      setMobileLayout(narrow.matches);
      setInspectorCollapsed(narrow.matches);
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
      ? "API connected"
      : server.connection === "error"
        ? "API unreachable"
        : "Checking API...";
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
    ? "Restoring canonical history before a new turn."
    : continuity.restoreState.status === "loading"
      ? "Restoring canonical history before a new turn."
    : continuity.restoreState.status === "error"
      ? "Retry transcript restore before sending."
      : server.sessionModelConfigLoading || server.sessionModelConfig === null
        ? "Loading session model settings."
      : routing.routeError
              ? "Resolve the product route before sending."
              : undefined;
  const resumeLabel = activeSession?.hasDurableTurn
    ? "continuity: exact product session"
    : "first turn: server-bound session";

  function closeWorkspaceDrawer() {
    setWorkspaceOpen(false);
    window.requestAnimationFrame(() => workspaceButtonRef.current?.focus());
  }

  function closeInspector() {
    setInspectorCollapsed(true);
    if (mobileLayout) {
      window.requestAnimationFrame(() => inspectorButtonRef.current?.focus());
    }
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
            setInspectorCollapsed((value) => !value);
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
                {server.catalogError}
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
                      {activeWorkspace.displayName} / {activeWorkspace.rootPath}
                    </p>
                  </div>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => void handleForkSession()}
                    disabled={!forkAvailable}
                    title={
                      forkAvailable
                        ? "Create an independent branch from this completed turn"
                        : "Fork is available after this session reaches a completed turn"
                    }
                  >
                    Fork
                  </button>
                  <button
                    ref={inspectorButtonRef}
                    type="button"
                    className="secondary"
                    onClick={() => setInspectorCollapsed((value) => !value)}
                    aria-label={inspectorCollapsed ? "Open run evidence" : "Close run evidence"}
                  >
                    {inspectorCollapsed ? "Evidence" : "Close evidence"}
                  </button>
                </div>
                <Transcript
                  timeline={selectTranscriptTimeline(continuity.runState)}
                  messages={continuity.messages}
                  messageBusy={continuity.controlBusy}
                  canPromote={controlAvailable}
                  approvalBusy={continuity.approvalBusy}
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
                  disabled={composerDisabled}
                  busy={busy}
                  resumeLabel={resumeLabel}
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
                />
              </div>
            )}
          </main>

          {activeWorkspace &&
          activeSession &&
          !routing.routeError &&
          !routing.routePending ? (
            <RunInspector
              productSessionId={activeSession.id}
              workspaceId={activeWorkspace.id}
              collapsed={inspectorCollapsed}
              onToggle={() => {
                if (!inspectorCollapsed) {
                  closeInspector();
                } else {
                  setInspectorCollapsed(false);
                }
              }}
              runState={continuity.runState}
              restoreState={transcriptRestoreState}
              sessionUsage={sessionUsage}
              dialogOpen={mobileLayout && !inspectorCollapsed}
            />
          ) : (
            <div />
          )}
          {mobileLayout && (workspaceOpen || !inspectorCollapsed) ? (
            <button
              type="button"
              className="product-mobile-scrim"
              aria-label="Close open panel"
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
  return (
    <main className="boot-state" role={state.status === "error" ? "alert" : "status"}>
      <h1>
        {state.status === "loading"
          ? "Loading product state"
          : "Product state unavailable"}
      </h1>
      <p>
        {state.status === "loading"
          ? "Reading server workspaces, sessions, and preferences."
          : state.error}
      </p>
      {state.status === "error" ? (
        <button type="button" onClick={onRetry}>
          Retry
        </button>
      ) : null}
    </main>
  );
}

function RouteLoadingView() {
  return (
    <section className="route-state" role="status">
      <h1>Opening product route</h1>
      <p>Matching the server catalog to this workspace and session.</p>
    </section>
  );
}

function RouteErrorView({ error, onReturn }: { error: string; onReturn: () => void }) {
  return (
    <section className="route-state" role="alert">
      <h1>Route unavailable</h1>
      <p>{error}</p>
      <button type="button" onClick={onReturn}>
        Return to product home
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
  return (
    <section className="route-state">
      <h1>{workspaceName}</h1>
      <p>This workspace has no active sessions.</p>
      <button type="button" onClick={onNewSession}>
        New session
      </button>
    </section>
  );
}
