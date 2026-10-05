"use client";

import { FileIcon } from "@radix-ui/react-icons";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  desktopWorkspacePickerAvailable,
  selectDesktopWorkspace,
} from "../platform/desktop-commands";
import { createProductApiClient } from "../product/product-client";
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
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [path, setPath] = useState("");
  const [kind, setKind] = useState<WorkspaceKind>("folder");
  const [error, setError] = useState<string | null>(null);
  const [pickerBusy, setPickerBusy] = useState(false);
  const pickerGeneration = useRef(0);

  useEffect(() => () => {
    pickerGeneration.current += 1;
  }, []);

  function invalidatePicker() {
    pickerGeneration.current += 1;
    setPickerBusy(false);
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    invalidatePicker();
    const trimmed = path.trim();
    if (!trimmed) {
      setError(t("empty.pathRequired"));
      return;
    }
    setError(null);
    onOpenWorkspace(trimmed, kind);
  }

  async function browseWorkspace() {
    const generation = ++pickerGeneration.current;
    setPickerBusy(true);
    setError(null);
    try {
      if (desktopWorkspacePickerAvailable()) {
        const selected = await selectDesktopWorkspace();
        if (generation !== pickerGeneration.current) return;
        if (selected) {
          setPath(selected);
        }
        return;
      }
      const result = await client.pickWorkspaceFolder();
      if (generation !== pickerGeneration.current) return;
      if (result.status === "selected") {
        setPath(result.path);
        return;
      }
      if (result.status === "unavailable") {
        setError(t("empty.pickerUnavailable"));
      }
    } catch (caught) {
      if (generation !== pickerGeneration.current) return;
      setError(caught instanceof Error ? caught.message : t("workspace.openPickerFailed"));
    } finally {
      if (generation === pickerGeneration.current) setPickerBusy(false);
    }
  }

  return (
    <div className="empty-state">
      <div className="empty-state__card">
        <h1>{t("empty.title")}</h1>
        <p>{t("empty.body")}</p>
        <form onSubmit={handleSubmit} className="settings-card" style={{ padding: 0, border: "none", background: "transparent" }}>
          <div className="empty-state__actions" style={{ justifyContent: "flex-start", marginBottom: 8 }}>
            <button
              type="button"
              onClick={() => void browseWorkspace()}
              disabled={pickerBusy}
            >
              <FileIcon aria-hidden="true" />
              {pickerBusy ? t("common.loading") : t("empty.browseFolder")}
            </button>
          </div>
          <p className="settings-inline-note">{t("empty.browseFolderHint")}</p>
          <div className="field">
            <label htmlFor="empty-workspace-path">{t("empty.absolutePath")}</label>
            <div className="workspace-path-control">
              <input
                id="empty-workspace-path"
                value={path}
                onChange={(event) => {
                  invalidatePicker();
                  setPath(event.target.value);
                }}
                placeholder="D:\\path\\to\\project"
                aria-invalid={error ? "true" : undefined}
                aria-describedby={error ? "empty-workspace-error" : undefined}
              />
            </div>
          </div>
          <div className="field">
            <label htmlFor="empty-workspace-kind">{t("empty.kind")}</label>
            <select
              id="empty-workspace-kind"
              value={kind}
              onChange={(event) => {
                invalidatePicker();
                setKind(event.target.value as WorkspaceKind);
              }}
            >
              <option value="folder">{t("empty.folder")}</option>
              <option value="repo">{t("empty.repo")}</option>
            </select>
          </div>
          {error ? (
            <div className="chat-error" id="empty-workspace-error" role="alert">
              {error}
            </div>
          ) : null}
          <div className="empty-state__actions">
            <button type="submit" className="secondary">{t("empty.open")}</button>
            <button type="button" className="ghost" onClick={() => {
              invalidatePicker();
              onOpenProviders();
            }}>
              {t("empty.configureProvider")}
            </button>
          </div>
        </form>
        {recents.length > 0 ? (
          <div className="empty-state__recents">
            <h3>{t("empty.recents")}</h3>
            {recents.map((workspace) => (
              <button
                key={workspace.id}
                type="button"
                className="recent-item"
                onClick={() => {
                  invalidatePicker();
                  onOpenRecent(workspace.id);
                }}
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
