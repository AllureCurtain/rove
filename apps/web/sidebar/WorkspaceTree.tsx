"use client";

import { FormEvent, useState } from "react";
import {
  Cross2Icon,
  DrawingPinFilledIcon,
  DrawingPinIcon,
  PlusIcon,
} from "@radix-ui/react-icons";

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
}) {
  const [openDialog, setOpenDialog] = useState(false);

  return (
    <aside
      className="product-sidebar"
      aria-label="Workspaces"
      aria-busy={mutationBusy}
    >
      <div className="product-sidebar__header">
        <h2>Workspaces</h2>
        <button
          type="button"
          className="secondary icon-button"
          onClick={() => setOpenDialog(true)}
          aria-label="Add workspace"
          disabled={mutationBusy}
        >
          <PlusIcon />
        </button>
      </div>
      <div className="product-sidebar__scroll">
        {workspaces.length === 0 ? (
          <p className="sidebar-empty">No workspaces yet. Open a local folder or repo path.</p>
        ) : (
          workspaces.map((workspace) => {
            const sessions = sessionsByWorkspace[workspace.id] ?? [];
            const active = workspace.id === activeWorkspaceId;
            const runningCount = sessions.filter((session) => session.status === "running").length;
            const attentionCount = sessions.filter(
              (session) => session.status === "needs_attention",
            ).length;
            const errorCount = sessions.filter((session) => session.status === "error").length;
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
                {active ? (
                  <>
                    <ul className="session-list">
                      {sessions.map((session) => (
                        <li key={session.id}>
                          <button
                            type="button"
                            className="session-item"
                            data-active={session.id === activeSessionId}
                            data-status={session.status}
                            onClick={() => onSelectSession(workspace.id, session.id)}
                            aria-label={sessionAriaLabel(session)}
                            disabled={mutationBusy}
                          >
                            <span className="session-item__title">{session.title}</span>
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
                        </li>
                      ))}
                    </ul>
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
      {openDialog ? (
        <OpenWorkspaceDialog
          onCancel={() => setOpenDialog(false)}
          onOpen={(path, kind) => {
            onOpenWorkspace(path, kind);
            setOpenDialog(false);
          }}
        />
      ) : null}
    </aside>
  );
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

function sessionAriaLabel(session: SessionRecord): string {
  if (session.status === "idle") {
    return session.title;
  }
  return `${session.title}, ${sessionStatusLabel(session.status)}`;
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

  return (
    <div className="modal-backdrop" role="presentation">
      <form className="modal-card" onSubmit={handleSubmit} role="dialog" aria-label="Open workspace">
        <h2>Open workspace</h2>
        <p className="modal-card__lede">
          Bind the agent to an absolute local folder or repository path. No full-disk scan.
        </p>
        <div className="field">
          <label htmlFor="workspace-path">Absolute path</label>
          <input
            id="workspace-path"
            value={path}
            onChange={(event) => setPath(event.target.value)}
            placeholder="D:\\Study\\project\\agent\\rove"
            autoFocus
          />
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
        {error ? <div className="chat-error">{error}</div> : null}
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
