"use client";

import dynamic from "next/dynamic";
import type { ComponentPropsWithoutRef } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";

import { DiffView } from "./DiffView";

const RichCodeBlock = dynamic(() => import("./RichCodeBlock"), {
  loading: () => <div className="rich-render-loading" role="status">Loading code renderer…</div>,
});
const MermaidDiagram = dynamic(() => import("./MermaidDiagram"), {
  loading: () => <div className="rich-render-loading" role="status">Loading diagram renderer…</div>,
});

const MAX_MARKDOWN_CHARACTERS = 300_000;

export function RichText({ content }: { content: string }) {
  const bounded = content.slice(0, MAX_MARKDOWN_CHARACTERS);
  const truncated = bounded.length !== content.length;

  return (
    <div className="rich-text">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        skipHtml
        urlTransform={safeRichTextUrl}
        components={{
          a: SafeLink,
          code: MarkdownCode,
          img: BlockedImage,
        }}
      >
        {bounded}
      </ReactMarkdown>
      {truncated ? (
        <p className="rich-text__limit" role="note">
          Message rendering stopped at the browser safety limit.
        </p>
      ) : null}
    </div>
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
  const safeHref = href ? safeRichTextUrl(href) : "";
  if (!safeHref) {
    return <span className="rich-text__blocked-link">{children}</span>;
  }
  const external = /^https?:/iu.test(safeHref);
  return (
    <a
      {...props}
      href={safeHref}
      target={external ? "_blank" : undefined}
      rel={external ? "noreferrer noopener" : undefined}
    >
      {children}
    </a>
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
