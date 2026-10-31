"use client";

import {
  CheckIcon,
  ChevronDownIcon,
  CodeIcon,
  CopyIcon,
} from "@radix-ui/react-icons";
import { useEffect, useRef, useState, type ReactNode } from "react";

import type { ToolCallView } from "../lib/rove-state";
import { useCopy } from "../copy/CopyProvider";
import { DiffView } from "../product-v2/DiffView";
import { RichText } from "../product-v2/RichText";
import { segmentFileReferences, type FileReference } from "./file-ref";
import { buildToolPresentation, type ToolBlock } from "./tool-presentation";
import { durationBadgeLabel } from "./tool-timing";

/**
 * One tool call as a compact, expandable card — the transcript's tool-row
 * language. Shared by the main transcript and the work-panel subagent mini
 * transcript so a delegate's own calls render exactly like the parent's.
 */
export function ToolCard({
  tool,
  onOpenFile,
  footer,
}: {
  tool: ToolCallView;
  onOpenFile?: (reference: FileReference) => void;
  /**
   * Content attached inside the card's frame below the head/details — the
   * subagent topology a `task` call fans out (issue #31). It renders whether
   * the card is open or folded so the delegation stays reachable either way.
   */
  footer?: ReactNode;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(
    tool.status === "running" || tool.status === "error" || Boolean(tool.mutations?.length),
  );
  const [copied, setCopied] = useState(false);
  const copyResetRef = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (copyResetRef.current !== null) {
        window.clearTimeout(copyResetRef.current);
      }
    },
    [],
  );
  const detailId = `tool-detail-${tool.timelineId ?? tool.id}`.replace(/[^A-Za-z0-9_-]/gu, "-");
  // Body blocks are built only while open, so a collapsed card stays cheap.
  const presentation = buildToolPresentation(tool, open);
  // The runtime's own measurement of an MCP call. A live arrival span is not a
  // substitute here: it also covers queueing on this client.
  const durationBadge = durationBadgeLabel(tool.protocolMetadata?.duration_ms);
  const copyPayload = tool.output ?? tool.details;

  async function copyToolPayload() {
    try {
      await navigator.clipboard.writeText(copyPayload);
      setCopied(true);
      if (copyResetRef.current !== null) {
        window.clearTimeout(copyResetRef.current);
      }
      copyResetRef.current = window.setTimeout(() => {
        setCopied(false);
        copyResetRef.current = null;
      }, 1_500);
    } catch {
      setCopied(false);
    }
  }

  return (
    <article className="tool-card" data-status={tool.status}>
      {/* One compact line: icon, the named object, a mono summary, the state
          (dot + text, spinner while running), duration, caret. The payload
          copy rides the row and only surfaces on hover/focus. */}
      <div className="tool-card__row">
        <button
          type="button"
          className="tool-card__head"
          aria-controls={detailId}
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          <CodeIcon />
          <strong>{presentation.title}</strong>
          {presentation.subtitle.text ? (
            presentation.subtitle.fileRef && onOpenFile ? (
              // A resolvable path summary is the row's file affordance: the
              // whole head still toggles, but this span takes the click first
              // (issue #27).
              <code
                className="tool-card__summary tool-card__summary--link"
                role="button"
                tabIndex={0}
                title={t("chat.openFileRef")}
                onClick={(event) => {
                  event.stopPropagation();
                  onOpenFile(presentation.subtitle.fileRef!);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.stopPropagation();
                    event.preventDefault();
                    onOpenFile(presentation.subtitle.fileRef!);
                  }
                }}
              >
                {presentation.subtitle.text}
              </code>
            ) : (
              <code className="tool-card__summary">{presentation.subtitle.text}</code>
            )
          ) : null}
          <span className="tool-card__state" data-status={tool.status}>
            {tool.status === "running" ? (
              <span className="tool-card__spinner" aria-hidden="true" />
            ) : (
              <i className="tool-card__state-dot" aria-hidden="true" />
            )}
            <span>{toolStatusLabel(tool.status, t)}</span>
          </span>
          {durationBadge ? (
            <span className="tool-card__duration">{durationBadge}</span>
          ) : null}
          <ChevronDownIcon data-open={open} />
        </button>
        <button
          type="button"
          className="tool-card__copy icon-button"
          aria-label={copied ? t("chat.messageCopied") : t("chat.toolCopy")}
          title={copied ? t("chat.messageCopied") : t("chat.toolCopy")}
          onClick={() => void copyToolPayload()}
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
        </button>
      </div>
      {open ? (
        <div className="tool-card__details" id={detailId}>
          {presentation.chips.length > 0 ? (
            <div className="tool-card__chips">
              {presentation.chips.map((chip) => (
                <span className="tool-card__chip" data-tone={chip.tone} key={chip.label}>
                  {toolChipLabel(chip.label, t)}
                </span>
              ))}
            </div>
          ) : null}
          {presentation.blocks.map((block, index) => (
            <ToolBlockView block={block} key={index} onOpenFile={onOpenFile} />
          ))}
        </div>
      ) : null}
      {footer}
    </article>
  );
}

/** The one-word state next to the row's dot: data, but the words are ours. */
function toolStatusLabel(
  status: ToolCallView["status"],
  t: (path: string) => string,
): string {
  switch (status) {
    case "running":
      return t("chat.toolStateRunning");
    case "waiting":
      return t("chat.toolStateWaiting");
    case "error":
      return t("chat.toolStateError");
    default:
      return t("chat.toolStateDone");
  }
}

