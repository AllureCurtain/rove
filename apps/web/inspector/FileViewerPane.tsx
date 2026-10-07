"use client";

import {
  DownloadIcon,
  ExternalLinkIcon,
  FileTextIcon,
  ImageIcon,
} from "@radix-ui/react-icons";
import { Highlight } from "prism-react-renderer";
import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type {
  ProductFileContentEnvelope,
  ProductPreviewSession,
} from "../product/product-api-types";
import {
  desktopRevealInFolderAvailable,
  revealDesktopPath,
} from "../platform/desktop-commands";
import { InspectorEmpty } from "./InspectorEmpty";
import { RichText } from "../product-v2/RichText";
import { normalizeLanguage, STEEL_THEME } from "../product-v2/RichCodeBlock";

/** Design §8: the viewer keeps a very large file bounded (~5000 lines). */
const FILE_VIEWER_LINE_CAP = 5000;

function isHtmlPath(path: string): boolean {
  return /\.(html|htm)$/i.test(path);
}

function isMarkdownPath(path: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(path);
}

/** Prism language from a file extension; unknown extensions read as text. */
function languageForPath(path: string, mime: string): string {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  const byExtension: Record<string, string> = {
    js: "javascript",
    mjs: "javascript",
    cjs: "javascript",
    ts: "typescript",
    mts: "typescript",
    cts: "typescript",
    jsx: "jsx",
    tsx: "tsx",
    json: "json",
    rs: "rust",
    py: "python",
    css: "css",
    scss: "scss",
    less: "less",
    html: "markup",
    htm: "markup",
    xml: "markup",
    svg: "markup",
    md: "markdown",
    markdown: "markdown",
    sh: "bash",
    bash: "bash",
    zsh: "bash",
    toml: "toml",
    yaml: "yaml",
    yml: "yaml",
    sql: "sql",
    go: "go",
    java: "java",
    c: "c",
    h: "c",
    cpp: "cpp",
    cs: "csharp",
    rb: "ruby",
    php: "php",
    diff: "diff",
    patch: "diff",
  };
  const mapped = byExtension[extension];
  if (mapped) {
    return normalizeLanguage(mapped);
  }
  if (mime.startsWith("image/")) {
    return "text";
  }
  return normalizeLanguage(mime.includes("javascript") ? "javascript" : extension);
}

function joinWorkspacePath(rootPath: string, path: string): string {
  const root = rootPath.replace(/[\\/]+$/, "");
  // Windows paths accept a forward slash; the desktop host canonicalizes.
  return `${root}/${path}`;
}

/**
 * One file's viewer pane (design §8): a `file` tab mounts this with the
 * workspace-relative path it owns. Text renders syntax-highlighted with a
 * bounded line cap, Markdown renders as Markdown, and "open externally"
 * reveals the file in the OS on Desktop or opens a blob tab in the browser.
 */
