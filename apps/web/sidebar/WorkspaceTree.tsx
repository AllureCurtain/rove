"use client";

import {
  FormEvent,
  type KeyboardEvent,
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
  DotsHorizontalIcon,
  Cross2Icon,
  DrawingPinFilledIcon,
  FileIcon,
  GearIcon,
  LockClosedIcon,
  MagnifyingGlassIcon,
  Pencil1Icon,
  PlusIcon,
} from "@radix-ui/react-icons";

import {
  desktopWorkspacePickerAvailable,
  selectDesktopWorkspace,
} from "../platform/desktop-commands";
import { useCopy } from "../copy/CopyProvider";
import { useArmedDelete } from "../shell/use-armed-delete";
import type { SessionRecord, WorkspaceKind, WorkspaceRecord } from "../state/product-types";
import { SessionHoverCard } from "./SessionHoverCard";
import { orderSessions, type SessionPin } from "./session-order";
import { resolveSessionRows } from "./session-server-search";
import { useServerSessionSearch } from "./use-server-session-search";
import {
  formatDisplayPath,
  sessionResultMark,
  sessionStatusLabel,
  shortId,
} from "./session-labels";
import { useSessionHoverCard } from "./use-session-hover-card";
import { useSessionPins } from "./use-session-pins";

/** Sidebar order: pins lead, then modification time (newest first). */
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