/**
 * The presentation model uses stable label vocabulary for chips; only the
 * fixed vocabulary is translated here. Dynamic values (tool status, error
 * codes) are data, not copy, and render as-is.
 */
function toolChipLabel(label: string, t: (path: string) => string): string {
  switch (label) {
    case "read only":
      return t("chat.chipReadOnly");
    case "mutation capable":
      return t("chat.chipMutation");
    case "high risk":
      return t("chat.chipHighRisk");
    case "workspace changed":
      return t("chat.chipWorkspaceChanged");
    default:
      return label;
  }
}

/**
 * Fact labels are mostly data (diff summaries). Only the labels the shell
 * itself introduces are translated.
 */
function toolFactLabel(label: string, t: (path: string) => string): string {
  return label === "duration" ? t("chat.toolDuration") : label;
}

function ToolBlockView({
  block,
  onOpenFile,
}: {
  block: ToolBlock;
  onOpenFile?: (reference: FileReference) => void;
}) {
  const { t } = useCopy();
  switch (block.kind) {
    case "args":
      return (
        <section>
          <h4>{t("chat.toolInput")}</h4>
          <pre className="tool-card__args" tabIndex={0}>
            <RefLinkedText text={block.text} onOpenFile={onOpenFile} />
          </pre>
          {block.truncated ? (
            <p className="tool-card__truncated">{t("chat.outputTruncated")}</p>
          ) : null}
        </section>
      );
    case "diff":
      return (
        <section>
          <h4>
            <RefLink
              text={block.path}
              onOpenFile={onOpenFile}
            />
          </h4>
          <DiffView diff={block.text} label={`${block.path} diff`} sourcePath={block.path} />
          {block.truncated ? <p className="tool-card__truncated">{t("chat.diffTruncated")}</p> : null}
        </section>
      );
    case "code":
      return (
        <section>
          <RichText content={"```" + block.language + "\n" + block.text + "\n```"} />
          {block.truncated ? <p className="tool-card__truncated">{t("chat.outputTruncated")}</p> : null}
        </section>
      );
    case "files":
      return (
        <section>
          <h4>{t("chat.filesTitle")}</h4>
          <ul className="tool-card__list">
            {block.entries.map((entry) => (
              <li key={entry.path}>
                <RefLink text={entry.path} onOpenFile={onOpenFile} />
                {entry.annotation ? ` ${entry.annotation}` : null}
              </li>
            ))}
          </ul>
        </section>
      );
    case "matches":
      return (
        <section>
          <h4>{t("chat.matchesTitle")}</h4>
          <ul className="tool-card__list">
            {block.entries.map((entry, index) => (
              <li key={index}>
                <RefLink
                  text={`${entry.path}${entry.line ? `:${entry.line}` : ""}`}
                  onOpenFile={onOpenFile}
                />{" "}
                {entry.text}
              </li>
            ))}
          </ul>
          {block.truncated ? <p className="tool-card__truncated">{t("chat.matchesTruncated")}</p> : null}
        </section>
      );
    case "fields":
      return (
        <section>
          <dl className="tool-facts">
            {block.rows.map((row, index) => (
              <div key={index}><dt>{toolFactLabel(row.label, t)}</dt><dd>{row.value}</dd></div>
            ))}
          </dl>
        </section>
      );
    case "note":
      return (
        <pre tabIndex={0}>
          <RefLinkedText text={block.text} onOpenFile={onOpenFile} />
        </pre>
      );
  }
}

/**
 * A span that reads as a file reference. With an open handler it is a button;
 * without one it is plain text — the affordance degrades, the data never does.
 */
function RefLink({
  text,
  onOpenFile,
}: {
  text: string;
  onOpenFile?: (reference: FileReference) => void;
}) {
  const { t } = useCopy();
  const segments = segmentFileReferences(text);
  if (segments.length === 1 && segments[0]!.kind === "file" && onOpenFile) {
    const reference = segments[0].reference;
    return (
      <button
        type="button"
        className="tool-card__ref"
        title={t("chat.openFileRef")}
        onClick={(event) => {
          event.stopPropagation();
          onOpenFile(reference);
        }}
      >
        {text}
      </button>
    );
  }
  return <code>{text}</code>;
}

/**
 * Mixed prose + references: file spans and URLs render as affordances, the
 * rest as raw text. Segmentation always preserves the source, so joining the
 * parts reproduces the original string.
 */
function RefLinkedText({
  text,
  onOpenFile,
}: {
  text: string;
  onOpenFile?: (reference: FileReference) => void;
}) {
  const { t } = useCopy();
  const segments = segmentFileReferences(text);
  return (
    <>
      {segments.map((segment, index) => {
        if (segment.kind === "file" && onOpenFile) {
          const reference = segment.reference;
          return (
            <button
              type="button"
              className="tool-card__ref"
              key={index}
              title={t("chat.openFileRef")}
              onClick={(event) => {
                event.stopPropagation();
                onOpenFile(reference);
              }}
            >
              {segment.text}
            </button>
          );
        }
        if (segment.kind === "url") {
          return (
            <a
              key={index}
              className="tool-card__url"
              href={segment.url}
              target="_blank"
              rel="noreferrer noopener"
              onClick={(event) => event.stopPropagation()}
            >
              {segment.text}
            </a>
          );
        }
        return <span key={index}>{segment.text}</span>;
      })}
    </>
  );
}

export function formatValue(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
