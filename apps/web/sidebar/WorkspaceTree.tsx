"use client";

import {
  FormEvent,
  type KeyboardEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  Cross2Icon,
  DrawingPinFilledIcon,
  DrawingPinIcon,
  FileIcon,
  GearIcon,
  LockClosedIcon,
  MagnifyingGlassIcon,
  PlusIcon,
} from "@radix-ui/react-icons";

import {
  desktopWorkspacePickerAvailable,
  selectDesktopWorkspace,
} from "../platform/desktop-commands";
import type { SessionRecord, WorkspaceKind, WorkspaceRecord } from "../state/product-types";

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
  mobileOpen = false,
  onCloseMobile,
  onOpenSettings,
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
  onCloseMobile?: () => void;
  onOpenSettings?: () => void;
}) {
  const [openDialog, setOpenDialog] = useState(false);
  const [query, setQuery] = useState("");
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
    window.requestAnimationFrame(() => addWorkspaceButtonRef.current?.focus());
  }

  return (
    <aside
      ref={sidebarRef}
      className="product-sidebar"
      aria-label="Workspaces"
      aria-busy={mutationBusy}
      data-open={mobileOpen}
      aria-modal={mobileOpen ? true : undefined}
      role={mobileOpen ? "dialog" : undefined}
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
        <h2>Workspaces</h2>
        <button
          ref={addWorkspaceButtonRef}
          type="button"
          className="secondary icon-button"
          onClick={() => setOpenDialog(true)}
          aria-label="Add workspace"
          disabled={mutationBusy}
        >
          <PlusIcon />
        </button>
        {onCloseMobile && mobileOpen ? (
          <button
            ref={closeMobileButtonRef}
            type="button"
            className="ghost icon-button mobile-only"
            aria-label="Close workspaces"
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
          aria-label="Search workspaces and sessions"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search sessions"
        />
      </label>
      <div className="product-sidebar__scroll">
        {workspaces.length === 0 ? (
          <p className="sidebar-empty">No workspaces yet. Open a local folder or repo path.</p>
        ) : (
          workspaces.map((workspace) => {
            const normalizedQuery = query.trim().toLocaleLowerCase();
            const allSessions = sessionsByWorkspace[workspace.id] ?? [];
            const workspaceMatches = `${workspace.displayName} ${workspace.rootPath}`
              .toLocaleLowerCase()
              .includes(normalizedQuery);
            const sessions = normalizedQuery && !workspaceMatches
              ? allSessions.filter((session) =>
                  session.title.toLocaleLowerCase().includes(normalizedQuery),
                )
              : allSessions;
            if (normalizedQuery && !workspaceMatches && sessions.length === 0) {
              return null;
            }
            const active = workspace.id === activeWorkspaceId;
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
                    className="workspace-group__button"
                    data-active={active}
                    onClick={() => onSelectWorkspace(workspace.id)}
                    disabled={mutationBusy}
                  >
                    <span className="workspace-group__title">
                      <span>{workspace.displayName}</span>
                      {runningCount > 0 ? (
                        <span
                          className="session-badge"
                          data-status="running"
                          title={`${runningCount} session${runningCount === 1 ? "" : "s"} running`}
                        >
                          {runningCount === 1 ? "Running" : `${runningCount} running`}
                        </span>
                      ) : null}
                      {runningCount === 0 && attentionCount > 0 ? (
                        <span className="session-badge" data-status="needs_attention">
                          Needs attention
                        </span>
                      ) : null}
                      {runningCount === 0 && attentionCount === 0 && errorCount > 0 ? (
                        <span className="session-badge" data-status="error">
                          Error
                        </span>
                      ) : null}
                    </span>
                    <span className="workspace-group__path">{workspace.rootPath}</span>
                  </button>
                  <button
                    type="button"
                    className="ghost icon-button"
                    aria-label={workspace.pinned ? "Unpin workspace" : "Pin workspace"}
                    onClick={() => onTogglePin(workspace.id)}
                    disabled={mutationBusy}
                  >
                    {workspace.pinned ? <DrawingPinFilledIcon /> : <DrawingPinIcon />}
                  </button>
                  <button
                    type="button"
                    className="ghost icon-button"
                    aria-label="Remove workspace from list"
                    onClick={() => onRemoveWorkspace(workspace.id)}
                    disabled={mutationBusy}
                  >
                    <Cross2Icon />
                  </button>
                </div>
                {active || normalizedQuery ? (
                  <>
                    <SessionBranchList
                      sessions={sessions}
                      workspaceId={workspace.id}
                      activeSessionId={activeSessionId}
                      mutationBusy={mutationBusy}
                      onSelectSession={onSelectSession}
                    />
                    <div className="session-list__actions">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => onNewSession(workspace.id)}
                        disabled={mutationBusy}
                      >
                        New session
                      </button>
                    </div>
                  </>
                ) : runningCount > 0 ? (
                  <p className="workspace-group__parallel" role="status">
                    {runningCount} parallel session{runningCount === 1 ? "" : "s"} still running
                  </p>
                ) : null}
              </div>
            );
          })
        )}
      </div>
      <footer className="product-sidebar__footer">
        <div className="workspace-boundary">
          <LockClosedIcon />
          <span><strong>Bounded roots</strong><small>API-authoritative workspaces</small></span>
        </div>
        {onOpenSettings ? (
          <button type="button" className="ghost" onClick={onOpenSettings}>
            <GearIcon /> Product settings
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

function SessionBranchList({
  sessions,
  workspaceId,
  activeSessionId,
  mutationBusy,
  onSelectSession,
}: {
  sessions: SessionRecord[];
  workspaceId: string;
  activeSessionId: string | null;
  mutationBusy: boolean;
  onSelectSession: (workspaceId: string, sessionId: string) => void;
}) {
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
    <ul className="session-list" aria-label="Sessions and branches">
      {roots.map((session) => (
        <SessionBranch
          key={session.id}
          session={session}
          childrenByParent={childrenByParent}
          parentAvailable={
            !session.parentSessionId || visibleSessionIds.has(session.parentSessionId)
          }
          workspaceId={workspaceId}
          activeSessionId={activeSessionId}
          mutationBusy={mutationBusy}
          onSelectSession={onSelectSession}
        />
      ))}
    </ul>
  );
}

function SessionBranch({
  session,
  childrenByParent,
  parentAvailable,
  workspaceId,
  activeSessionId,
  mutationBusy,
  onSelectSession,
}: {
  session: SessionRecord;
  childrenByParent: Map<string, SessionRecord[]>;
  parentAvailable: boolean;
  workspaceId: string;
  activeSessionId: string | null;
  mutationBusy: boolean;
  onSelectSession: (workspaceId: string, sessionId: string) => void;
}) {
  const children = childrenByParent.get(session.id) ?? [];
  return (
    <li
      className="session-branch"
      data-forked={session.parentSessionId ? "true" : undefined}
      data-orphaned={session.parentSessionId && !parentAvailable ? "true" : undefined}
    >
      <button
        type="button"
        className="session-item"
        data-active={session.id === activeSessionId}
        data-status={session.status}
        onClick={() => onSelectSession(workspaceId, session.id)}
        aria-label={sessionAriaLabel(session, parentAvailable)}
        disabled={mutationBusy}
      >
        <span className="session-item__title">
          <span>{session.title}</span>
          {session.parentSessionId ? (
            <small className="session-item__lineage">
              {forkPointLabel(session, parentAvailable)}
            </small>
          ) : null}
        </span>
        {session.status !== "idle" ? (
          <span className="session-badge" data-status={session.status}>
            {sessionStatusLabel(session.status)}
          </span>
        ) : null}
        <span
          className="session-item__status"
          data-status={session.status}
          aria-hidden="true"
        />
      </button>
      {children.length > 0 ? (
        <ul className="session-list session-list--branch">
          {children.map((child) => (
            <SessionBranch
              key={child.id}
              session={child}
              childrenByParent={childrenByParent}
              parentAvailable
              workspaceId={workspaceId}
              activeSessionId={activeSessionId}
              mutationBusy={mutationBusy}
              onSelectSession={onSelectSession}
            />
          ))}
        </ul>
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

function sessionStatusLabel(status: SessionRecord["status"]): string {
  switch (status) {
    case "running":
      return "Running";
    case "needs_attention":
      return "Attention";
    case "error":
      return "Error";
    default:
      return "Idle";
  }
}

function sessionAriaLabel(session: SessionRecord, parentAvailable: boolean): string {
  const lineage = session.parentSessionId
    ? parentAvailable
      ? "Forked session, "
      : "Forked session with removed parent, "
    : "";
  if (session.status === "idle") {
    return `${lineage}${session.title}`;
  }
  return `${lineage}${session.title}, ${sessionStatusLabel(session.status)}`;
}

function forkPointLabel(session: SessionRecord, parentAvailable: boolean): string {
  const source = session.forkPointRunId
    ? shortId(session.forkPointRunId)
    : "boundary unavailable";
  const sequence = session.forkPointSeq ? `event ${session.forkPointSeq}` : "event unavailable";
  return `${parentAvailable ? "Fork" : "Parent removed"} · ${source} · ${sequence}`;
}

function shortId(value: string): string {
  return value.length <= 10 ? value : value.slice(0, 10);
}

function OpenWorkspaceDialog({
  onOpen,
  onCancel,
}: {
  onOpen: (path: string, kind: WorkspaceKind) => void;
  onCancel: () => void;
}) {
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
      setError("Enter an absolute path.");
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
      setError(caught instanceof Error ? caught.message : "Failed to open folder picker.");
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
        <h2 id="open-workspace-title">Open workspace</h2>
        <p className="modal-card__lede">
          Bind the agent to an absolute local folder or repository path. No full-disk scan.
        </p>
        <div className="field">
          <label htmlFor="workspace-path">Absolute path</label>
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
                {pickerBusy ? "Opening..." : "Browse"}
              </button>
            ) : null}
          </div>
        </div>
        <div className="field">
          <label htmlFor="workspace-kind">Kind</label>
          <select
            id="workspace-kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as WorkspaceKind)}
          >
            <option value="folder">Folder</option>
            <option value="repo">Repo</option>
          </select>
        </div>
        {error ? (
          <div className="chat-error" id="workspace-path-error" role="alert">
            {error}
          </div>
        ) : null}
        <div className="modal-actions">
          <button type="button" className="secondary" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit">Open</button>
        </div>
      </form>
    </div>
  );
}
