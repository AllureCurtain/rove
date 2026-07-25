"use client";

import { useEffect, useMemo, useReducer, useRef, useState } from "react";

import { createRunController, describeError } from "../api/run-controller";
import { Composer } from "../chat/Composer";
import { Transcript } from "../chat/Transcript";
import { RunInspector } from "../inspector/RunInspector";
import {
  createWorkbenchState,
  workbenchReducer,
  type ToolCallView,
} from "../lib/rove-state";
import { applyDocumentTheme, webPlatform } from "../platform/web";
import { SettingsShell } from "../settings/SettingsShell";
import type { SettingsSectionId } from "../settings/sections";
import { TopBar } from "../shell/TopBar";
import { EmptyState } from "../sidebar/EmptyState";
import { WorkspaceTree } from "../sidebar/WorkspaceTree";
import {
  createSession,
  ensureActiveSession,
  findSession,
  findWorkspace,
  loadProductCatalog,
  openWorkspace,
  removeWorkspace,
  saveProductCatalog,
  selectSession,
  selectWorkspace,
  sessionsForWorkspace,
  sortedWorkspaces,
  togglePinWorkspace,
  updateSession,
  type ProductCatalog,
} from "../state/product-catalog";
import type { WorkspaceKind } from "../state/product-types";
import {
  loadProviderProfiles,
  loadProviderSelection,
  saveProviderProfiles,
  saveProviderSelection,
} from "../state/provider-store";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
} from "../state/product-types";
import { buildTurnJobRequest, isHardResumeError } from "../state/turn-request";

type ViewMode = "chat" | "settings";

