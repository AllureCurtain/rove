"use client";

import {
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { flushSync } from "react-dom";
import {
  ChevronDownIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  Cross2Icon,
  DotsHorizontalIcon,
  DrawingPinFilledIcon,
  GearIcon,
  MagnifyingGlassIcon,
  MixerHorizontalIcon,
  PlusIcon,
} from "@radix-ui/react-icons";

import { useCopy } from "../copy/CopyProvider";
import { useArmedDelete } from "../shell/use-armed-delete";
import type { SessionRecord, WorkspaceKind, WorkspaceRecord } from "../state/product-types";
import { OpenWorkspaceDialog } from "./OpenWorkspaceDialog";
import {
  orderWorkspaces,
  readProjectOrder,
  reorderProjectOrder,
  writeProjectOrder,
} from "./rail-project-order";
import {
  EMPTY_SELECTION,
  applySelectionClick,
  isSelectionClick,
  pruneSelection,
  type SessionSelection,
} from "./rail-selection";
import {
  SESSION_TIME_GROUPS,
  groupSessionsByTime,
  type SessionSort,
  type SessionTimeGroup,
} from "./rail-time-groups";
import { SessionRow } from "./SessionRow";
import { orderSessions, type SessionPin } from "./session-order";
import { resolveSessionRows } from "./session-server-search";
import { useServerSessionSearch } from "./use-server-session-search";
import {
  formatDisplayPath,
} from "./session-labels";
import { useSessionPins } from "./use-session-pins";

/** Rows rendered per time group before a "load more" step (§7.1). */
const RAIL_GROUP_PAGE = 20;
const SESSION_SORT_KEY = "rove.ui-session-sort";

function readSessionSort(): SessionSort {
  if (typeof window === "undefined") {
    return "recent";
  }
  return window.localStorage.getItem(SESSION_SORT_KEY) === "title"
    ? "title"
    : "recent";
}

/** Sidebar order inside a project body: pins lead, then modification time. */
function applySessionOrder(
  sessions: SessionRecord[],
  pins: readonly SessionPin[],
  activeSessionId: string | null,
): SessionRecord[] {
  const dated = sessions.map((session) => ({
    ...session,
    modifiedAt: Number.isNaN(Date.parse(session.updatedAt)) ? 0 : Date.parse(session.updatedAt),
  }));
  return orderSessions(dated, pins, {
    collapsedLimit: 0,
    activeSessionId,
  }).ordered;
}

/** §7.2: hover affordances go quiet while the window itself is not focused. */
function useWindowBlur(): boolean {
  const [blurred, setBlurred] = useState(false);
  useEffect(() => {
    const onBlur = () => setBlurred(true);
    const onFocus = () => setBlurred(false);
    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", onFocus);
    };
  }, []);
  return blurred;
}

