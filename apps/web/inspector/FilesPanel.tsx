"use client";

import {
  ChevronUpIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FileIcon,
  ImageIcon,
} from "@radix-ui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type {
  ProductFileContentEnvelope,
  ProductFileEntry,
  ProductPreviewSession,
} from "../product/product-api-types";

function isHtmlPath(path: string): boolean {
  return /\.(html|htm)$/i.test(path);
}

export function FilesPanel({
  workspaceId,
  focusPath,
  focusLine,
}: {
  workspaceId: string;
  focusPath?: string | null;
  focusLine?: number | null;
}) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [prefix, setPrefix] = useState("");
  const [entries, setEntries] = useState<ProductFileEntry[]>([]);
  // Listing and preview are independent: opening a file must not discard a page.
  const requestRef = useRef(0);
  const previewRequestRef = useRef(0);
  const downloadRequestRef = useRef(0);
  useEffect(() => () => {
    requestRef.current += 1;
    previewRequestRef.current += 1;
    downloadRequestRef.current += 1;
  }, [workspaceId]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [scanLimited, setScanLimited] = useState(false);
  const [content, setContent] = useState<ProductFileContentEnvelope | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [htmlPreview, setHtmlPreview] = useState<ProductPreviewSession | null>(null);
  const [htmlPreviewBusy, setHtmlPreviewBusy] = useState(false);
  const [htmlPreviewError, setHtmlPreviewError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  }, [previewUrl]);

  // Closing the panel (or switching workspace) revokes the isolated preview
  // token so the loopback origin stops resolving it (plan P5b / A8).
  useEffect(() => {
    return () => {
      const open = htmlPreview;
      if (open) {
        void client
          .closeWorkspacePreview(open.workspace_id, open.preview_id)
          .catch(() => undefined);
      }
    };
  }, [client, htmlPreview, workspaceId]);

  async function openHtmlPreview(path: string) {
    setHtmlPreviewBusy(true);
    setHtmlPreviewError(null);
    try {
      const session = await client.createWorkspacePreview(workspaceId, { path });
      if (session.workspace_id !== workspaceId) {
        throw new Error("Preview session does not match this workspace.");
      }
      const previous = htmlPreview;
      if (previous) {
        void client
          .closeWorkspacePreview(previous.workspace_id, previous.preview_id)
          .catch(() => undefined);
      }
      setHtmlPreview(session);
      // The preview URL embeds the session token. Open it only in a new
      // isolated tab and never write it to logs or persistent state (A7).
      window.open(session.url, "_blank", "noopener,noreferrer");
    } catch (caught) {
      setHtmlPreviewError(
        caught instanceof Error ? caught.message : "Failed to open preview",
      );
    } finally {
      setHtmlPreviewBusy(false);
    }
  }

  async function closeHtmlPreview() {
    const open = htmlPreview;
    if (!open) {
      return;
    }
    setHtmlPreviewBusy(true);
    setHtmlPreviewError(null);
    try {
      await client.closeWorkspacePreview(open.workspace_id, open.preview_id);
      setHtmlPreview(null);
    } catch (caught) {
      setHtmlPreviewError(
        caught instanceof Error ? caught.message : "Failed to close preview",
      );
    } finally {
      setHtmlPreviewBusy(false);
    }
  }

  useEffect(() => {
    const request = ++requestRef.current;
    previewRequestRef.current += 1;
    downloadRequestRef.current += 1;
    const stale = () => requestRef.current !== request;
    async function load() {
      setLoading(true);
      setError(null);
      setContent(null);
      setPreviewUrl(null);
      try {
        const response = await client.listWorkspaceFiles(workspaceId, {
          prefix: prefix || undefined,
          limit: 100,
        });
        if (!stale()) {
          setEntries(response.entries);
          setNextCursor(response.next_cursor ?? null);
          setScanLimited(response.scan_limit_reached);
        }
      } catch (caught) {
        if (!stale()) {
          setError(caught instanceof Error ? caught.message : "Failed to list files");
        }
      } finally {
        if (!stale()) {
          setLoading(false);
        }
      }
    }
    void load();
    return () => { requestRef.current += 1; };
  }, [client, workspaceId, prefix]);

  useEffect(() => {
    if (!focusPath) {
      return;
    }
    const request = ++previewRequestRef.current;
    const stale = () => previewRequestRef.current !== request;
    async function loadFocusedFile() {
      setError(null);
      try {
        const nextContent = await client.getWorkspaceFileContent(workspaceId, focusPath!);
        const nextPreviewUrl =
          nextContent.image && nextContent.preview_allowed
            ? URL.createObjectURL(
                await client.fetchWorkspaceFilePreview(workspaceId, focusPath!),
              )
            : null;
        if (!stale()) {
          setContent(nextContent);
          setPreviewUrl(nextPreviewUrl);
        } else if (nextPreviewUrl) {
          URL.revokeObjectURL(nextPreviewUrl);
        }
      } catch (caught) {
        if (!stale()) {
          setError(caught instanceof Error ? caught.message : "Failed to open finding file");
        }
      }
    }
    void loadFocusedFile();
    return () => { previewRequestRef.current += 1; };
  }, [client, focusPath, workspaceId]);

  async function loadMore() {
    if (!nextCursor || loading) return;
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    setLoading(true);
    setError(null);
    try {
      const response = await client.listWorkspaceFiles(workspaceId, {
        prefix: prefix || undefined,
        cursor: nextCursor,
        limit: 100,
      });
      if (!stale()) {
        setEntries((current) => [...current, ...response.entries]);
        setNextCursor(response.next_cursor ?? null);
        setScanLimited(response.scan_limit_reached);
      }
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to load more files");
      }
    } finally {
      if (!stale()) {
        setLoading(false);
      }
    }
  }

  async function openFile(entry: ProductFileEntry) {
    if (entry.kind === "directory") {
      setPrefix(entry.path);
      return;
    }
    const request = ++previewRequestRef.current;
    const stale = () => previewRequestRef.current !== request;
    setContent(null);
    setPreviewUrl(null);
    setError(null);
    try {
      const nextContent = await client.getWorkspaceFileContent(workspaceId, entry.path);
      const nextPreviewUrl =
        nextContent.image && nextContent.preview_allowed
          ? URL.createObjectURL(
              await client.fetchWorkspaceFilePreview(workspaceId, entry.path),
            )
          : null;
      if (!stale()) {
        setContent(nextContent);
        setPreviewUrl(nextPreviewUrl);
      } else if (nextPreviewUrl) {
        URL.revokeObjectURL(nextPreviewUrl);
      }
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to read file");
      }
    }
  }

  async function downloadFile(path: string) {
    const request = ++downloadRequestRef.current;
    const stale = () => downloadRequestRef.current !== request;
    setError(null);
    try {
      await downloadBlob(
        await client.fetchWorkspaceFileDownload(workspaceId, path),
        filenameForPath(path),
      );
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to download file");
      }
    }
  }

  return (
    <section className="inspector-section" aria-label={t("inspector.files")}>
      <div className="inspector-section__heading">
        <h3>{t("inspector.files")}</h3>
        {prefix ? (
          <button
            type="button"
            className="ghost icon-button"
            onClick={() => {
              const parts = prefix.split("/").filter(Boolean);
              parts.pop();
              setPrefix(parts.join("/"));
            }}
            aria-label={t("inspector.filesParent")}
            title={t("inspector.filesParent")}
          >
            <ChevronUpIcon />
          </button>
        ) : null}
      </div>
      <p className="inspector-empty-line"><code>{prefix || "/"}</code></p>
      {loading && entries.length === 0 ? (
        <p className="inspector-empty-line">{t("inspector.filesLoading")}</p>
      ) : null}
      {error ? <p className="inspector-empty-line" role="alert">{t("chrome.loadError")}</p> : null}
      <ul className="evidence-file-list">
        {entries.map((entry) => (
          <li key={entry.path}>
            <button
              type="button"
              className="ghost evidence-file-list__open"
              onClick={() => void openFile(entry)}
            >
              <FileIcon aria-hidden="true" />
              <span>{entry.path}</span>
              <small>{entry.kind === "directory" ? t("chrome.directory") : formatBytes(entry.size)}</small>
            </button>
            {entry.kind === "file" ? (
              <button
                type="button"
                className="ghost icon-button"
                onClick={() => void downloadFile(entry.path)}
                aria-label={t("chrome.download", { name: entry.path })}
                title={t("chrome.download", { name: entry.path })}
              >
                <DownloadIcon />
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      {nextCursor ? (
        <button type="button" className="ghost" onClick={() => void loadMore()} disabled={loading}>
          {loading ? t("common.loading") : t("chrome.loadMore")}
        </button>
      ) : null}
      {scanLimited ? (
        <p className="inspector-empty-line" role="status">
          {t("chrome.scanLimited")}
        </p>
      ) : null}
      {content ? (
        <div className="evidence-preview">
          <div className="evidence-preview__heading">
            <div>
              <strong>{content.path}</strong>
              <span>{content.mime} · {formatBytes(content.size)}{content.truncated ? ` · ${t("chrome.truncated")}` : ""}</span>
            </div>
            <button
              type="button"
              className="ghost icon-button"
              onClick={() => void downloadFile(content.path)}
              aria-label={t("chrome.download", { name: content.path })}
              title={t("chrome.download", { name: content.path })}
            >
              <DownloadIcon />
            </button>
            {isHtmlPath(content.path) ? (
              htmlPreview ? (
                <button
                  type="button"
                  className="ghost icon-button"
                  onClick={() => void closeHtmlPreview()}
                  disabled={htmlPreviewBusy}
                  aria-label={t("chrome.previewHtmlClose")}
                  title={t("chrome.previewHtmlClose")}
                  data-testid="close-html-preview"
                >
                  {t("chrome.previewHtmlClose")}
                </button>
              ) : (
                <button
                  type="button"
                  className="ghost icon-button"
                  onClick={() => void openHtmlPreview(content.path)}
                  disabled={htmlPreviewBusy}
                  aria-label={t("chrome.previewHtmlOpen")}
                  title={t("chrome.previewHtmlOpen")}
                  data-testid="open-html-preview"
                >
                  <ExternalLinkIcon />
                </button>
              )
            ) : null}
          </div>
          {isHtmlPath(content.path) ? (
            <p className="inspector-empty-line" role="status">
              {htmlPreview
                ? t("chrome.previewHtmlActive")
                : t("chrome.previewHtmlHint")}
            </p>
          ) : null}
          {htmlPreviewError ? (
            <p className="inspector-empty-line" role="alert">
              {htmlPreviewError}
            </p>
          ) : null}
          {content.validation_error ? (
            <p className="inspector-empty-line" role="alert">{t("chrome.invalidContent")}</p>
          ) : null}
          {content.text !== undefined ? (
            <pre className="evidence-preview__text" data-focus-line={focusLine ?? undefined}>
              {(() => {
                const lines = content.text.split("\n");
                return lines.map((line, index) => (
                  <span
                    key={`${content.path}-${index}`}
                    data-line={index + 1}
                    data-focused={focusLine === index + 1 ? "true" : undefined}
                  >
                    {line || " "}{index < lines.length - 1 ? "\n" : ""}
                  </span>
                ));
              })()}
            </pre>
          ) : null}
          {content.image && content.preview_allowed ? (
            <figure className="evidence-preview__image">
              <img
                src={previewUrl ?? undefined}
                alt={content.path}
              />
              <figcaption>
                <ImageIcon aria-hidden="true" /> {content.image.width} × {content.image.height} {content.image.format}
              </figcaption>
            </figure>
          ) : null}
          {content.text === undefined && !content.image && !content.validation_error ? (
            <p className="inspector-empty-line">{t("chrome.previewUnavailable")}</p>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

async function downloadBlob(blob: Blob, filename: string): Promise<void> {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.hidden = true;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

function filenameForPath(path: string): string {
  const filename = path.split("/").filter(Boolean).pop();
  return filename || "download";
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}
