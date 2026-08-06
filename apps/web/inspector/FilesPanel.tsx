"use client";

import {
  ChevronUpIcon,
  DownloadIcon,
  FileIcon,
  ImageIcon,
} from "@radix-ui/react-icons";
import { useEffect, useMemo, useState } from "react";

import { createProductApiClient } from "../product/product-client";
import type {
  ProductFileContentEnvelope,
  ProductFileEntry,
} from "../product/product-api-types";

export function FilesPanel({ workspaceId }: { workspaceId: string }) {
  const client = useMemo(() => createProductApiClient(), []);
  const [prefix, setPrefix] = useState("");
  const [entries, setEntries] = useState<ProductFileEntry[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [scanLimited, setScanLimited] = useState(false);
  const [content, setContent] = useState<ProductFileContentEnvelope | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      setError(null);
      setContent(null);
      try {
        const response = await client.listWorkspaceFiles(workspaceId, {
          prefix: prefix || undefined,
          limit: 100,
        });
        if (!cancelled) {
          setEntries(response.entries);
          setNextCursor(response.next_cursor ?? null);
          setScanLimited(response.scan_limit_reached);
        }
      } catch (caught) {
        if (!cancelled) {
          setError(caught instanceof Error ? caught.message : "Failed to list files");
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [client, workspaceId, prefix]);

  async function loadMore() {
    if (!nextCursor || loading) return;
    setLoading(true);
    setError(null);
    try {
      const response = await client.listWorkspaceFiles(workspaceId, {
        prefix: prefix || undefined,
        cursor: nextCursor,
        limit: 100,
      });
      setEntries((current) => [...current, ...response.entries]);
      setNextCursor(response.next_cursor ?? null);
      setScanLimited(response.scan_limit_reached);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to load more files");
    } finally {
      setLoading(false);
    }
  }

  async function openFile(entry: ProductFileEntry) {
    if (entry.kind === "directory") {
      setPrefix(entry.path);
      return;
    }
    setError(null);
    try {
      setContent(await client.getWorkspaceFileContent(workspaceId, entry.path));
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to read file");
    }
  }

  return (
    <section className="inspector-section" aria-label="Workspace files">
      <div className="inspector-section__heading">
        <h3>Files</h3>
        {prefix ? (
          <button
            type="button"
            className="ghost icon-button"
            onClick={() => {
              const parts = prefix.split("/").filter(Boolean);
              parts.pop();
              setPrefix(parts.join("/"));
            }}
            aria-label="Open parent directory"
            title="Open parent directory"
          >
            <ChevronUpIcon />
          </button>
        ) : null}
      </div>
      <p className="inspector-empty-line"><code>{prefix || "/"}</code></p>
      {loading && entries.length === 0 ? (
        <p className="inspector-empty-line">Loading files…</p>
      ) : null}
      {error ? <p className="inspector-empty-line" role="alert">{error}</p> : null}
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
              <small>{entry.kind === "directory" ? "directory" : formatBytes(entry.size)}</small>
            </button>
            {entry.kind === "file" ? (
              <a
                className="ghost icon-button"
                href={client.workspaceFileDownloadUrl(workspaceId, entry.path)}
                download
                aria-label={`Download ${entry.path}`}
                title={`Download ${entry.path}`}
              >
                <DownloadIcon />
              </a>
            ) : null}
          </li>
        ))}
      </ul>
      {nextCursor ? (
        <button type="button" className="ghost" onClick={() => void loadMore()} disabled={loading}>
          {loading ? "Loading…" : "Load more"}
        </button>
      ) : null}
      {scanLimited ? (
        <p className="inspector-empty-line" role="status">
          Directory scan stopped at the server safety limit.
        </p>
      ) : null}
      {content ? (
        <div className="evidence-preview">
          <div className="evidence-preview__heading">
            <div>
              <strong>{content.path}</strong>
              <span>{content.mime} · {formatBytes(content.size)}{content.truncated ? " · truncated" : ""}</span>
            </div>
            <a
              className="ghost icon-button"
              href={client.workspaceFileDownloadUrl(workspaceId, content.path)}
              download
              aria-label={`Download ${content.path}`}
              title={`Download ${content.path}`}
            >
              <DownloadIcon />
            </a>
          </div>
          {content.validation_error ? (
            <p className="inspector-empty-line" role="alert">{content.validation_error}</p>
          ) : null}
          {content.text !== undefined ? (
            <pre className="evidence-preview__text">{content.text}</pre>
          ) : null}
          {content.image && content.preview_allowed ? (
            <figure className="evidence-preview__image">
              <img
                src={client.workspaceFilePreviewUrl(workspaceId, content.path)}
                alt={content.path}
              />
              <figcaption>
                <ImageIcon aria-hidden="true" /> {content.image.width} × {content.image.height} {content.image.format}
              </figcaption>
            </figure>
          ) : null}
          {content.text === undefined && !content.image && !content.validation_error ? (
            <p className="inspector-empty-line">Preview unavailable for this file type.</p>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}