export function SessionRail({
  workspaces,
  sessions,
  activeWorkspaceId,
  activeSessionId,
  mutationBusy,
  onOpenWorkspace,
  onSelectWorkspace,
  onSelectSession,
  onNewSession,
  onTogglePin,
  onRenameWorkspace,
  onRemoveWorkspace,
  onRenameSession,
  onArchiveSession,
  onDeleteSession,
  onBranchSession,
  onExportSession,
  onRevealWorkspace,
  queueCounts,
  version,
  mobileOpen = false,
  peekOpen = false,
  onCloseMobile,
  onOverlayPointerEnter,
  onOverlayPointerLeave,
  onOpenSettings,
  footerActions,
  railCollapsed,
  onToggleCollapsed,
  searchSessions,
}: {
  workspaces: WorkspaceRecord[];
  /** Every catalog session, archived included (§7.1 archived group). */
  sessions: SessionRecord[];
  activeWorkspaceId: string | null;
  activeSessionId: string | null;
  mutationBusy: boolean;
  onOpenWorkspace: (path: string, kind: WorkspaceKind) => void;
  onSelectWorkspace: (workspaceId: string) => void;
  onSelectSession: (workspaceId: string, sessionId: string) => void;
  onNewSession: (workspaceId: string) => void;
  onTogglePin: (workspaceId: string) => void;
  /** Persists a project rename; resolves false when the server refused. */
  onRenameWorkspace: (workspaceId: string, displayName: string) => Promise<boolean>;
  /**
   * Removes a workspace and its sessions; a confirmation dialog in the rail
   * already lists the session count before this runs.
   */
  onRemoveWorkspace: (workspaceId: string) => void;
  /** Persist a renamed session title; rejects when the server refused. */
  onRenameSession?: (sessionId: string, title: string) => Promise<boolean>;
  onArchiveSession: (sessionId: string, archived: boolean) => void;
  /** Resolves true when the removed row was the active session. */
  onDeleteSession: (sessionId: string) => Promise<boolean>;
  onBranchSession: (sessionId: string) => void;
  /**
   * Evidence export is a row-menu action (design §8/A11): the host opens the
   * export dialog for this session.
   */
  onExportSession?: (session: SessionRecord) => void;
  /**
   * Desktop-only: reveal the project root in the OS file manager. Absent in
   * the browser build, where the menu hides the item instead of dead-ending.
   */
  onRevealWorkspace?: (workspaceId: string) => void;
  /** Queued-prompt depth per session (the active session's queue; §3.1 slot). */
  queueCounts?: ReadonlyMap<string, number>;
  /** API build version rendered in the footer. */
  version?: string | null;
  mobileOpen?: boolean;
  /**
   * Narrow screen: the rail is showing because a pointer summoned it, so it is
   * visually open but *not* a dialog — no modal semantics, no focus trap, and
   * the page behind it stays interactive.
   */
  peekOpen?: boolean;
  onCloseMobile?: () => void;
  onOverlayPointerEnter?: () => void;
  onOverlayPointerLeave?: () => void;
  onOpenSettings?: () => void;
  /**
   * Icon-row content for the footer (design §3.1): the shell passes the
   * notification inbox, the theme toggle and any other chrome that used to
   * live in the removed page-level top bar.
   */
  footerActions?: ReactNode;
  /** Collapsed rail: the subtree stays mounted but inert and aria-hidden. */
  railCollapsed?: boolean;
  onToggleCollapsed?: () => void;
  /**
   * F11: the server's own session search for the active query. Optional — a
   * caller that cannot reach the product API keeps the local substring filter.
   */
  searchSessions?: (workspaceId: string, query: string) => Promise<SessionRecord[]>;
}) {
  const { t } = useCopy();
  const [openDialog, setOpenDialog] = useState(false);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SessionSort>("recent");
  const [projectOrder, setProjectOrder] = useState<string[]>([]);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [groupPages, setGroupPages] = useState<Record<string, number>>({});
  const [selection, setSelection] = useState<SessionSelection>(EMPTY_SELECTION);
  const [sortMenuOpen, setSortMenuOpen] = useState(false);
  const [pendingDeleteWorkspace, setPendingDeleteWorkspace] = useState<string | null>(null);
  const [renamingWorkspace, setRenamingWorkspace] = useState<string | null>(null);
  const [draggingWorkspace, setDraggingWorkspace] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ id: string; after: boolean } | null>(null);
  const dialogTriggerRef = useRef<HTMLButtonElement | null>(null);
  const windowBlurred = useWindowBlur();

  useEffect(() => {
    setSort(readSessionSort());
    setProjectOrder(readProjectOrder());
  }, []);

  // Session pins and the paint-first selection live here so every zone shares
  // one ordering rule and one selection barrier.
  const liveSessionIds = useMemo(
    () => sessions.map((session) => session.id),
    [sessions],
  );
  const { pins, togglePin, isPinned } = useSessionPins(liveSessionIds);
  const [selectionPaint, setSelectionPaint] = useState<{
    workspaceId: string;
    sessionId: string;
  } | null>(null);
  const selectSequenceRef = useRef(0);
  useEffect(() => {
    // The real selection arrived: the optimistic highlight has served its purpose.
    if (selectionPaint?.sessionId === activeSessionId) {
      setSelectionPaint(null);
    }
  }, [activeSessionId, selectionPaint]);

  const sessionsByWorkspace = useMemo(() => {
    const map = new Map<string, SessionRecord[]>();
    for (const session of sessions) {
      const list = map.get(session.workspaceId) ?? [];
      list.push(session);
      map.set(session.workspaceId, list);
    }
    return map;
  }, [sessions]);
  const workspaceById = useMemo(
    () => new Map(workspaces.map((workspace) => [workspace.id, workspace])),
    [workspaces],
  );

  // ── Zones (§7.1) ──────────────────────────────────────────────────────────

  const pinnedSessions = useMemo(
    () => sessions.filter((session) => isPinned(session.id)),
    [sessions, isPinned],
  );

  const timeGroups = useMemo(
    () => groupSessionsByTime(sessions, new Date(), sort),
    [sessions, sort],
  );

  const orderedWorkspaces = useMemo(
    () => orderWorkspaces(workspaces, projectOrder),
    [workspaces, projectOrder],
  );

  // The flat row order the range selection resolves against: the Pinned strip
  // first, then the time groups, then project bodies — the rail's visual order
  // top to bottom, deduped (a pinned session renders twice).
  const railOrder = useMemo(() => {
    const seen = new Set<string>();
    const order: string[] = [];
    const push = (session: SessionRecord) => {
      if (!seen.has(session.id)) {
        seen.add(session.id);
        order.push(session.id);
      }
    };
    for (const session of pinnedSessions) {
      push(session);
    }
    for (const group of SESSION_TIME_GROUPS) {
      for (const session of timeGroups.get(group) ?? []) {
        push(session);
      }
    }
    for (const workspace of orderedWorkspaces) {
      for (const session of sessionsByWorkspace.get(workspace.id) ?? []) {
        push(session);
      }
    }
    return order;
  }, [pinnedSessions, timeGroups, orderedWorkspaces, sessionsByWorkspace]);

  // A deleted or archived-out session must not linger in the selection.
  useEffect(() => {
    setSelection((current) =>
      pruneSelection(current, new Set(liveSessionIds)),
    );
  }, [liveSessionIds]);

  const selectionAllArchived = useMemo(() => {
    if (selection.selected.size === 0) {
      return false;
    }
    return [...selection.selected].every(
      (id) =>
        sessions.find((session) => session.id === id)?.archived === true,
    );
  }, [selection.selected, sessions]);

  /**
   * Select-then-paint: commit the highlight synchronously with the click
   * (flushSync, so concurrent rendering cannot defer it), let one committed
   * frame pass, then start the session load. A newer click invalidates the
   * older request so rapid clicks never stack stale loads. Keyboard activation
   * skips the barrier — repeated Enter must not pile up delayed navigations.
   * Input typed inside the barrier belongs to the still-mounted session.
   */
  function handleSelectSession(workspaceId: string, sessionId: string, immediate: boolean) {
    const sequence = ++selectSequenceRef.current;
    // A discrete pointer action: commit the lightweight sidebar state before
    // scheduling the navigation.
    flushSync(() => {
      setSelectionPaint({ workspaceId, sessionId });
    });
    const navigate = () => {
      if (sequence !== selectSequenceRef.current) {
        return;
      }
      onSelectSession(workspaceId, sessionId);
    };
    if (immediate || typeof document === "undefined" || document.visibilityState === "hidden") {
      navigate();
      return;
    }
    // Wait for a committed paint, the web way: one rAF to commit, one to
    // present.
    window.requestAnimationFrame(() => window.requestAnimationFrame(navigate));
  }

  function handleRowSelect(
    session: SessionRecord,
    modifiers: { additive: boolean; range: boolean } | null,
    immediate: boolean,
  ) {
    if (modifiers !== null && isSelectionClick(modifiers)) {
      setSelection((current) =>
        applySelectionClick(current, railOrder, session.id, modifiers),
      );
      return;
    }
    // A plain click navigates and ends any multi-select.
    setSelection(EMPTY_SELECTION);
    handleSelectSession(session.workspaceId, session.id, immediate);
  }

  const handleBatchArchive = useCallback(
    (archived: boolean) => {
      const ids = [...selection.selected];
      setSelection(EMPTY_SELECTION);
      for (const id of ids) {
        onArchiveSession(id, archived);
      }
    },
    [selection.selected, onArchiveSession],
  );

  const handleBatchDelete = useCallback(async () => {
    const ids = [...selection.selected];
    setSelection(EMPTY_SELECTION);
    for (const id of ids) {
      try {
        await onDeleteSession(id);
      } catch {
        // The hook surfaces the failure; the loop continues through the rest.
      }
    }
  }, [selection.selected, onDeleteSession]);

  const normalizedQuery = query.trim().toLocaleLowerCase();
  const workspaceIds = useMemo(
    () => workspaces.map((workspace) => workspace.id),
    [workspaces],
  );
  const serverSearch = useServerSessionSearch({
    query,
    workspaceIds,
    search: searchSessions,
    enabled: Boolean(searchSessions),
  });
  const visibleWorkspaces = orderedWorkspaces.flatMap((workspace) => {
    const allSessions = applySessionOrder(
      sessionsByWorkspace.get(workspace.id) ?? [],
      pins,
      activeSessionId,
    );
    const workspaceMatches = normalizedQuery
      ? workspace.displayName.toLocaleLowerCase().includes(normalizedQuery)
      : false;
    // F11: the server's rows win while it answered; with no answer the tested
    // local projection decides (design §12's offline degradation).
    const serverMatches = serverSearch.matchesByWorkspace?.[workspace.id];
    const resolved = resolveSessionRows({
      localSessions: allSessions,
      serverMatches: serverMatches
        ? applySessionOrder(serverMatches, pins, activeSessionId)
        : null,
      query,
      workspaceMatches,
    });
    return resolved.visible
      ? [{ workspace, allSessions, sessions: resolved.sessions }]
      : [];
  });

  // Workspaces whose session list is currently mounted. A collapse keeps the
  // subtree mounted for the length of the animation so it can shrink instead
  // of vanishing, then unmounts (design §3.3: 200ms animation, 220ms delay).
  const [mountedGroups, setMountedGroups] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(
      workspaces
        .filter((workspace) => workspace.id === activeWorkspaceId)
        .map((workspace) => [workspace.id, true]),
    ),
  );

  // Reveal a newly activated project, but never undo a user's collapse on polling.
  useEffect(() => {
    if (activeWorkspaceId) {
      setCollapsed((current) => ({ ...current, [activeWorkspaceId]: false }));
    }
  }, [activeWorkspaceId]);

  // Delay the subtree unmount so the collapse can animate (design §3.3).
  useEffect(() => {
    const timers: number[] = [];
    for (const workspace of workspaces) {
      const expanded = Boolean(normalizedQuery) ||
        !(collapsed[workspace.id] ?? workspace.id !== activeWorkspaceId);
      const isMounted = mountedGroups[workspace.id] ?? false;
      if (expanded === isMounted) {
        continue;
      }
      if (expanded) {
        setMountedGroups((current) => ({ ...current, [workspace.id]: true }));
      } else {
        timers.push(
          window.setTimeout(() => {
            setMountedGroups((current) => ({ ...current, [workspace.id]: false }));
          }, 220),
        );
      }
    }
    return () => {
      for (const timer of timers) {
        window.clearTimeout(timer);
      }
    };
  }, [workspaces, collapsed, activeWorkspaceId, normalizedQuery, mountedGroups]);
  const addWorkspaceButtonRef = useRef<HTMLButtonElement>(null);
  const closeMobileButtonRef = useRef<HTMLButtonElement>(null);
  const sidebarRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (!mobileOpen) {
      return;
    }
    const sidebar = sidebarRef.current;
    const focusCloseButton = () => {
      if (sidebar && !sidebar.contains(document.activeElement)) {
        closeMobileButtonRef.current?.focus();
      }
    };
    const handleTransitionEnd = (event: TransitionEvent) => {
      if (event.target === sidebar && event.propertyName === "transform") {
        focusCloseButton();
      }
    };
    const frame = window.requestAnimationFrame(focusCloseButton);
    const fallback = window.setTimeout(focusCloseButton, 220);
    sidebar?.addEventListener("transitionend", handleTransitionEnd);
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(fallback);
      sidebar?.removeEventListener("transitionend", handleTransitionEnd);
    };
  }, [mobileOpen]);

  function closeDialog() {
    setOpenDialog(false);
    window.requestAnimationFrame(() => dialogTriggerRef.current?.focus());
  }

  function openWorkspaceDialog(trigger: HTMLButtonElement | null) {
    dialogTriggerRef.current = trigger;
    setOpenDialog(true);
  }

  function renderSessionRow(
    session: SessionRecord,
    options: { showProject: boolean; allSessions?: SessionRecord[] },
    children?: ReactNode,
  ) {
    const workspace = workspaceById.get(session.workspaceId);
    const allSessions = options.allSessions ?? sessions;
    const parentTitle = session.parentSessionId
      ? (allSessions.find((parent) => parent.id === session.parentSessionId)?.title ?? null)
      : null;
    const parentAvailable = session.parentSessionId
      ? allSessions.some((parent) => parent.id === session.parentSessionId)
      : true;
    return (
      <SessionRow
        key={session.id}
        session={session}
        workspace={workspace}
        showProject={options.showProject}
        parentAvailable={parentAvailable}
        parentTitle={parentTitle}
        active={session.id === activeSessionId}
        painted={selectionPaint?.sessionId === session.id}
        pinned={isPinned(session.id)}
        multiSelected={selection.selected.has(session.id)}
        selectedCount={selection.selected.size}
        selectionAllArchived={selectionAllArchived}
        queueCount={queueCounts?.get(session.id) ?? 0}
        disabled={mutationBusy}
        onSelect={(row, modifiers, immediate) =>
          handleRowSelect(row, modifiers, immediate)
        }
        onTogglePin={togglePin}
        onRename={onRenameSession}
        onArchive={(sessionId, archived) => onArchiveSession(sessionId, archived)}
        onDelete={(sessionId) => void onDeleteSession(sessionId)}
        onBranch={(sessionId) => onBranchSession(sessionId)}
        onExport={
          onExportSession ? () => onExportSession(session) : undefined
        }
        onBatchArchive={handleBatchArchive}
        onBatchDelete={() => void handleBatchDelete()}
        onClearSelection={() => setSelection(EMPTY_SELECTION)}
      >
        {children}
      </SessionRow>
    );
  }

  function renderProjectGroup(entry: {
    workspace: WorkspaceRecord;
    allSessions: SessionRecord[];
    sessions: SessionRecord[];
  }) {
    const { workspace, allSessions, sessions: groupSessions } = entry;
    const active = workspace.id === activeWorkspaceId;
    const expanded = normalizedQuery ? true : !(collapsed[workspace.id] ?? !active);
    const isGroupMounted = mountedGroups[workspace.id] ?? false;
    const runningCount = allSessions.filter((session) => session.status === "running").length;
    const attentionCount = allSessions.filter(
      (session) => session.status === "needs_attention",
    ).length;
    const errorCount = allSessions.filter((session) => session.status === "error").length;
    const workspaceTone =
      runningCount > 0
        ? "running"
        : attentionCount > 0
          ? "needs_attention"
          : errorCount > 0
            ? "error"
            : "idle";
    const dropPosition =
      dropTarget?.id === workspace.id
        ? dropTarget.after
          ? "after"
          : "before"
        : null;

    return (
      <div
        className="workspace-group"
        key={workspace.id}
        data-tone={workspaceTone}
        data-drop-target={dropPosition ?? undefined}
        data-dragging={draggingWorkspace === workspace.id || undefined}
      >
        <div
          className="workspace-group__row"
          draggable={!normalizedQuery && renamingWorkspace !== workspace.id}
          onDragStart={(event) => handleProjectDragStart(event, workspace)}
          onDragOver={(event) => handleProjectDragOver(event, workspace)}
          onDrop={(event) => handleProjectDrop(event, workspace)}
          onDragEnd={handleProjectDragEnd}
        >
          <button
            type="button"
            className="ghost icon-button workspace-group__disclosure"
            aria-label={t(expanded ? "workspace.collapseProject" : "workspace.expandProject", { name: workspace.displayName })}
            aria-expanded={expanded}
            aria-controls={`workspace-sessions-${workspace.id}`}
            disabled={mutationBusy || Boolean(normalizedQuery)}
            onClick={() => setCollapsed((current) => ({ ...current, [workspace.id]: expanded }))}
          >
            {expanded ? <ChevronDownIcon /> : <ChevronRightIcon />}
          </button>
          {renamingWorkspace === workspace.id ? (
            <ProjectRenameField
              workspace={workspace}
              onSubmit={async (name) => {
                const persisted = await onRenameWorkspace(workspace.id, name);
                if (persisted) {
                  setRenamingWorkspace(null);
                }
                return persisted;
              }}
              onCancel={() => setRenamingWorkspace(null)}
            />
          ) : (
            <button
              type="button"
              className="workspace-group__button"
              data-active={active}
              title={`${workspace.displayName}\n${formatDisplayPath(workspace.rootPath)}`}
              aria-label={`${workspace.displayName}, ${formatDisplayPath(workspace.rootPath)}`}
              aria-current={active ? "true" : undefined}
              onClick={() => {
                setCollapsed((current) => ({ ...current, [workspace.id]: false }));
                onSelectWorkspace(workspace.id);
              }}
              disabled={mutationBusy}
            >
              <span className="workspace-group__title">
                <span>{workspace.displayName}</span>
                {workspace.pinned ? <DrawingPinFilledIcon aria-label={t("workspace.pinned")} /> : null}
                <span className="workspace-group__count" aria-hidden="true">
                  {allSessions.length}
                </span>
                {runningCount > 0 ? (
                  <span
                    className="session-badge"
                    data-status="running"
                    title={t("workspace.runningCount", { count: runningCount })}
                  >
                    {runningCount === 1
                      ? t("workspace.running")
                      : t("workspace.runningCount", { count: runningCount })}
                  </span>
                ) : null}
                {runningCount === 0 && attentionCount > 0 ? (
                  <span className="session-badge" data-status="needs_attention">
                    {t("workspace.needsAttention")}
                  </span>
                ) : null}
                {runningCount === 0 && attentionCount === 0 && errorCount > 0 ? (
                  <span className="session-badge" data-status="error">
                    {t("inspector.statusFailed")}
                  </span>
                ) : null}
              </span>
            </button>
          )}
          <ProjectActions
            workspace={workspace}
            sessionCount={allSessions.length}
            disabled={mutationBusy}
            canReveal={onRevealWorkspace !== undefined}
            onReveal={onRevealWorkspace}
            onTogglePin={onTogglePin}
            onStartRename={() => setRenamingWorkspace(workspace.id)}
            onRequestDelete={() => setPendingDeleteWorkspace(workspace.id)}
          />
        </div>
        {/* Collapse animates 0fr -> 1fr and unmounts the subtree only
            after the animation, so a re-open never re-renders mid
            flight (design §3.3). */}
        <div
          id={`workspace-sessions-${workspace.id}`}
          className="workspace-sessions"
          data-expanded={expanded}
          aria-hidden={expanded ? undefined : true}
          hidden={!isGroupMounted}
        >
          {isGroupMounted ? (
            <>
              <SessionBranchList
                sessions={groupSessions}
                allSessions={allSessions}
                renderRow={(session, children) =>
                  renderSessionRow(
                    session,
                    { showProject: false, allSessions },
                    children,
                  )
                }
              />
              {groupSessions.length === 0 ? <p className="sidebar-empty">{t("workspace.noSessionsBody")}</p> : null}
            </>
          ) : null}
        </div>
      </div>
    );
  }

  // ── Project row drag reorder (§7.3) ───────────────────────────────────────
  //
  // The remembered order is a per-machine reading preference in localStorage —
  // the same rule session pins follow — because the product contract carries
  // no workspace ordering field. Drop positions stay inside the dragged row's
  // own pinned class: pinned projects always lead (orderWorkspaces), so a drop
  // that crossed the boundary would look like it did nothing.
  function handleProjectDragStart(event: DragEvent<HTMLDivElement>, workspace: WorkspaceRecord) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", workspace.id);
    setDraggingWorkspace(workspace.id);
  }

  function handleProjectDragOver(event: DragEvent<HTMLDivElement>, workspace: WorkspaceRecord) {
    if (!draggingWorkspace || draggingWorkspace === workspace.id) {
      return;
    }
    const dragged = workspaceById.get(draggingWorkspace);
    if (!dragged || dragged.pinned !== workspace.pinned) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    const rect = event.currentTarget.getBoundingClientRect();
    const after = event.clientY > rect.top + rect.height / 2;
    setDropTarget((current) =>
      current?.id === workspace.id && current.after === after
        ? current
        : { id: workspace.id, after },
    );
  }

  function handleProjectDrop(event: DragEvent<HTMLDivElement>, workspace: WorkspaceRecord) {
    event.preventDefault();
    if (!draggingWorkspace || draggingWorkspace === workspace.id) {
      return;
    }
    const dragged = workspaceById.get(draggingWorkspace);
    if (!dragged || dragged.pinned !== workspace.pinned) {
      setDropTarget(null);
      setDraggingWorkspace(null);
      return;
    }
    const after = dropTarget?.id === workspace.id ? dropTarget.after : false;
    const ids = orderedWorkspaces.map((item) => item.id);
    const next = reorderProjectOrder(ids, draggingWorkspace, workspace.id, after);
    setProjectOrder(next);
    writeProjectOrder(next);
    setDropTarget(null);
    setDraggingWorkspace(null);
  }

  function handleProjectDragEnd() {
    setDropTarget(null);
    setDraggingWorkspace(null);
  }

  const searching = Boolean(normalizedQuery);
  const sessionGroups = SESSION_TIME_GROUPS.map((group) => ({
    group,
    sessions: timeGroups.get(group) ?? [],
  })).filter((entry) => entry.sessions.length > 0);

  return (
    <aside
      ref={sidebarRef}
      className="product-sidebar"
      aria-label={t("workspace.label")}
      aria-busy={mutationBusy}
      data-open={mobileOpen || peekOpen}
      data-peek={peekOpen ? "true" : undefined}
      data-collapsed={railCollapsed}
      data-window-blur={windowBlurred || undefined}
      aria-hidden={railCollapsed ? true : undefined}
      inert={railCollapsed ? true : undefined}
      aria-modal={mobileOpen ? true : undefined}
      role={mobileOpen ? "dialog" : undefined}
      onPointerEnter={onOverlayPointerEnter}
      onPointerLeave={onOverlayPointerLeave}
      onKeyDown={(event) => {
        if (mobileOpen) {
          if (event.key === "Escape") {
            event.preventDefault();
            onCloseMobile?.();
            return;
          }
          trapFocus(event);
          return;
        }
        if (event.key === "Escape" && selection.selected.size > 0) {
          setSelection(EMPTY_SELECTION);
        }
      }}
    >
      <div className="product-sidebar__header">
        <span className="product-sidebar__brand">
          <span className="product-topbar__mark" aria-hidden="true">R</span>
          <strong>{t("workspace.brandName")}</strong>
        </span>
        {/* Collapse is a desktop-column affordance: inside the drawer or a
            peek the rail is already transient, so the control would only add
            a dead tab stop to the dialog's focus loop. */}
        {!mobileOpen && !peekOpen && onToggleCollapsed ? (
          <button
            type="button"
            className="ghost icon-button sidebar-collapse"
            onClick={onToggleCollapsed}
            aria-label={t("nav.collapseWorkspace")}
            title={t("nav.collapseWorkspace")}
          >
            <ChevronLeftIcon />
          </button>
        ) : null}
        {onCloseMobile && mobileOpen ? (
          <button
            ref={closeMobileButtonRef}
            type="button"
            className="ghost icon-button mobile-only"
            aria-label={t("workspace.close")}
            onClick={onCloseMobile}
          >
            <Cross2Icon />
          </button>
        ) : null}
      </div>
      <label className="workspace-search">
        <MagnifyingGlassIcon aria-hidden="true" />
        <input
          type="search"
          aria-label={t("workspace.search")}
          value={query}
          aria-describedby="workspace-search-scope"
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("workspace.searchPlaceholder")}
        />
      </label>
      <p id="workspace-search-scope" className="workspace-search-scope" data-tone={serverSearch.unavailable ? "warning" : undefined}>
        {serverSearch.unavailable
          ? t("workspace.searchUnavailable")
          : t("workspace.searchScope")}
      </p>
      <div className="product-sidebar__scroll">
        {searching ? (
          visibleWorkspaces.length === 0 ? (
            <p className="sidebar-empty" role="status">{t("workspace.noMatches")}</p>
          ) : (
            visibleWorkspaces.map(renderProjectGroup)
          )
        ) : (
          <>
            {pinnedSessions.length > 0 ? (
              <section className="rail-zone" aria-label={t("workspace.zonePinned")}>
                <h2 className="rail-zone__heading">{t("workspace.zonePinned")}</h2>
                {/* The pinned strip is a convenience shelf, not a second home:
                    its inner scroll caps near 30vh so the zones below stay on
                    screen (§7.1). */}
                <ul className="session-list session-list--flat rail-zone__pinned-list">
                  {pinnedSessions.map((session) =>
                    renderSessionRow(session, { showProject: true }),
                  )}
                </ul>
              </section>
            ) : null}
            <section className="rail-zone" aria-label={t("workspace.sessionsTitle")}>
              <div className="rail-zone__toolbar">
                <h2 className="rail-zone__heading">{t("workspace.sessionsTitle")}</h2>
                <SessionSortControl
                  sort={sort}
                  open={sortMenuOpen}
                  onToggle={() => setSortMenuOpen((value) => !value)}
                  onClose={() => setSortMenuOpen(false)}
                  onChange={(next) => {
                    setSort(next);
                    writeStoredSort(next);
                    setSortMenuOpen(false);
                  }}
                />
                <button
                  type="button"
                  className="ghost icon-button"
                  aria-label={t("nav.newSession")}
                  title={activeWorkspaceId
                    ? t("workspace.newSessionIn", { name: workspaceById.get(activeWorkspaceId)?.displayName ?? activeWorkspaceId })
                    : t("nav.newSession")}
                  disabled={mutationBusy}
                  onClick={(event) => {
                    if (activeWorkspaceId) {
                      onNewSession(activeWorkspaceId);
                    } else {
                      openWorkspaceDialog(event.currentTarget);
                    }
                  }}
                >
                  <PlusIcon />
                </button>
              </div>
              {sessions.length === 0 ? (
                <p className="sidebar-empty">{t("workspace.noSessions")}</p>
              ) : (
                sessionGroups.map(({ group, sessions: groupSessions }) => {
                  const page = groupPages[group] ?? 1;
                  const shown = groupSessions.slice(0, page * RAIL_GROUP_PAGE);
                  const remaining = groupSessions.length - shown.length;
                  return (
                    <div className="rail-time-group" key={group}>
                      <h3 className="rail-time-group__heading">
                        {t(`workspace.group.${group}`)}
                        <span className="rail-time-group__count" aria-hidden="true">
                          {groupSessions.length}
                        </span>
                      </h3>
                      <ul className="session-list session-list--flat rail-time-group__list">
                        {shown.map((session) =>
                          renderSessionRow(session, { showProject: true }),
                        )}
                      </ul>
                      {remaining > 0 ? (
                        <button
                          type="button"
                          className="ghost rail-load-more"
                          onClick={() =>
                            setGroupPages((current) => ({
                              ...current,
                              [group]: (current[group] ?? 1) + 1,
                            }))
                          }
                        >
                          {t("workspace.loadMoreSessions", { count: Math.min(remaining, RAIL_GROUP_PAGE) })}
                        </button>
                      ) : null}
                    </div>
                  );
                })
              )}
            </section>
            <section className="rail-zone" aria-label={t("workspace.zoneProjects")}>
              <div className="rail-zone__toolbar">
                <h2 className="rail-zone__heading">{t("workspace.zoneProjects")}</h2>
                <button
                  ref={addWorkspaceButtonRef}
                  type="button"
                  className="ghost icon-button"
                  onClick={(event) => openWorkspaceDialog(event.currentTarget)}
                  aria-label={t("workspace.add")}
                  title={t("workspace.add")}
                  disabled={mutationBusy}
                >
                  <PlusIcon />
                </button>
              </div>
              {workspaces.length === 0 ? (
                <p className="sidebar-empty">{t("workspace.none")}</p>
              ) : (
                orderedWorkspaces
                  .flatMap((workspace) => {
                    const allSessions = applySessionOrder(
                      (sessionsByWorkspace.get(workspace.id) ?? []).filter(
                        (session) => !session.archived,
                      ),
                      pins,
                      activeSessionId,
                    );
                    return [
                      {
                        workspace,
                        allSessions,
                        sessions: allSessions,
                      },
                    ];
                  })
                  .map(renderProjectGroup)
              )}
            </section>
          </>
        )}
      </div>
      <footer className="product-sidebar__footer">
        {/* Icon row (design §3.1/§7.1): the chrome the shell hands down —
            inbox and theme — beside the rail's own settings entry, with the
            build version pinned to the trailing edge. */}
        {footerActions}
        {onOpenSettings ? (
          <button
            type="button"
            className="ghost icon-button"
            onClick={onOpenSettings}
            aria-label={t("nav.settings")}
            title={t("nav.settings")}
          >
            <GearIcon />
          </button>
        ) : null}
        {version ? (
          <span className="product-sidebar__version" title={t("workspace.versionTitle", { version })}>
            {t("workspace.version", { version })}
          </span>
        ) : null}
      </footer>
      {openDialog ? (
        <OpenWorkspaceDialog
          onCancel={closeDialog}
          onOpen={(path, kind) => {
            onOpenWorkspace(path, kind);
            setOpenDialog(false);
          }}
        />
      ) : null}
      {pendingDeleteWorkspace ? (
        <ProjectDeleteDialog
          workspace={workspaceById.get(pendingDeleteWorkspace)}
          sessionCount={(sessionsByWorkspace.get(pendingDeleteWorkspace) ?? []).length}
          busy={mutationBusy}
          onCancel={() => setPendingDeleteWorkspace(null)}
          onConfirm={() => {
            const id = pendingDeleteWorkspace;
            setPendingDeleteWorkspace(null);
            onRemoveWorkspace(id);
          }}
        />
      ) : null}
    </aside>
  );
}

