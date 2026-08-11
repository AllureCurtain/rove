"use client";

import { FileIcon } from "@radix-ui/react-icons";
import { FormEvent, useEffect, useState } from "react";

import {
  desktopWorkspacePickerAvailable,
  selectDesktopWorkspace,
} from "../platform/desktop-commands";
import type { WorkspaceKind, WorkspaceRecord } from "../state/product-types";

export function EmptyState({
  recents,
  onOpenWorkspace,
  onOpenRecent,
  onOpenProviders,
}: {
  recents: WorkspaceRecord[];
  onOpenWorkspace: (path: string, kind: WorkspaceKind) => void;
  onOpenRecent: (workspaceId: string) => void;
  onOpenProviders: () => void;
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
      setError("Enter an absolute path to open.");
      return;
    }
    setError(null);
    onOpenWorkspace(trimmed, kind);
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

  return (
    <div className="empty-state">
      <div className="empty-state__card">
        <h1>Open a workspace to start</h1>
        <p>
          rove runs agent turns against a real local root. Open a folder or repo path,
          then chat in a session with durable hard resume.
        </p>
        <form onSubmit={handleSubmit} className="settings-card" style={{ padding: 0, border: "none", background: "transparent" }}>
          <div className="field">
            <label htmlFor="empty-workspace-path">Absolute path</label>
            <div className="workspace-path-control">
              <input
                id="empty-workspace-path"
                value={path}
                onChange={(event) => setPath(event.target.value)}
                placeholder="D:\\path\\to\\project"
                aria-invalid={error ? "true" : undefined}
                aria-describedby={error ? "empty-workspace-error" : undefined}
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
            <label htmlFor="empty-workspace-kind">Kind</label>
            <select
              id="empty-workspace-kind"
              value={kind}
              onChange={(event) => setKind(event.target.value as WorkspaceKind)}
            >
              <option value="folder">Folder</option>
              <option value="repo">Repo</option>
            </select>
          </div>
          {error ? (
            <div className="chat-error" id="empty-workspace-error" role="alert">
              {error}
            </div>
          ) : null}
          <div className="empty-state__actions">
            <button type="submit">Open workspace</button>
            <button type="button" className="secondary" onClick={onOpenProviders}>
              Configure provider
            </button>
          </div>
        </form>
        {recents.length > 0 ? (
          <div className="empty-state__recents">
            <h3>Recents</h3>
            {recents.map((workspace) => (
              <button
                key={workspace.id}
                type="button"
                className="recent-item"
                onClick={() => onOpenRecent(workspace.id)}
              >
                <span>
                  <strong>{workspace.displayName}</strong>
                  <span>{workspace.rootPath}</span>
                </span>
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}
