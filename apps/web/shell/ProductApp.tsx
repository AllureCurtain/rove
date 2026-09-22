"use client";

import { HamburgerMenuIcon } from "@radix-ui/react-icons";
import type { CSSProperties } from "react";

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
import {
  MAIN_PANE_MIN_WIDTH,
  workPanelLayout,
  workPanelWidthForSidebarReopen,
  type WorkPanelLayout,
} from "../inspector/work-panel-layout";
import { useSessionUsage } from "../state/use-session-usage";
import {
  createComposerDraftStore,
  type ComposerDraftStore,
} from "../state/composer-draft-store";
import { selectTranscriptTimeline } from "../lib/rove-state";
import { DRAWER_MEDIA_QUERY } from "../lib/viewport-breakpoints";
import {
  SIDEBAR_OVERLAY_CLOSE_DELAY_MS,
  shouldFocusSessionHeadingAfterSelection,
  shouldShowSidebarHoverZone,
} from "./sidebar-overlay";
import { CommandPalette, type CommandPaletteLabels } from "./CommandPalette";
import {
  type CommandPaletteAction,
  type CommandPaletteEntry,
} from "./command-palette";
import { routePrefetchTargets } from "./route-prefetch";
import { useIdleRoutePrefetch } from "./use-idle-route-prefetch";
import { SettingsShell } from "../settings/SettingsShell";
import { matchKeyboardShortcut } from "../settings/keyboard-settings-model";
import {
  KEYBOARD_SHORTCUTS,
  type KeyboardShortcutActionId,
} from "../settings/keyboard-settings-model";
import {
  SETTINGS_SECTION_COPY_KEYS,
  VISIBLE_SETTINGS_SECTIONS,
} from "../settings/sections";
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
import { SidebarResizeHandle } from "./SidebarResizeHandle";
import { useSidebarWidth } from "./use-sidebar-width";

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
  // Left navigation width and collapse are UI-only layout state (design 搂3.1).
  // The width persists as a UI preference; the collapse does not, so a reload
  // always returns to the expanded rail.
  const sidebar = useSidebarWidth();
  // Warm the routes the header can reach in one click, off the boot path
  // (open-vetta's idle prefetch; see `route-prefetch`).
  const prefetchTargets = useMemo(
    () =>
      routePrefetchTargets({
        activeWorkspaceId: server.catalog.active.workspaceId,
      }),
    [server.catalog.active.workspaceId],
  );
  useIdleRoutePrefetch(prefetchTargets);
  const [navCollapsed, setNavCollapsed] = useState(false);
  // Focus target for the narrow-screen "select session, close rail" flow.
  const sessionTitleRef = useRef<HTMLHeadingElement>(null);
  const panelRef = useRef(panel);
  const inspectorCollapsed = panel.collapsed;
  const [mobileLayout, setMobileLayout] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  // Narrow screen: a pointer at the left edge peeks the rail without turning it
  // into a modal dialog (open-vetta's hover-summoned overlay).
  const [workspacePeek, setWorkspacePeek] = useState(false);
  const peekCloseTimerRef = useRef<number | null>(null);
  // The command palette is shell chrome: open state only, the entries are data.
  const [commandOpen, setCommandOpen] = useState(false);

  // ── Shared three-column width budget (design §5.0, ported from PI-Desktop) ──
  const [shellNode, setShellNode] = useState<HTMLDivElement | null>(null);
  const [shellWidth, setShellWidth] = useState(0);
  // The live measurement, for event handlers that must not read a stale render.
  const shellWidthRef = useRef(0);
  // The budget inputs and the layout they produced. The shell is a parent of the
  // whole conversation, so a resize that does not move the budget must not
  // re-render: the visible geometry stays exact anyway because the track is
  // `min(panelWidth, calc(100% - floor))` and CSS evaluates `100%` live.
  const geometryRef = useRef<{
    sidebarWidth: number;
    navCollapsed: boolean;
    requestedPanelWidth: number;
    layout: WorkPanelLayout | null;
  }>({
    sidebarWidth: sidebar.width,
    navCollapsed: false,
    requestedPanelWidth: panel.width,
    layout: null,
  });
  // A callback ref rather than a mount-time effect: `.product-body` only exists
  // once the boot state resolves, so an effect with `[]` dependencies could run
  // against a null ref and never observe the shell.
  useEffect(() => {
    if (!shellNode) {
      return;
    }
    const sync = () => {
      const next = shellNode.clientWidth;
      const snapshot = geometryRef.current;
      const layout = workPanelLayout({
        containerWidth: next,
        sidebarWidth: snapshot.sidebarWidth,
        sidebarCollapsed: snapshot.navCollapsed,
        requestedPanelWidth: snapshot.requestedPanelWidth,
      });
      const previous = snapshot.layout;
      shellWidthRef.current = next;
      const unchanged =
        previous !== null &&
        previous.panelWidth === layout.panelWidth &&
        previous.maxPanelWidth === layout.maxPanelWidth &&
        previous.mainWidth === layout.mainWidth &&
        previous.shouldCollapseSidebar === layout.shouldCollapseSidebar;
      if (unchanged) {
        return;
      }
      setShellWidth(next);
    };
    sync();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", sync);
      return () => window.removeEventListener("resize", sync);
    }
    const observer = new ResizeObserver(sync);
    observer.observe(shellNode);
    return () => observer.disconnect();
  }, [shellNode]);
  // The first render precedes the observer's first notification, so it uses the
  // same conservative estimate PI-Desktop uses: the request plus the rail plus
  // the conversation floor.
  const measuredShellWidth =
    shellWidth > 0
      ? shellWidth
      : panel.width + (navCollapsed ? 0 : sidebar.width) + MAIN_PANE_MIN_WIDTH;
  const panelLayout = useMemo(
    () =>
      workPanelLayout({
        containerWidth: measuredShellWidth,
        sidebarWidth: sidebar.width,
        sidebarCollapsed: navCollapsed,
        requestedPanelWidth: panel.width,
      }),
    [measuredShellWidth, navCollapsed, panel.width, sidebar.width],
  );
  // The rail yields first: when the panel request cannot coexist with the
  // conversation floor, collapse the rail instead of squeezing the chat. The
  // effect settles in one step because collapsing removes the condition.
  useEffect(() => {
    if (!mobileLayout && shellWidth > 0 && panelLayout.shouldCollapseSidebar) {
      setNavCollapsed(true);
    }
  }, [mobileLayout, panelLayout.shouldCollapseSidebar, shellWidth]);

  // Keep the observer's snapshot of the budget inputs current. Declared after
  // `panelLayout` so it records the layout this render committed.
  useEffect(() => {
    geometryRef.current = {
      sidebarWidth: sidebar.width,
      navCollapsed,
      requestedPanelWidth: panel.width,
      layout: panelLayout,
    };
  });

  /**
   * Reopening the rail spends the panel's column instead of squeezing the
   * conversation: the panel gives up space first (PI-Desktop
   * `workPanelWidthForSidebarReopen`).
   */
  function expandRail() {
    panel.setWidth(
      workPanelWidthForSidebarReopen({
        containerWidth: shellWidthRef.current || measuredShellWidth,
        sidebarWidth: sidebar.width,
        currentPanelWidth: panel.width,
      }),
    );
    setNavCollapsed(false);
  }

  // Keep a stable ref to the panel for the *current* workspace/session. The
  // media-query listener below is registered once, so it must not close over
  // the panel object from the first render (that would drive another
  // session's cached selection instead of the one being viewed).
  useEffect(() => {
    panelRef.current = panel;
  });

  useEffect(() => {
    const narrow = window.matchMedia(DRAWER_MEDIA_QUERY);
    const syncInspector = () => {
      setMobileLayout(narrow.matches);
      panelRef.current.setCollapsed(narrow.matches);
      if (!narrow.matches) {
        // The peek only exists on a narrow screen.
        cancelPeekClose();
        setWorkspacePeek(false);
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

  function cancelPeekClose() {
    if (peekCloseTimerRef.current !== null) {
      window.clearTimeout(peekCloseTimerRef.current);
      peekCloseTimerRef.current = null;
    }
  }

  /** The pointer reached the edge: show the rail without taking focus. */
  function openWorkspacePeek() {
    cancelPeekClose();
    setWorkspacePeek(true);
  }

  /** The pointer left: hide shortly after, so crossing a gap does not flicker. */
  function schedulePeekClose() {
    cancelPeekClose();
    peekCloseTimerRef.current = window.setTimeout(() => {
      peekCloseTimerRef.current = null;
      setWorkspacePeek(false);
    }, SIDEBAR_OVERLAY_CLOSE_DELAY_MS);
  }

  function closeWorkspaceDrawer() {
    cancelPeekClose();
    setWorkspacePeek(false);
    setWorkspaceOpen(false);
    window.requestAnimationFrame(() => workspaceButtonRef.current?.focus());
  }

  // A pending peek timer must not outlive the shell.
  useEffect(() => cancelPeekClose, []);

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

  /**
   * One implementation of "what a shortcut does", shared by the global key
   * listener and the command palette so the two can never drift.
   */
  function runShortcut(action: KeyboardShortcutActionId): boolean {
    switch (action) {
      case "focus-composer":
        if (routing.viewSettings || composerDisabled || !composerRef.current) {
          return false;
        }
        composerRef.current.focus();
        return true;
      case "new-session":
        if (!activeWorkspace || server.catalogMutationBusy) {
          return false;
        }
        void handleNewSession(activeWorkspace.id);
        return true;
      case "open-settings":
        routing.openSettings("general");
        return true;
      case "toggle-inspector":
        if (routing.viewSettings || !activeWorkspace || !activeSession) {
          return false;
        }
        panel.toggle();
        return true;
      case "open-command-palette":
        setCommandOpen((open) => !open);
        return true;
    }
  }

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent) {
      const shortcut = matchKeyboardShortcut(event);
      if (!shortcut) {
        return;
      }
      if (runShortcut(shortcut.action)) {
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

  const commandLabels = useMemo<CommandPaletteLabels>(
    () => ({
      title: t("commandPalette.title"),
      placeholder: t("commandPalette.placeholder"),
      empty: t("commandPalette.empty"),
      groups: {
        actions: t("commandPalette.groupActions"),
        workspaces: t("commandPalette.groupWorkspaces"),
        sessions: t("commandPalette.groupSessions"),
        settings: t("commandPalette.groupSettings"),
      },
    }),
    [t],
  );

  // Entries are data, so the palette stays a pure view and this is the only place
  // that knows how to build them from the catalog and the shortcut registry.
  const commandEntries = useMemo<CommandPaletteEntry[]>(() => {
    const actionTitles: Record<KeyboardShortcutActionId, string> = {
      "focus-composer": t("commandPalette.actionFocusComposer"),
      "new-session": t("commandPalette.actionNewSession"),
      "open-settings": t("commandPalette.actionOpenSettings"),
      "toggle-inspector": t("commandPalette.actionToggleInspector"),
      "open-command-palette": t("commandPalette.title"),
    };
    const entries: CommandPaletteEntry[] = [];

    for (const descriptor of KEYBOARD_SHORTCUTS) {
      // The palette is not an entry inside itself.
      if (descriptor.action === "open-command-palette") {
        continue;
      }
      const unavailable =
        descriptor.action === "new-session" && !activeWorkspace
          ? t("commandPalette.unavailableNoWorkspace")
          : descriptor.action === "toggle-inspector" &&
              (!activeWorkspace || !activeSession)
            ? t("commandPalette.unavailableNoSession")
            : descriptor.action === "focus-composer" &&
                (routing.viewSettings || composerDisabled)
              ? t("commandPalette.unavailableComposer")
              : undefined;
      entries.push({
        id: `action:${descriptor.action}`,
        groupKey: "actions",
        title: actionTitles[descriptor.action],
        subtitle: descriptor.display,
        // A command that cannot run right now is shown and disabled rather than
        // dropped: "it is not here" is a worse answer than "not yet".
        disabled: unavailable !== undefined,
        disabledReason: unavailable,
        order: entries.length,
        action: { kind: "shortcut", action: descriptor.action },
      });
    }
    entries.push({
      id: "action:toggle-theme",
      groupKey: "actions",
      title: t("commandPalette.actionToggleTheme"),
      order: entries.length,
      action: { kind: "toggle-theme" },
    });

    workspaces.forEach((workspace, index) => {
      entries.push({
        id: `workspace:${workspace.id}`,
        groupKey: "workspaces",
        title: workspace.displayName,
        subtitle: formatDisplayPath(workspace.rootPath),
        order: index,
        action: { kind: "open-workspace", workspaceId: workspace.id },
      });
    });

    const workspaceNames = new Map(
      server.catalog.workspaces.map((workspace) => [
        workspace.id,
        workspace.displayName,
      ]),
    );
    server.catalog.sessions.forEach((session, index) => {
      entries.push({
        id: `session:${session.id}`,
        groupKey: "sessions",
        title: session.title,
        subtitle: workspaceNames.get(session.workspaceId) ?? "",
        order: index,
        action: {
          kind: "open-session",
          workspaceId: session.workspaceId,
          sessionId: session.id,
        },
      });
    });

    VISIBLE_SETTINGS_SECTIONS.forEach((section, index) => {
      entries.push({
        id: `settings:${section.id}`,
        groupKey: "settings",
        title: t(SETTINGS_SECTION_COPY_KEYS[section.id]),
        order: index,
        action: { kind: "open-settings-section", section: section.id },
      });
    });

    return entries;
  }, [
    activeSession,
    activeWorkspace,
    composerDisabled,
    routing.viewSettings,
    server.catalog,
    t,
    workspaces,
  ]);

  /** The single place that turns a palette action into navigation or a command. */
  function handleCommandAction(action: CommandPaletteAction) {
    switch (action.kind) {
      case "shortcut":
        runShortcut(action.action);
        return;
      case "toggle-theme":
        server.changeTheme(server.theme === "dark" ? "light" : "dark");
        return;
      case "open-settings-section":
        routing.openSettings(action.section);
        return;
      case "open-workspace":
        routing.navigateWorkspace(action.workspaceId);
        return;
      case "open-session":
        routing.navigateSession(action.workspaceId, action.sessionId);
        return;
    }
  }

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
            : () => {
                // A deliberate open is the modal one: it may trap focus, and it
                // hands focus back to this button when it closes.
                cancelPeekClose();
                setWorkspacePeek(false);
                setWorkspaceOpen((value) => !value);
              }
        }
        // When the rail is collapsed its entries move to the header so they
        // stay reachable (design 搂3.1).
        collapsedActions={
          navCollapsed && !routing.viewSettings ? (
            <>
              <button
                type="button"
                className="secondary"
                onClick={() => {
                  if (server.catalog.active.workspaceId) {
                    void handleNewSession(server.catalog.active.workspaceId);
                  } else {
                    expandRail();
                  }
                }}
              >
                {t("nav.newSession")}
              </button>
              <button
                type="button"
                className="ghost icon-button"
                onClick={expandRail}
                aria-label={t("nav.expandWorkspace")}
                title={t("nav.expandWorkspace")}
              >
                <HamburgerMenuIcon />
              </button>
            </>
          ) : null
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
          ref={setShellNode}
          data-workspace-open={workspaceOpen}
          data-inspector-open={mobileLayout && !inspectorCollapsed}
          data-nav-collapsed={navCollapsed}
          style={
            {
              "--sidebar-nav-width": `${sidebar.width}px`,
              // Design §5.0 shared width budget: `workPanelLayout` owns the
              // answer (it knows the rail's actual state), and the CSS keeps a
              // viewport-level safety net against a stale measurement. Deriving
              // the cap from the rail's *expanded* width here instead would let
              // CSS and JS disagree whenever the rail is collapsed.
              "--work-panel-track": inspectorCollapsed
                ? "minmax(0, var(--work-panel-collapsed-width, 40px))"
                : `min(${panelLayout.panelWidth}px, calc(100% - var(--pane-floor)))`,
            } as CSSProperties
          }
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
              // Narrow screen: the rail closes on selection. Only a deliberate
              // open moves focus to the session title in the conversation header
              // (design §3.1); a pointer-driven peek must not pull focus.
              cancelPeekClose();
              setWorkspacePeek(false);
              setWorkspaceOpen(false);
              if (
                shouldFocusSessionHeadingAfterSelection({
                  deliberate: workspaceOpen,
                })
              ) {
                window.requestAnimationFrame(() =>
                  sessionTitleRef.current?.focus(),
                );
              }
            }}
            onNewSession={(workspaceId) => void handleNewSession(workspaceId)}
            onTogglePin={(workspaceId) =>
              void server.togglePin(workspaceId).catch(() => undefined)
            }
            onRemoveWorkspace={(workspaceId) =>
              void handleRemoveWorkspace(workspaceId)
            }
            mobileOpen={mobileLayout && workspaceOpen}
            peekOpen={mobileLayout && workspacePeek}
            onOverlayPointerEnter={mobileLayout ? cancelPeekClose : undefined}
            onOverlayPointerLeave={mobileLayout ? schedulePeekClose : undefined}
            onCloseMobile={closeWorkspaceDrawer}
            onOpenSettings={() => {
              setWorkspaceOpen(false);
              routing.openSettings("general");
            }}
            railCollapsed={navCollapsed}
            onToggleCollapsed={() =>
              navCollapsed ? expandRail() : setNavCollapsed(true)
            }
          />
          {/* Hidden on narrow layouts by CSS; the drawer has no width to drag. */}
          {!mobileLayout && !navCollapsed ? (
            <SidebarResizeHandle
              width={sidebar.width}
              previewWidth={sidebar.previewWidth}
              commitWidth={sidebar.commitWidth}
              cancelPreview={sidebar.cancelPreview}
              onHandleKeyDown={sidebar.onHandleKeyDown}
            />
          ) : null}
          {mobileLayout &&
          shouldShowSidebarHoverZone({
            narrow: mobileLayout,
            open: workspaceOpen || workspacePeek,
          }) ? (
            /* A fine-pointer affordance (see `sidebar-overlay`): decorative, so
               it stays out of the accessibility tree, and the header button
               remains the deliberate, keyboard-reachable way in. */
            <div
              className="sidebar-hover-zone"
              aria-hidden="true"
              data-testid="sidebar-hover-zone"
              onPointerEnter={openWorkspacePeek}
            />
          ) : null}

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
                    <h1 ref={sessionTitleRef} tabIndex={-1}>{activeSession.title}</h1>
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
                      ? t("inspector.title")
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
                    if (activeJobId && activeRunId) panel.open("pending", {
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
              panelLayout={panelLayout}
              uiVersion={uiVersion}
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
                // A finding opens its file, so the files tab takes the target
                // (PI-Desktop opens a file tab from a review finding).
                panel.open("files", { kind: "file", path, line });
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
      <CommandPalette
        open={commandOpen}
        entries={commandEntries}
        labels={commandLabels}
        onAction={handleCommandAction}
        onClose={() => setCommandOpen(false)}
      />
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