function writeStoredSort(sort: SessionSort) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(SESSION_SORT_KEY, sort);
  } catch {
    // A blocked or full storage quota must not break sorting.
  }
}

/** The Sessions toolbar's sort control: a small two-option menu (§7.1). */
function SessionSortControl({
  sort,
  open,
  onToggle,
  onClose,
  onChange,
}: {
  sort: SessionSort;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onChange: (sort: SessionSort) => void;
}) {
  const { t } = useCopy();
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) {
      return;
    }
    function outside(event: PointerEvent) {
      if (event.target instanceof Node && !root.current?.contains(event.target)) {
        onClose();
      }
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open, onClose]);
  return (
    <div className="workspace-actions" ref={root}>
      <button
        type="button"
        className="ghost icon-button"
        aria-label={t("workspace.sortSessions")}
        title={t("workspace.sortSessions")}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={onToggle}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            if (!open) {
              onToggle();
            }
          } else if (event.key === "Escape" && open) {
            event.preventDefault();
            onClose();
          }
        }}
      >
        <MixerHorizontalIcon />
      </button>
      {open ? (
        <div
          className="workspace-actions__menu"
          role="menu"
          aria-label={t("workspace.sortSessions")}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              onClose();
            }
          }}
        >
          {(["recent", "title"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              role="menuitemradio"
              aria-checked={sort === mode}
              className="ghost"
              onClick={() => onChange(mode)}
            >
              {t(mode === "recent" ? "workspace.sortRecent" : "workspace.sortTitle")}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** §7.3 project header menu: open folder / rename / pin / armed delete. */
function ProjectActions({
  workspace,
  sessionCount,
  disabled,
  canReveal,
  onReveal,
  onTogglePin,
  onStartRename,
  onRequestDelete,
}: {
  workspace: WorkspaceRecord;
  sessionCount: number;
  disabled: boolean;
  canReveal: boolean;
  onReveal?: (workspaceId: string) => void;
  onTogglePin: (workspaceId: string) => void;
  onStartRename: () => void;
  onRequestDelete: () => void;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(false);
  // A workspace removal drops its sessions too, so the menu item arms first
  // and the second click escalates to the count-listing dialog (§7.3).
  const { armed, setArmed } = useArmedDelete();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    menu.current?.scrollIntoView({ block: "nearest" });
    function outside(event: PointerEvent) {
      if (event.target instanceof Node && !root.current?.contains(event.target)) setOpen(false);
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  function close() {
    setOpen(false);
    // A dismissed menu never reopens armed.
    setArmed(null);
    trigger.current?.focus();
  }
  return (
    <div className="workspace-actions" ref={root}
      onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) { setOpen(false); setArmed(null); } }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          close();
        }
      }}
    >
      <button ref={trigger} type="button" className="ghost icon-button"
        aria-label={t("workspace.projectActions", { name: workspace.displayName })}
        aria-haspopup="menu" aria-expanded={open} aria-controls={`workspace-menu-${workspace.id}`}
        disabled={disabled} onClick={() => { setArmed(null); setOpen((value) => !value); }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") { event.preventDefault(); setOpen(true); }
        }}
      ><DotsHorizontalIcon /></button>
      {open ? (
        <div ref={menu} className="workspace-actions__menu" role="menu"
          id={`workspace-menu-${workspace.id}`}
          aria-label={t("workspace.projectActions", { name: workspace.displayName })}
          onKeyDown={(event) => {
            const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
            const index = items.indexOf(document.activeElement as HTMLButtonElement);
            const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
              : event.key === "ArrowDown" ? (index + 1) % items.length
              : event.key === "ArrowUp" ? (index - 1 + items.length) % items.length : null;
            if (next !== null) { event.preventDefault(); items[next]?.focus(); }
          }}
        >
          {canReveal && onReveal ? (
            <button type="button" role="menuitem" className="ghost" disabled={disabled}
              onClick={() => { close(); onReveal(workspace.id); }}>
              {t("workspace.openInFileManager")}
            </button>
          ) : null}
          <button type="button" role="menuitem" className="ghost" disabled={disabled}
            onClick={() => { close(); onStartRename(); }}>
            {t("workspace.renameWorkspace")}
          </button>
          <button type="button" role="menuitem" className="ghost" disabled={disabled}
            onClick={() => { close(); onTogglePin(workspace.id); }}>
            {t(workspace.pinned ? "workspace.unpinWorkspace" : "workspace.pinWorkspace")}
          </button>
          <button type="button" role="menuitem" className="ghost" disabled={disabled}
            data-armed={armed === workspace.id ? "true" : undefined}
            data-destructive="true"
            onClick={() => {
              // First click arms and relabels; the second opens the dialog
              // that lists how many sessions the removal drops (§7.3).
              if (armed !== workspace.id) {
                setArmed(workspace.id);
                return;
              }
              close();
              onRequestDelete();
            }}
            title={t("workspace.removeWorkspaceDialogHint", { count: sessionCount })}>
            {t(
              armed === workspace.id
                ? "workspace.removeWorkspaceArmed"
                : "workspace.removeWorkspace",
            )}
          </button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * §7.3: project removal escalates from the armed menu item to this dialog,
 * which names the workspace and counts the sessions the removal drops.
 */
function ProjectDeleteDialog({
  workspace,
  sessionCount,
  busy,
  onCancel,
  onConfirm,
}: {
  workspace: WorkspaceRecord | undefined;
  sessionCount: number;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useCopy();
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancelRef.current?.focus();
  }, []);
  if (!workspace) {
    return null;
  }
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onCancel();
        }
      }}
    >
      <div
        className="modal-card"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="project-delete-title"
        aria-describedby="project-delete-body"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
      >
        <h2 id="project-delete-title">{t("workspace.removeWorkspaceTitle")}</h2>
        <p id="project-delete-body" className="modal-card__lede">
          {t("workspace.removeWorkspaceBody", {
            name: workspace.displayName,
            count: sessionCount,
          })}
        </p>
        <div className="modal-actions">
          <button
            type="button"
            className="secondary"
            onClick={onCancel}
            ref={cancelRef}
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="secondary"
            data-destructive="true"
            disabled={busy}
            onClick={onConfirm}
          >
            {t("workspace.removeWorkspaceConfirm", { count: sessionCount })}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Inline rename inside the project row: Enter commits, Escape cancels, blur commits. */
