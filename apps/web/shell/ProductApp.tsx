"use client";

import { useEffect, useMemo, useRef, useState } from "react";

import { Composer } from "../chat/Composer";
import { Transcript } from "../chat/Transcript";
import { RunInspector } from "../inspector/RunInspector";
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
import { TopBar } from "./TopBar";

export function ProductApp() {
  const server = useServerProductState();
  const settingsClient = useMemo(() => createSettingsPlatformClient(), []);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(false);
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
    useDefaultApproval: server.preferences?.provider_selection === undefined,
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
  const awaitingInitialRestore =
    routing.route.kind === "session" &&
    activeSession?.id === routing.route.sessionId &&
    continuity.restoreState.status === "idle";
  const transcriptRestoreState = awaitingInitialRestore
    ? ({ status: "loading", sessionId: activeSession.id } as const)
    : continuity.restoreState;
  const composerDisabled =
    awaitingInitialRestore ||
    continuity.restoreState.status === "loading" ||
    continuity.restoreState.status === "error" ||
    activeSession?.status === "running" ||
    activeSession?.status === "needs_attention" ||
    routing.routeError !== null;
  const resumeLabel = activeSession?.hasDurableTurn
    ? "continuity: exact product session"
    : "first turn: server-bound session";

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
      <div className="product-root">
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
    <div className="product-root">
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
        <div className="product-body">
          <WorkspaceTree
            workspaces={workspaces}
            sessionsByWorkspace={sessionsByWorkspace}
            activeWorkspaceId={server.catalog.active.workspaceId}
            activeSessionId={server.catalog.active.sessionId}
            mutationBusy={server.catalogMutationBusy}
            onOpenWorkspace={(path, kind) => void handleOpenWorkspace(path, kind)}
            onSelectWorkspace={routing.navigateWorkspace}
            onSelectSession={routing.navigateSession}
            onNewSession={(workspaceId) => void handleNewSession(workspaceId)}
            onTogglePin={(workspaceId) =>
              void server.togglePin(workspaceId).catch(() => undefined)
            }
            onRemoveWorkspace={(workspaceId) =>
              void handleRemoveWorkspace(workspaceId)
            }
          />

          <main className="product-main">
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
                    onClick={() => setInspectorCollapsed((value) => !value)}
                  >
                    {inspectorCollapsed ? "Show inspector" : "Hide inspector"}
                  </button>
                </div>
                <Transcript
                  messages={continuity.runState.messages}
                  tools={[
                    ...continuity.runState.tools,
                    ...continuity.runState.historicalTools,
                  ]}
                  pendingInputs={continuity.runState.pendingInputs}
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
                />
                <Composer
                  disabled={composerDisabled}
                  busy={busy}
                  modelLabel={`model ${server.selection.model || "default"}`}
                  resumeLabel={resumeLabel}
                  error={continuity.runState.error}
                  textareaRef={composerRef}
                  onSend={continuity.send}
                  onCancel={() => void continuity.cancel()}
                />
              </div>
            )}
          </main>

          {activeWorkspace &&
          activeSession &&
          !routing.routeError &&
          !routing.routePending ? (
            <RunInspector
              collapsed={inspectorCollapsed}
              onToggle={() => setInspectorCollapsed((value) => !value)}
              runState={continuity.runState}
            />
          ) : (
            <div />
          )}
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
