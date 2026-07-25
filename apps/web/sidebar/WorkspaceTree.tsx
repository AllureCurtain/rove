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
  onOpenWorkspace: (path: string, kind: WorkspaceKind) => void;
  onSelectWorkspace: (workspaceId: string) => void;
  onSelectSession: (workspaceId: string, sessionId: string) => void;
  onNewSession: (workspaceId: string) => void;
  onTogglePin: (workspaceId: string) => void;
  onRemoveWorkspace: (workspaceId: string) => void;
}) {
  const [openDialog, setOpenDialog] = useState(false);

  return (
    <aside className="product-sidebar" aria-label="Workspaces">
      <div className="product-sidebar__header">
        <h2>Workspaces</h2>
        <button
          type="button"
          className="secondary icon-button"
          onClick={() => setOpenDialog(true)}
          aria-label="Add workspace"
        >
          <PlusIcon />
        </button>
      </div>
      <div className="product-sidebar__scroll">
        {workspaces.length === 0 ? (
          <p style={{ color: "var(--muted)", fontSize: "0.9rem", padding: "8px" }}>
            No workspaces yet. Open a local folder or repo path.
          </p>
        ) : (
          workspaces.map((workspace) => {
            const sessions = sessionsByWorkspace[workspace.id] ?? [];
            const active = workspace.id === activeWorkspaceId;
            return (
              <div className="workspace-group" key={workspace.id}>
                <div className="workspace-group__row">
                  <button
                    type="button"
                    className="workspace-group__button"
                    data-active={active}
                    onClick={() => onSelectWorkspace(workspace.id)}
                  >
                    <span>{workspace.displayName}</span>
                    <span className="workspace-group__path">{workspace.rootPath}</span>
                  </button>
                  <button
                    type="button"
                    className="ghost icon-button"
                    aria-label={workspace.pinned ? "Unpin workspace" : "Pin workspace"}
                    onClick={() => onTogglePin(workspace.id)}
                  >
                    {workspace.pinned ? <DrawingPinFilledIcon /> : <DrawingPinIcon />}
                  </button>
                  <button
                    type="button"
                    className="ghost icon-button"
                    aria-label="Remove workspace from list"
                    onClick={() => onRemoveWorkspace(workspace.id)}
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
                            onClick={() => onSelectSession(workspace.id, session.id)}
                          >
                            <span>{session.title}</span>
                            <span
                              className="session-item__status"
                              data-status={session.status}
                              aria-hidden="true"
                            />
                          </button>
                        </li>
                      ))}
                    </ul>
                    <div style={{ padding: "4px 0 8px 14px" }}>
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => onNewSession(workspace.id)}
                      >
                        New session
                      </button>
                    </div>
                  </>
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
        <p style={{ margin: 0, color: "var(--muted)", fontSize: "0.9rem" }}>
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