export function WorkspaceTree({
  workspaces,
  sessionsByWorkspace,
  activeWorkspaceId,
  activeSessionId,
  mutationBusy,
  onOpenWorkspace,
  onSelectWorkspace,
  onSelectSession,
  onNewSession,
  onTogglePin,
  onRemoveWorkspace,
  onRenameSession,
  mobileOpen = false,
  peekOpen = false,
  onCloseMobile,
  onOverlayPointerEnter,
  onOverlayPointerLeave,
  onOpenSettings,
  railCollapsed,
  onToggleCollapsed,
  searchSessions,
}: {
  workspaces: WorkspaceRecord[];
  sessionsByWorkspace: Record<string, SessionRecord[]>;
  activeWorkspaceId: string | null;
  activeSessionId: string | null;
  mutationBusy: boolean;
  onOpenWorkspace: (path: string, kind: WorkspaceKind) => void;
  onSelectWorkspace: (workspaceId: string) => void;
  onSelectSession: (workspaceId: string, sessionId: string) => void;
  onNewSession: (workspaceId: string) => void;
  onTogglePin: (workspaceId: string) => void;
  onRemoveWorkspace: (workspaceId: string) => void;
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
  /** Persist a renamed session title; rejects when the server refused. */
  onRenameSession?: (sessionId: string, title: string) => Promise<boolean>;
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
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const newSessionButtonRef = useRef<HTMLButtonElement>(null);
  const dialogTriggerRef = useRef<HTMLButtonElement | null>(null);
  // Session pins and the paint-first selection live here so every workspace's
  // list shares one ordering rule and one selection barrier.
  const liveSessionIds = useMemo(
    () =>
      Object.values(sessionsByWorkspace)
        .flat()
        .map((session) => session.id),
    [sessionsByWorkspace],
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

  /**
   * Select-then-paint (open-vetta's `selectAfterPaint`): commit the highlight
   * synchronously with the click (flushSync, so concurrent rendering cannot
   * defer it), let one committed frame pass, then start the session load. A
   * newer click invalidates the older request so rapid clicks never stack
   * stale loads. Keyboard activation skips the barrier — repeated Enter must
   * not pile up delayed navigations. Input typed inside the barrier belongs
   * to the still-mounted session, exactly like the reference.
   */
  function handleSelectSession(workspaceId: string, sessionId: string, immediate: boolean) {
    const sequence = ++selectSequenceRef.current;
    // A discrete pointer action: commit the lightweight sidebar state before
    // scheduling the navigation (open-vetta's flushSync contract).
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
    // waitForCommittedPaint's web equivalent (open-vetta
    // `committed-paint.ts`): one rAF to commit, one to present.
    window.requestAnimationFrame(() => window.requestAnimationFrame(navigate));
  }
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
  const visibleWorkspaces = workspaces.flatMap((workspace) => {
    const allSessions = applySessionOrder(
      sessionsByWorkspace[workspace.id] ?? [],
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

  return (
    <aside
      ref={sidebarRef}
      className="product-sidebar"
      aria-label={t("workspace.label")}
      aria-busy={mutationBusy}
      data-open={mobileOpen || peekOpen}
      data-peek={peekOpen ? "true" : undefined}
      data-collapsed={railCollapsed}
      aria-hidden={railCollapsed ? true : undefined}
      inert={railCollapsed ? true : undefined}
      aria-modal={mobileOpen ? true : undefined}
      role={mobileOpen ? "dialog" : undefined}
      onPointerEnter={onOverlayPointerEnter}
      onPointerLeave={onOverlayPointerLeave}
      onKeyDown={
        mobileOpen
          ? (event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                onCloseMobile?.();
                return;
              }
              trapFocus(event);
            }
          : undefined
      }
    >
      <div className="product-sidebar__header">
        <button
          ref={newSessionButtonRef}
          type="button"
          className="workspace-new-session"
          disabled={mutationBusy}
          title={activeWorkspaceId
            ? t("workspace.newSessionIn", { name: workspaces.find((workspace) => workspace.id === activeWorkspaceId)?.displayName ?? activeWorkspaceId })
            : t("empty.open")}
          onClick={() => {
            if (activeWorkspaceId) onNewSession(activeWorkspaceId);
            else {
              dialogTriggerRef.current = newSessionButtonRef.current;
              setOpenDialog(true);
            }
          }}
        >
          <PlusIcon aria-hidden="true" /> {t("nav.newSession")}
        </button>
        <button
          ref={addWorkspaceButtonRef}
          type="button"
          className="secondary icon-button"
          onClick={() => {
            dialogTriggerRef.current = addWorkspaceButtonRef.current;
            setOpenDialog(true);
          }}
          aria-label={t("workspace.add")}
          disabled={mutationBusy}
        >
          <PlusIcon />
        </button>
        {onToggleCollapsed ? (
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
        {workspaces.length === 0 ? (
          <p className="sidebar-empty">{t("workspace.none")}</p>
        ) : visibleWorkspaces.length === 0 ? (
          <p className="sidebar-empty" role="status">{t("workspace.noMatches")}</p>
        ) : (
          visibleWorkspaces.map(({ workspace, allSessions, sessions }) => {
            const active = workspace.id === activeWorkspaceId;
            const expanded = normalizedQuery ? true : !(collapsed[workspace.id] ?? !active);
            // The group subtree stays mounted through the collapse animation.
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

            return (
              <div className="workspace-group" key={workspace.id} data-tone={workspaceTone}>
                <div className="workspace-group__row">
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
                    <span className="workspace-group__path">{formatDisplayPath(workspace.rootPath)}</span>
                  </button>
                  <WorkspaceActions
                    workspace={workspace}
                    disabled={mutationBusy}
                    onTogglePin={onTogglePin}
                    onRemoveWorkspace={onRemoveWorkspace}
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
                        sessions={sessions}
                        allSessions={allSessions}
                        workspaceId={workspace.id}
                        workspaceRootPath={workspace.rootPath}
                        activeSessionId={activeSessionId}
                        paintedSessionId={selectionPaint?.sessionId ?? activeSessionId}
                        mutationBusy={mutationBusy}
                        onSelectSession={handleSelectSession}
                        onToggleSessionPin={togglePin}
                        isSessionPinned={isPinned}
                        onRenameSession={onRenameSession}
                      />
                      {sessions.length === 0 ? <p className="sidebar-empty">{t("workspace.noSessionsBody")}</p> : null}
                    </>
                  ) : null}
                </div>
              </div>
            );
          })
        )}
      </div>
      <footer className="product-sidebar__footer">
        <div className="workspace-boundary">
          <LockClosedIcon />
          <span><strong>{t("workspace.label")}</strong><small>{t("workspace.known")}</small></span>
        </div>
        {onOpenSettings ? (
          <button type="button" className="ghost" onClick={onOpenSettings}>
            <GearIcon /> {t("nav.settings")}
          </button>
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
    </aside>
  );
}

function WorkspaceActions({ workspace, disabled, onTogglePin, onRemoveWorkspace }: {
  workspace: WorkspaceRecord;
  disabled: boolean;
  onTogglePin: (workspaceId: string) => void;
  onRemoveWorkspace: (workspaceId: string) => void;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(false);
  // A workspace removal also drops its sessions from the catalog, so it needs a
  // second click (ported from PI-Desktop's `use-armed-delete`).
  const { armed, setArmed } = useArmedDelete();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLButtonElement>("button")?.focus();
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
          <button type="button" role="menuitem" className="ghost" disabled={disabled}
            onClick={() => { close(); onTogglePin(workspace.id); }}>
            {t(workspace.pinned ? "workspace.unpinWorkspace" : "workspace.pinWorkspace")}
          </button>
          <button type="button" role="menuitem" className="ghost" disabled={disabled}
            data-armed={armed === workspace.id ? "true" : undefined}
            onClick={() => {
              // First click arms and relabels; the menu stays open so the second
              // click lands on the same control. The arm expires by itself.
              if (armed !== workspace.id) {
                setArmed(workspace.id);
                return;
              }
              close();
              onRemoveWorkspace(workspace.id);
            }}>
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

function SessionBranchList({
  sessions,
  allSessions,
  workspaceId,
  workspaceRootPath,
  activeSessionId,
  paintedSessionId,
  mutationBusy,
  onSelectSession,
  onToggleSessionPin,
  isSessionPinned,
  onRenameSession,
}: {
  sessions: SessionRecord[];
  allSessions: SessionRecord[];
  workspaceId: string;
  workspaceRootPath: string;
  activeSessionId: string | null;
  /** The optimistically highlighted session while a selection paints/loads. */
  paintedSessionId: string | null;
  mutationBusy: boolean;
  onSelectSession: (workspaceId: string, sessionId: string, immediate: boolean) => void;
  onToggleSessionPin: (sessionId: string) => void;
  isSessionPinned: (sessionId: string) => boolean;
  onRenameSession?: (sessionId: string, title: string) => Promise<boolean>;
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

  return (
    <ul className="session-list" aria-label={t("workspace.sessionsAndBranches")}>
      {roots.map((session) => (
        <SessionBranch
          key={session.id}
          session={session}
          childrenByParent={childrenByParent}
          parentAvailable={
            !session.parentSessionId || allSessions.some((parent) => parent.id === session.parentSessionId)
          }
          parentTitle={
            allSessions.find((parent) => parent.id === session.parentSessionId)?.title ?? null
          }
          workspaceId={workspaceId}
          workspaceRootPath={workspaceRootPath}
          activeSessionId={activeSessionId}
          paintedSessionId={paintedSessionId}
          mutationBusy={mutationBusy}
          pinned={isSessionPinned(session.id)}
          onToggleSessionPin={onToggleSessionPin}
          isSessionPinned={isSessionPinned}
          onSelectSession={onSelectSession}
          onRenameSession={onRenameSession}
        />
      ))}
    </ul>
  );
}

function SessionBranch({
  session,
  childrenByParent,
  parentAvailable,
  parentTitle,
  workspaceId,
  workspaceRootPath,
  activeSessionId,
  paintedSessionId,
  mutationBusy,
  pinned,
  onToggleSessionPin,
  isSessionPinned,
  onSelectSession,
  onRenameSession,
}: {
  session: SessionRecord;
  childrenByParent: Map<string, SessionRecord[]>;
  parentAvailable: boolean;
  parentTitle?: string | null;
  workspaceId: string;
  workspaceRootPath: string;
  activeSessionId: string | null;
  paintedSessionId: string | null;
  mutationBusy: boolean;
  pinned: boolean;
  onToggleSessionPin: (sessionId: string) => void;
  isSessionPinned: (sessionId: string) => boolean;
  onSelectSession: (workspaceId: string, sessionId: string, immediate: boolean) => void;
  onRenameSession?: (sessionId: string, title: string) => Promise<boolean>;
}) {
  const { t } = useCopy();
  const hoverCard = useSessionHoverCard(session.id);
  const children = childrenByParent.get(session.id) ?? [];
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const selected = session.id === paintedSessionId;
  const active = session.id === activeSessionId;
  // W4.4 + R3: the dot reports the last finished turn's outcome and clears once
  // the session is opened. Before R3 the status field was the only source, so a
  // successful turn and a never-run session both drew nothing.
  const resultMark = sessionResultMark(session, active, t);

  useEffect(() => {
    if (renaming) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renaming]);

  function startRename() {
    if (!onRenameSession) {
      return;
    }
    setRenameDraft(session.title);
    setRenameError(null);
    setRenaming(true);
  }

  function cancelRename() {
    setRenaming(false);
    setRenameError(null);
  }

  async function commitRename() {
    if (!onRenameSession) {
      return;
    }
    const title = renameDraft.trim();
    if (!title) {
      // An empty title keeps the editor open instead of silently clearing.
      setRenameError(t("workspace.sessionRenameEmpty"));
      return;
    }
    if (title === session.title) {
      cancelRename();
      return;
    }
    setRenameSaving(true);
    try {
      const persisted = await onRenameSession(session.id, title);
      if (persisted) {
        setRenaming(false);
        setRenameError(null);
      } else {
        setRenameError(t("workspace.sessionRenameFailed"));
      }
    } catch {
      setRenameError(t("workspace.sessionRenameFailed"));
    } finally {
      setRenameSaving(false);
    }
  }

  return (
    <li
      className="session-branch"
      data-forked={session.parentSessionId ? "true" : undefined}
      data-orphaned={session.parentSessionId && !parentAvailable ? "true" : undefined}
    >
      {renaming ? (
        <div className="session-item session-item--editing">
          <input
            ref={renameInputRef}
            type="text"
            value={renameDraft}
            onChange={(event) => setRenameDraft(event.target.value)}
            disabled={renameSaving}
            aria-label={t("workspace.sessionRename")}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void commitRename();
              } else if (event.key === "Escape") {
                event.preventDefault();
                cancelRename();
              }
            }}
            onBlur={() => {
              // Blur commits, matching the plan's contract; Escape still cancels
              // because the keydown clears the editor before the blur lands.
              if (!renameSaving) void commitRename();
            }}
          />
          {renameError ? <p className="session-item__error" role="alert">{renameError}</p> : null}
        </div>
      ) : (
        <button
          type="button"
          className="session-item"
          data-active={selected}
          aria-current={active ? "page" : undefined}
          title={sessionAriaLabel(session, parentAvailable, t)}
          data-status={session.status}
          ref={hoverCard.anchorRef}
          aria-describedby={hoverCard.open ? hoverCard.cardId : undefined}
          onMouseEnter={hoverCard.rowProps.onMouseEnter}
          onMouseLeave={hoverCard.rowProps.onMouseLeave}
          onClick={(event) =>
            onSelectSession(workspaceId, session.id, event.detail === 0)
          }
          onDoubleClick={startRename}
          aria-label={sessionAriaLabel(session, parentAvailable, t)}
          disabled={mutationBusy}
        >
          {pinned ? (
            <DrawingPinFilledIcon className="session-item__pinmark" aria-hidden="true" />
          ) : null}
          <span className="session-item__title">
            <span>{session.title}</span>
            {session.parentSessionId ? (
              <small className="session-item__lineage">
                {forkPointLabel(session, parentAvailable, t)}
              </small>
            ) : null}
          </span>
          {session.status !== "idle" ? (
            <span className="session-badge" data-status={session.status}>
              {sessionStatusLabel(session.status, t)}
            </span>
          ) : null}
          {/* Leading status icon (design §3.1/§3.3): running spins, awaiting
              approval warns, a failed background run dots, otherwise nothing.
              The badge above stays as the accessible text. */}
          <span
            className="session-item__status"
            data-status={session.status}
            aria-hidden="true"
          >
            {session.status === "running" ? (
              <span className="session-item__spinner" />
            ) : session.status === "needs_attention" ? (
              <span className="session-item__warning" />
            ) : null}
          </span>
          {resultMark ? (
            <span
              className="session-item__dot"
              data-tone={resultMark.tone}
              role="status"
              title={resultMark.label}
            />
          ) : null}
        </button>
      )}
      <span className="session-item__actions">
        <button
          type="button"
          className="icon-button"
          aria-label={pinned ? t("workspace.sessionUnpin") : t("workspace.sessionPin")}
          title={pinned ? t("workspace.sessionUnpin") : t("workspace.sessionPin")}
          onClick={() => onToggleSessionPin(session.id)}
        >
          <DrawingPinFilledIcon data-pinned={pinned || undefined} />
        </button>
        {onRenameSession ? (
          <button
            type="button"
            className="icon-button"
            aria-label={t("workspace.sessionRename")}
            title={t("workspace.sessionRename")}
            onClick={startRename}
          >
            <Pencil1Icon />
          </button>
        ) : null}
      </span>
      {children.length > 0 ? (
        <ul className="session-list session-list--branch">
          {children.map((child) => (
            <SessionBranch
              key={child.id}
              session={child}
              childrenByParent={childrenByParent}
              parentAvailable
              parentTitle={session.title}
              workspaceId={workspaceId}
              workspaceRootPath={workspaceRootPath}
              activeSessionId={activeSessionId}
              paintedSessionId={paintedSessionId}
              mutationBusy={mutationBusy}
              pinned={isSessionPinned(child.id)}
              onToggleSessionPin={onToggleSessionPin}
              isSessionPinned={isSessionPinned}
              onSelectSession={onSelectSession}
              onRenameSession={onRenameSession}
            />
          ))}
        </ul>
      ) : null}
      {hoverCard.open ? (
        <SessionHoverCard
          id={hoverCard.cardId}
          session={session}
          workspaceRootPath={workspaceRootPath}
          parentTitle={parentTitle}
          anchor={hoverCard.anchor}
          host={hoverCard.portalHost}
        />
      ) : null}
    </li>
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

function sessionAriaLabel(
  session: SessionRecord,
  parentAvailable: boolean,
  t: (path: string) => string,
): string {
  const lineage = session.parentSessionId
    ? parentAvailable
      ? `${t("workspace.forked")}, `
      : `${t("workspace.forkedRemovedParent")}, `
    : "";
  if (session.status === "idle") {
    return `${lineage}${session.title}`;
  }
  return `${lineage}${session.title}, ${sessionStatusLabel(session.status, t)}`;
}

function forkPointLabel(
  session: SessionRecord,
  parentAvailable: boolean,
  t: (path: string) => string,
): string {
  const source = session.forkPointRunId
    ? shortId(session.forkPointRunId)
    : t("inspector.notAvailable");
  const sequence = session.forkPointSeq
    ? `event ${session.forkPointSeq}`
    : t("inspector.notAvailable");
  return `${parentAvailable ? t("workspace.forkPrefix") : t("workspace.parentRemoved")} · ${source} · ${sequence}`;
}

function OpenWorkspaceDialog({
  onOpen,
  onCancel,
}: {
  onOpen: (path: string, kind: WorkspaceKind) => void;
  onCancel: () => void;
}) {
  const { t } = useCopy();
  const [path, setPath] = useState("");
  const [kind, setKind] = useState<WorkspaceKind>("folder");
  const [error, setError] = useState<string | null>(null);
  const [nativePickerAvailable, setNativePickerAvailable] = useState(false);
  const [pickerBusy, setPickerBusy] = useState(false);

  useEffect(() => {
    setNativePickerAvailable(desktopWorkspacePickerAvailable());
  }, []);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = path.trim();
    if (!trimmed) {
      setError(t("empty.pathRequired"));
      return;
    }
    setError(null);
    onOpen(trimmed, kind);
  }

  async function browseWorkspace() {
    setPickerBusy(true);
    setError(null);
    try {
      const selected = await selectDesktopWorkspace();
      if (selected) {
        setPath(selected);
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : t("workspace.openPickerFailed"));
    } finally {
      setPickerBusy(false);
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLFormElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onCancel();
      return;
    }
    if (event.key !== "Tab") {
      return;
    }
    event.stopPropagation();
    const focusable = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(
        "button:not([disabled]), input:not([disabled]), select:not([disabled])",
      ),
    );
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

  return (
    <div className="modal-backdrop" role="presentation">
      <form
        className="modal-card"
        onSubmit={handleSubmit}
        onKeyDown={handleKeyDown}
        role="dialog"
        aria-modal="true"
        aria-labelledby="open-workspace-title"
      >
        <h2 id="open-workspace-title">{t("empty.open")}</h2>
        <p className="modal-card__lede">
          {t("workspace.openDescription")}
        </p>
        <div className="field">
          <label htmlFor="workspace-path">{t("empty.absolutePath")}</label>
          <div className="workspace-path-control">
            <input
              id="workspace-path"
              value={path}
              onChange={(event) => setPath(event.target.value)}
              placeholder="D:\\Study\\project\\agent\\rove"
              autoFocus
              aria-invalid={error ? "true" : undefined}
              aria-describedby={error ? "workspace-path-error" : undefined}
            />
            {nativePickerAvailable ? (
              <button
                type="button"
                className="secondary"
                onClick={() => void browseWorkspace()}
                disabled={pickerBusy}
              >
                <FileIcon aria-hidden="true" />
                {pickerBusy ? t("common.loading") : t("empty.browse")}
              </button>
            ) : null}
          </div>
        </div>
        <div className="field">
          <label htmlFor="workspace-kind">{t("empty.kind")}</label>
          <select
            id="workspace-kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as WorkspaceKind)}
          >
            <option value="folder">{t("empty.folder")}</option>
            <option value="repo">{t("empty.repo")}</option>
          </select>
        </div>
        {error ? (
          <div className="chat-error" id="workspace-path-error" role="alert">
            {error}
          </div>
        ) : null}
        <div className="modal-actions">
          <button type="button" className="secondary" onClick={onCancel}>
            {t("common.cancel")}
          </button>
          <button type="submit">{t("empty.open")}</button>
        </div>
      </form>
    </div>
  );
}