export function ProductApp() {
  const [catalog, setCatalog] = useState<ProductCatalog>(() => loadProductCatalog());
  const [profiles, setProfiles] = useState<ProviderProfileRecord[]>(() =>
    loadProviderProfiles(),
  );
  const [selection, setSelection] = useState<ActiveProviderSelection>(() =>
    loadProviderSelection(),
  );
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    webPlatform.resolveTheme(webPlatform.getThemePreference()),
  );
  const [view, setView] = useState<ViewMode>("chat");
  const [settingsSection, setSettingsSection] = useState<SettingsSectionId>("providers");
  const [inspectorCollapsed, setInspectorCollapsed] = useState(false);
  const [connection, setConnection] = useState<"unknown" | "ok" | "error">("unknown");
  const [approvalBusy, setApprovalBusy] = useState<string | null>(null);
  const [inputBusy, setInputBusy] = useState<string | null>(null);
  const [runState, dispatch] = useReducer(
    workbenchReducer,
    undefined,
    createWorkbenchState,
  );
  const controllerRef = useRef<ReturnType<typeof createRunController> | null>(null);
  const catalogRef = useRef(catalog);

  useEffect(() => {
    catalogRef.current = catalog;
    saveProductCatalog(catalog);
  }, [catalog]);

  useEffect(() => {
    saveProviderProfiles(profiles);
  }, [profiles]);

  useEffect(() => {
    saveProviderSelection(selection);
  }, [selection]);

  useEffect(() => {
    applyDocumentTheme(theme);
    webPlatform.setThemePreference(theme);
  }, [theme]);

  useEffect(() => {
    let cancelled = false;
    void fetch("/api/runs?limit=1")
      .then((response) => {
        if (cancelled) {
          return;
        }
        setConnection(response.ok ? "ok" : "error");
      })
      .catch(() => {
        if (!cancelled) {
          setConnection("error");
        }
      });
    return () => {
      cancelled = true;
      controllerRef.current?.close();
    };
  }, []);

  const activeWorkspace = findWorkspace(catalog, catalog.active.workspaceId);
  const activeSession = findSession(catalog, catalog.active.sessionId);
  const workspaces = sortedWorkspaces(catalog);
  const sessionsByWorkspace = useMemo(() => {
    const map: Record<string, ReturnType<typeof sessionsForWorkspace>> = {};
    for (const workspace of catalog.workspaces) {
      map[workspace.id] = sessionsForWorkspace(catalog, workspace.id);
    }
    return map;
  }, [catalog]);

  const connectionLabel =
    connection === "ok"
      ? "API connected"
      : connection === "error"
        ? "API unreachable"
        : "Checking API…";
  const connectionTone =
    connection === "ok" ? "ok" : connection === "error" ? "error" : "idle";

  function patchCatalog(updater: (current: ProductCatalog) => ProductCatalog) {
    setCatalog((current) => {
      const next = updater(current);
      catalogRef.current = next;
      return next;
    });
  }

  function handleOpenWorkspace(path: string, kind: WorkspaceKind) {
    try {
      patchCatalog((current) => {
        const opened = openWorkspace(current, path, kind);
        const withSession = ensureActiveSession(opened, opened.active.workspaceId!);
        return withSession;
      });
      dispatch({ type: "reset" });
      setView("chat");
    } catch (error) {
      dispatch({ type: "set_error", error: describeError(error) });
    }
  }

  function handleSelectWorkspace(workspaceId: string) {
    controllerRef.current?.close();
    patchCatalog((current) => ensureActiveSession(selectWorkspace(current, workspaceId), workspaceId));
    dispatch({ type: "reset" });
  }

  function handleSelectSession(workspaceId: string, sessionId: string) {
    controllerRef.current?.close();
    patchCatalog((current) => selectSession(current, workspaceId, sessionId));
    dispatch({ type: "reset" });
  }

  function handleNewSession(workspaceId: string) {
    controllerRef.current?.close();
    patchCatalog((current) => createSession(current, workspaceId));
    dispatch({ type: "reset" });
  }

  function markSession(
    sessionId: string,
    patch: Parameters<typeof updateSession>[2],
  ) {
    patchCatalog((current) => updateSession(current, sessionId, patch));
  }

  async function handleSend(message: string) {
    const workspace = findWorkspace(catalogRef.current, catalogRef.current.active.workspaceId);
    const session = findSession(catalogRef.current, catalogRef.current.active.sessionId);
    if (!workspace || !session) {
      dispatch({ type: "set_error", error: "Open a workspace and session first." });
      return;
    }

    controllerRef.current?.close();
    // Keep transcript across turns; clear only run-scoped inspector state.
    dispatch({ type: "prepare_turn" });
    dispatch({ type: "append_user_message", content: message });
    dispatch({ type: "set_status", statusText: "Submitting job" });

    const request = buildTurnJobRequest({
      message,
      workspace,
      session,
      selection,
      profiles,
    });

    const controller = createRunController(dispatch, {
      onTerminal: () => {
        const latest = findSession(catalogRef.current, session.id);
        if (!latest) {
          return;
        }
        // A finished turn under this workspace is durable for hard resume.
        markSession(session.id, {
          status: "idle",
          hasDurableTurn: true,
        });
      },
    });
    controllerRef.current = controller;

    markSession(session.id, {
      status: "running",
      title: session.title === "New session" ? truncateTitle(message) : session.title,
    });

    try {
      const started = await controller.start(request);
      markSession(session.id, {
        activeJobId: started.jobId,
        activeRunId: started.runId,
        resumedFromRunId: started.resumedFromRunId,
        // First successful create under resume path still counts once terminal;
        // keep running until terminal callback.
        status: "running",
      });
      if (session.hasDurableTurn && !started.resumedFromRunId) {
        // API accepted resume:latest but did not report a source run — still OK if
        // runtime bound state; surface soft notice only when body implies fresh.
        // Hard failures throw below.
      }
      setConnection("ok");
    } catch (error) {
      const messageText = describeError(error);
      const hard = session.hasDurableTurn || isHardResumeError(messageText);
      dispatch({
        type: "set_error",
        error: hard
          ? `Hard resume failed: ${messageText}. Do not continue as a disconnected one-shot. Fix runtime state or start a new session.`
          : messageText,
      });
      markSession(session.id, { status: "error" });
      setConnection(
        messageText.toLowerCase().includes("fetch") || messageText.toLowerCase().includes("network")
          ? "error"
          : connection === "ok"
            ? "ok"
            : connection,
      );
    }
  }

  async function handleCancel() {
    if (!runState.activeJobId) {
      return;
    }
    try {
      await controllerRef.current?.cancel(runState.activeJobId);
      if (activeSession) {
        markSession(activeSession.id, { status: "idle" });
      }
    } catch (error) {
      dispatch({ type: "set_error", error: describeError(error) });
    }
  }

  async function handleApproval(tool: ToolCallView, decision: "approve" | "reject") {
    if (!runState.activeJobId || !tool.pendingApproval) {
      return;
    }
    setApprovalBusy(tool.id);
    try {
      await controllerRef.current?.approve(runState.activeJobId, tool.id, decision);
      if (activeSession) {
        markSession(activeSession.id, {
          status: decision === "reject" ? "idle" : "running",
        });
      }
    } catch (error) {
      dispatch({ type: "set_error", error: describeError(error) });
    } finally {
      setApprovalBusy(null);
    }
  }

  async function handleInputSubmit(inputId: string, answer: string) {
    if (!runState.activeJobId) {
      return;
    }
    setInputBusy(inputId);
    try {
      await controllerRef.current?.answer(runState.activeJobId, inputId, answer);
    } catch (error) {
      dispatch({ type: "set_error", error: describeError(error) });
    } finally {
      setInputBusy(null);
    }
  }

  const busy = runState.busy;
  const resumeLabel = activeSession?.hasDurableTurn
    ? "next turn: hard resume (latest)"
    : "first turn: fresh job";

  return (
    <div className="product-root">
      <TopBar
        connectionLabel={connectionLabel}
        connectionTone={
          busy ? "working" : runState.error ? "error" : connectionTone
        }
        theme={theme}
        onToggleTheme={() => setTheme((current) => (current === "dark" ? "light" : "dark"))}
        onOpenSettings={() => {
          setSettingsSection("providers");
          setView("settings");
        }}
        showSettingsBack={view === "settings"}
        onBackToChat={() => setView("chat")}
      />

      {view === "settings" ? (
        <div className="product-body" data-settings="true">
          <SettingsShell
            section={settingsSection}
            onSectionChange={setSettingsSection}
            profiles={profiles}
            selection={selection}
            onProfilesChange={setProfiles}
            onSelectionChange={setSelection}
            connectionLabel={connectionLabel}
            theme={theme}
            onThemeChange={setTheme}
          />
        </div>
      ) : (
        <div className="product-body">
          <WorkspaceTree
            workspaces={workspaces}
            sessionsByWorkspace={sessionsByWorkspace}
            activeWorkspaceId={catalog.active.workspaceId}
            activeSessionId={catalog.active.sessionId}
            onOpenWorkspace={handleOpenWorkspace}
            onSelectWorkspace={handleSelectWorkspace}
            onSelectSession={handleSelectSession}
            onNewSession={handleNewSession}
            onTogglePin={(workspaceId) =>
              patchCatalog((current) => togglePinWorkspace(current, workspaceId))
            }
            onRemoveWorkspace={(workspaceId) =>
              patchCatalog((current) => removeWorkspace(current, workspaceId))
            }
          />

          <main className="product-main">
            {!activeWorkspace || !activeSession ? (
              <EmptyState
                recents={workspaces.slice(0, 6)}
                onOpenWorkspace={handleOpenWorkspace}
                onOpenRecent={(workspaceId) => handleSelectWorkspace(workspaceId)}
                onOpenProviders={() => {
                  setSettingsSection("providers");
                  setView("settings");
                }}
              />
            ) : (
              <div className="chat-pane">
                <div className="chat-pane__header">
                  <div>
                    <h1>{activeSession.title}</h1>
                    <p>
                      {activeWorkspace.displayName} · {activeWorkspace.rootPath}
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
                  messages={runState.messages}
                  tools={runState.tools}
                  pendingInputs={runState.pendingInputs}
                  approvalBusy={approvalBusy}
                  inputBusy={inputBusy}
                  onApproval={handleApproval}
                  onInputSubmit={handleInputSubmit}
                />
                <Composer
                  disabled={false}
                  busy={busy}
                  modelLabel={`model ${selection.model || "default"}`}
                  resumeLabel={resumeLabel}
                  error={runState.error}
                  onSend={handleSend}
                  onCancel={handleCancel}
                />
              </div>
            )}
          </main>

          {activeWorkspace && activeSession ? (
            <RunInspector
              collapsed={inspectorCollapsed}
              onToggle={() => setInspectorCollapsed((value) => !value)}
              runState={runState}
            />
          ) : (
            <div />
          )}
        </div>
      )}
    </div>
  );
}

function truncateTitle(message: string): string {
  const compact = message.replace(/\s+/g, " ").trim();
  return compact.length <= 42 ? compact : `${compact.slice(0, 42)}…`;
}