function ProjectRenameField({
  workspace,
  onSubmit,
  onCancel,
}: {
  workspace: WorkspaceRecord;
  onSubmit: (name: string) => Promise<boolean>;
  onCancel: () => void;
}) {
  const { t } = useCopy();
  const [draft, setDraft] = useState(workspace.displayName);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);
  async function commit() {
    const name = draft.trim();
    if (!name) {
      setError(t("workspace.sessionRenameEmpty"));
      return;
    }
    if (name === workspace.displayName) {
      onCancel();
      return;
    }
    setSaving(true);
    try {
      const persisted = await onSubmit(name);
      if (!persisted) {
        setError(t("workspace.sessionRenameFailed"));
      }
    } catch {
      setError(t("workspace.sessionRenameFailed"));
    } finally {
      setSaving(false);
    }
  }
  return (
    <div className="workspace-group__rename">
      <input
        ref={inputRef}
        type="text"
        value={draft}
        disabled={saving}
        aria-label={t("workspace.renameWorkspace")}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void commit();
          } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => {
          if (!saving) void commit();
        }}
      />
      {error ? <p className="session-item__error" role="alert">{error}</p> : null}
    </div>
  );
}

/**
 * Project-body session list: keeps the parent/branch nesting so a forked
 * session renders indented under the session it came from.
 */