export function FileViewerPane({
  workspaceId,
  workspaceRootPath,
  path,
  focusLine,
}: {
  workspaceId: string;
  workspaceRootPath?: string | null;
  path: string;
  focusLine?: number | null;
}) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [content, setContent] = useState<ProductFileContentEnvelope | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [htmlPreview, setHtmlPreview] = useState<ProductPreviewSession | null>(null);
  const [htmlPreviewBusy, setHtmlPreviewBusy] = useState(false);
  const [htmlPreviewError, setHtmlPreviewError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sourceView, setSourceView] = useState(false);
  const requestRef = useRef(0);
  const previewRequestRef = useRef(0);
  const focusedLineRef = useRef<HTMLSpanElement>(null);
  const downloadRequestRef = useRef(0);

  useEffect(() => () => {
    requestRef.current += 1;
    previewRequestRef.current += 1;
    downloadRequestRef.current += 1;
  }, [workspaceId, path]);

  useEffect(() => {
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  }, [previewUrl]);

  // Closing the tab (or switching workspace) revokes the isolated preview
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

  useEffect(() => {
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    async function load() {
      setLoading(true);
      setError(null);
      setContent(null);
      setPreviewUrl(null);
      setSourceView(false);
      try {
        const nextContent = await client.getWorkspaceFileContent(workspaceId, path);
        const nextPreviewUrl =
          nextContent.image && nextContent.preview_allowed
            ? URL.createObjectURL(
                await client.fetchWorkspaceFilePreview(workspaceId, path),
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
      } finally {
        if (!stale()) {
          setLoading(false);
        }
      }
    }
    void load();
    return () => { requestRef.current += 1; };
  }, [client, workspaceId, path]);

  // A finding deep-link carries a line: scroll to it once the text renders.
  useEffect(() => {
    if (content && focusLine) {
      focusedLineRef.current?.scrollIntoView({ block: "center" });
    }
  }, [content, focusLine]);

  async function openHtmlPreview() {
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

  /**
   * "Open externally" (design §8): Desktop reveals the file in the OS file
   * manager inside the workspace root; the plain browser build has no host
   * command, so it opens the fetched content as a same-origin blob tab.
   */
  async function openExternally() {
    setError(null);
    if (desktopRevealInFolderAvailable() && workspaceRootPath) {
      try {
        await revealDesktopPath(joinWorkspacePath(workspaceRootPath, path));
      } catch (caught) {
        setError(caught instanceof Error ? caught.message : "Failed to reveal file");
      }
      return;
    }
    try {
      const url =
        previewUrl ??
        URL.createObjectURL(
          new Blob([content?.text ?? ""], {
            type: content?.mime || "text/plain",
          }),
        );
      const opened = window.open(url, "_blank", "noopener,noreferrer");
      if (!opened && url !== previewUrl) {
        URL.revokeObjectURL(url);
      }
      if (url !== previewUrl) {
        window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to open file");
    }
  }

  async function downloadFile() {
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

  const fileName = path.split("/").filter(Boolean).pop() ?? path;
  const textLines = useMemo(
    () => (content?.text != null ? content.text.split("\n") : null),
    [content],
  );
  const cappedLines = textLines ? textLines.slice(0, FILE_VIEWER_LINE_CAP) : null;
  const lineCapReached = textLines !== null && textLines.length > FILE_VIEWER_LINE_CAP;

  return (
    <section className="inspector-section file-viewer" aria-label={fileName}>
      {loading && !content ? (
        <p className="inspector-empty-line">{t("inspector.filesLoading")}</p>
      ) : null}
      {error ? (
        <p className="inspector-empty-line" role="alert">{t("chrome.loadError")}</p>
      ) : null}
      {!loading && !error && !content ? (
        <InspectorEmpty
          title={t("inspector.fileUnavailable")}
          body={t("inspector.fileUnavailableBody")}
        />
      ) : null}
      {content ? (
        <div className="evidence-preview">
          <div className="evidence-preview__heading">
            <div>
              <strong>{content.path}</strong>
              <span>
                {content.mime} · {formatBytes(content.size)}
                {content.truncated ? ` · ${t("chrome.truncated")}` : ""}
                {lineCapReached
                  ? ` · ${t("inspector.fileLineCap", {
                      shown: FILE_VIEWER_LINE_CAP,
                      total: textLines.length,
                    })}`
                  : ""}
              </span>
            </div>
            {isMarkdownPath(content.path) && content.text !== undefined ? (
              <button
                type="button"
                className="ghost icon-button"
                onClick={() => setSourceView((value) => !value)}
                aria-pressed={sourceView}
                aria-label={sourceView ? t("inspector.fileRendered") : t("inspector.fileSource")}
                title={sourceView ? t("inspector.fileRendered") : t("inspector.fileSource")}
              >
                <FileTextIcon />
              </button>
            ) : null}
            <button
              type="button"
              className="ghost icon-button"
              onClick={() => void openExternally()}
              aria-label={t("inspector.fileOpenExternal")}
              title={t("inspector.fileOpenExternal")}
            >
              <ExternalLinkIcon />
            </button>
            <button
              type="button"
              className="ghost icon-button"
              onClick={() => void downloadFile()}
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
                  onClick={() => void openHtmlPreview()}
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
          {content.text !== undefined &&
          content.text != null &&
          isMarkdownPath(content.path) &&
          !sourceView ? (
            <div className="evidence-preview__markdown">
              <RichText content={content.text} />
            </div>
          ) : null}
          {content.text !== undefined &&
          (!isMarkdownPath(content.path) || sourceView) &&
          cappedLines ? (
            <Highlight
              theme={STEEL_THEME}
              code={cappedLines.join("\n")}
              language={languageForPath(content.path, content.mime)}
            >
              {({ className, tokens, getLineProps, getTokenProps }) => (
                <pre
                  className={`${className} evidence-preview__text`}
                  data-focus-line={focusLine ?? undefined}
                  tabIndex={0}
                >
                  {tokens.map((line, lineIndex) => {
                    const lineNumber = lineIndex + 1;
                    const focused = focusLine === lineNumber;
                    return (
                      <span
                        {...getLineProps({ line })}
                        key={`${content.path}-${lineIndex}`}
                        ref={focused ? focusedLineRef : undefined}
                        data-line={lineNumber}
                        data-focused={focused ? "true" : undefined}
                      >
                        <span className="evidence-preview__line-number" aria-hidden="true">
                          {lineNumber}
                        </span>
                        {line.map((token, tokenIndex) => (
                          <span {...getTokenProps({ token })} key={tokenIndex} />
                        ))}
                        {lineIndex < tokens.length - 1 ? "\n" : ""}
                      </span>
                    );
                  })}
                </pre>
              )}
            </Highlight>
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
            <InspectorEmpty
              title={t("chrome.previewUnavailable")}
              body={t("inspector.filePreviewUnavailableBody")}
            />
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
