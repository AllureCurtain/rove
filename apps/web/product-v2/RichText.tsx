"use client";

import dynamic from "next/dynamic";
import { useState, type ComponentPropsWithoutRef } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";

import { DiffView } from "./DiffView";
import {
  desktopExternalLinkOpenerAvailable,
  openDesktopExternalLink,
} from "../platform/desktop-commands";
import { useCopy } from "../copy/CopyProvider";
import { segmentMarkdown } from "../chat/streaming-blocks";

const RichCodeBlock = dynamic(() => import("./RichCodeBlock"), {
  loading: () => <div className="rich-render-loading" role="status">Loading code renderer…</div>,
});
const MermaidDiagram = dynamic(() => import("./MermaidDiagram"), {
  loading: () => <div className="rich-render-loading" role="status">Loading diagram renderer…</div>,
});

const MAX_MARKDOWN_CHARACTERS = 300_000;

export function RichText({ content }: { content: string }) {
  const bounded = content.slice(0, MAX_MARKDOWN_CHARACTERS);
  // An unclosed fence stays plain text (inside segmentMarkdown's prose), so it
  // cannot swallow the prose that follows it — including the tail of a message
  // that was cut off mid-code-block.
  const truncated = bounded.length !== content.length;

  return (
    <div className="rich-text">
      {segmentMarkdown(bounded).map((segment, index) => (
        <RichTextMarkdown key={index}>{segment.text}</RichTextMarkdown>
      ))}
      {truncated ? (
        <p className="rich-text__limit" role="note">
          Message rendering stopped at the browser safety limit.
        </p>
      ) : null}
    </div>
  );
}

function RichTextMarkdown({ children }: { children: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      urlTransform={safeRichTextUrl}
      components={{ a: SafeLink, code: MarkdownCode, img: BlockedImage }}
    >
      {children}
    </ReactMarkdown>
  );
}

function MarkdownCode({ className, children }: ComponentPropsWithoutRef<"code">) {
  const code = String(children).replace(/\n$/u, "");
  const language = /language-([^\s]+)/u.exec(className ?? "")?.[1];
  if (!language && !code.includes("\n")) {
    return <code>{code}</code>;
  }
  if (language === "mermaid") {
    return <MermaidDiagram source={code} />;
  }
  if (language === "diff" || language === "patch") {
    return <DiffView diff={code} label="Markdown diff" />;
  }
  return <RichCodeBlock code={code} language={language ?? "text"} />;
}

function SafeLink({ href, children, ...props }: ComponentPropsWithoutRef<"a">) {
  const { t } = useCopy();
  const [status, setStatus] = useState<string | null>(null);
  // react-markdown passes the parsed MDAST node alongside the anchor props;
  // spreading it onto the DOM element would emit an unknown `node` attribute.
  const { node: _node, ...anchorProps } = props as typeof props & {
    node?: unknown;
  };
  const safeHref = href ? safeRichTextUrl(href) : "";
  if (!safeHref) {
    return <span className="rich-text__blocked-link">{children}</span>;
  }
  const external = /^https?:/iu.test(safeHref);
  return (
    <>
      <a
        {...anchorProps}
        href={safeHref}
        target={external ? "_blank" : undefined}
        rel={external ? "noreferrer noopener" : undefined}
        onClick={
          external
            ? (event) => {
                // In the packaged app an in-WebView navigation is the wrong
                // outcome: hand the URL to the controlled Desktop host so it
                // reaches the system browser. In a plain browser the default
                // anchor behavior is already correct, so leave it alone.
                if (!desktopExternalLinkOpenerAvailable()) {
                  return;
                }
                event.preventDefault();
                void openDesktopExternalLink(safeHref).then((outcome) => {
                  setStatus(
                    outcome.status === "opened"
                      ? null
                      : t(
                          outcome.status === "unsupported"
                            ? "richText.linkOpenUnsupported"
                            : outcome.status === "blocked"
                              ? "richText.linkOpenBlocked"
                              : "richText.linkOpenFailed",
                        ),
                  );
                });
              }
            : undefined
        }
      >
        {children}
      </a>
      {status ? (
        <span className="rich-text__link-status" role="status">
          {status}
        </span>
      ) : null}
    </>
  );
}

function BlockedImage({ alt, title }: ComponentPropsWithoutRef<"img">) {
  return (
    <span className="blocked-image" role="note" title={title}>
      <strong>Image unavailable</strong>
      <span>{alt || "The current API does not expose a safe image resource."}</span>
    </span>
  );
}

export function safeRichTextUrl(url: string): string {
  const trimmed = url.trim();
  if (trimmed.startsWith("#")) {
    return trimmed;
  }
  if (/^\/(?![\\/])/u.test(trimmed) && !trimmed.includes("\\")) {
    return trimmed;
  }
  if (/^(?:https?:|mailto:)/iu.test(trimmed)) {
    return defaultUrlTransform(trimmed);
  }
  return "";
}