function SessionBranchList({
  sessions,
  allSessions,
  renderRow,
}: {
  sessions: SessionRecord[];
  allSessions: SessionRecord[];
  renderRow: (session: SessionRecord, children?: ReactNode) => ReactNode;
}) {
  const { t } = useCopy();
  const visibleSessionIds = new Set(sessions.map((session) => session.id));
  const childrenByParent = new Map<string, SessionRecord[]>();
  for (const session of sessions) {
    if (!session.parentSessionId || !visibleSessionIds.has(session.parentSessionId)) {
      continue;
    }
    const children = childrenByParent.get(session.parentSessionId) ?? [];
    children.push(session);
    childrenByParent.set(session.parentSessionId, children);
  }
  const roots = sessions.filter(
    (session) =>
      !session.parentSessionId || !visibleSessionIds.has(session.parentSessionId),
  );

  function renderBranch(session: SessionRecord): ReactNode {
    const children = childrenByParent.get(session.id) ?? [];
    // renderRow returns the keyed <SessionRow> li; a fork's children nest
    // inside it as a sub-list.
    return renderRow(
      session,
      children.length > 0 ? (
        <ul className="session-list session-list--branch">
          {children.map((child) => renderBranch(child))}
        </ul>
      ) : undefined,
    );
  }

  return (
    <ul className="session-list" aria-label={t("workspace.sessionsAndBranches")}>
      {roots.map((session) => renderBranch(session))}
    </ul>
  );
}

function trapFocus(event: KeyboardEvent<HTMLElement>) {
  if (event.key !== "Tab") {
    return;
  }
  const focusable = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), select:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
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


