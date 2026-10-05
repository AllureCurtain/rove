"use client";

import {
  ArchiveIcon,
  DownloadIcon,
  FileIcon,
} from "@radix-ui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type { ProductFileEntry } from "../product/product-api-types";
import { InspectorEmpty } from "./InspectorEmpty";

/**
 * The directory browser tab (design §8): breadcrumb navigation plus a paged
 * entry list. Opening a file delegates to the host, which spawns that file's
 * own viewer tab — every opened file is its own tab, so the browser never
 * loses its place while files are inspected.
 */
export function FilesPanel({
  workspaceId,
  onOpenFile,
}: {
  workspaceId: string;
  onOpenFile?: (path: string) => void;
}) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [prefix, setPrefix] = useState("");
  const [entries, setEntries] = useState<ProductFileEntry[]>([]);
  // Listing and download are independent: starting a download must not discard
  // a page in flight.
  const requestRef = useRef(0);
  const downloadRequestRef = useRef(0);
  useEffect(() => () => {
    requestRef.current += 1;
    downloadRequestRef.current += 1;
  }, [workspaceId]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [scanLimited, setScanLimited] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    async function load() {
      setLoading(true);
      setError(null);
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

  async function openFile(entry: ProductFileEntry) {
    if (entry.kind === "directory") {
      setPrefix(entry.path);
      return;
    }
    onOpenFile?.(entry.path);
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

  // Breadcrumb segments: every ancestor prefix is a button back up the tree.
  const breadcrumbs = useMemo(() => {
    const segments = prefix.split("/").filter(Boolean);
    const crumbs: { label: string; path: string }[] = [];
    segments.forEach((segment, index) => {
      crumbs.push({ label: segment, path: segments.slice(0, index + 1).join("/") });
    });
    return crumbs;
  }, [prefix]);

  return (
    <section className="inspector-section" aria-label={t("inspector.files")}>
      <nav
        className="evidence-breadcrumbs"
        aria-label={t("inspector.filesBreadcrumbs")}
      >
        <button
          type="button"
          className="ghost evidence-breadcrumbs__crumb"
          onClick={() => setPrefix("")}
          aria-current={prefix === "" ? "location" : undefined}
        >
          {t("inspector.filesRoot")}
        </button>
        {breadcrumbs.map((crumb, index) => (
          <span key={crumb.path} className="evidence-breadcrumbs__segment">
            <span className="evidence-breadcrumbs__sep" aria-hidden="true">/</span>
            <button
              type="button"
              className="ghost evidence-breadcrumbs__crumb"
              onClick={() => setPrefix(crumb.path)}
              aria-current={index === breadcrumbs.length - 1 ? "location" : undefined}
            >
              {crumb.label}
            </button>
          </span>
        ))}
      </nav>
      {loading && entries.length === 0 ? (
        <p className="inspector-empty-line">{t("inspector.filesLoading")}</p>
      ) : null}
      {error ? <p className="inspector-empty-line" role="alert">{t("chrome.loadError")}</p> : null}
      {!loading && !error && entries.length === 0 ? (
        <InspectorEmpty
          title={t("inspector.filesEmpty")}
          body={t("inspector.filesEmptyBody")}
        />
      ) : null}
      <ul className="evidence-file-list">
        {entries.map((entry) => (
          <li key={entry.path}>
            <button
              type="button"
              className="ghost evidence-file-list__open"
              onClick={() => void openFile(entry)}
            >
              {entry.kind === "directory" ? (
                <ArchiveIcon aria-hidden="true" />
              ) : (
                <FileIcon aria-hidden="true" />
              )}
              <span>{entry.path.slice(prefix ? prefix.length + 1 : 0) || entry.path}</span>
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
    </section>
  );

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
