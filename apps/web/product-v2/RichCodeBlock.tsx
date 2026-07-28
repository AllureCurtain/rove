"use client";

import { CheckIcon, CopyIcon, Cross2Icon } from "@radix-ui/react-icons";
import { Highlight, type PrismTheme } from "prism-react-renderer";
import { useEffect, useRef, useState } from "react";

const STEEL_THEME: PrismTheme = {
  plain: { color: "var(--text)", backgroundColor: "transparent" },
  styles: [
    { types: ["comment", "prolog", "doctype", "cdata"], style: { color: "var(--muted)" } },
    { types: ["punctuation"], style: { color: "var(--text-secondary)" } },
    { types: ["property", "tag", "constant", "symbol", "deleted"], style: { color: "var(--error)" } },
    { types: ["boolean", "number"], style: { color: "var(--warning)" } },
    { types: ["selector", "attr-name", "string", "char", "builtin", "inserted"], style: { color: "var(--success)" } },
    { types: ["operator", "entity", "url", "variable"], style: { color: "var(--text-secondary)" } },
    { types: ["atrule", "attr-value", "function", "class-name"], style: { color: "var(--accent)" } },
    { types: ["keyword"], style: { color: "var(--accent-strong)" } },
    { types: ["regex", "important"], style: { color: "var(--warning)" } },
  ],
};

export default function RichCodeBlock({
  code,
  language,
}: {
  code: string;
  language: string;
}) {
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">("idle");
  const copyResetRef = useRef<number | null>(null);
  const normalizedLanguage = normalizeLanguage(language);

  useEffect(
    () => () => {
      if (copyResetRef.current !== null) {
        window.clearTimeout(copyResetRef.current);
      }
    },
    [],
  );

  async function copyCode() {
    try {
      await navigator.clipboard.writeText(code);
      setCopyState("copied");
      if (copyResetRef.current !== null) {
        window.clearTimeout(copyResetRef.current);
      }
      copyResetRef.current = window.setTimeout(() => {
        setCopyState("idle");
        copyResetRef.current = null;
      }, 2_000);
    } catch {
      setCopyState("error");
    }
  }

  return (
    <figure className="rich-code" data-language={normalizedLanguage}>
      <figcaption>
        <span>{normalizedLanguage}</span>
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => void copyCode()}
          aria-label={copyState === "copied" ? "Code copied" : "Copy code"}
          title={copyState === "error" ? "Copy failed" : "Copy code"}
        >
          {copyState === "copied" ? (
            <CheckIcon />
          ) : copyState === "error" ? (
            <Cross2Icon />
          ) : (
            <CopyIcon />
          )}
        </button>
        <span className="rich-code__copy-state" role="status" aria-live="polite">
          {copyState === "copied" ? "Copied" : copyState === "error" ? "Copy failed" : null}
        </span>
      </figcaption>
      <Highlight theme={STEEL_THEME} code={code} language={normalizedLanguage}>
        {({ className, tokens, getLineProps, getTokenProps }) => (
          <pre className={className} tabIndex={0}>
            {tokens.map((line, lineIndex) => (
              <div {...getLineProps({ line })} key={lineIndex}>
                <span className="rich-code__line-number" aria-hidden="true">
                  {lineIndex + 1}
                </span>
                <span>
                  {line.map((token, tokenIndex) => (
                    <span {...getTokenProps({ token })} key={tokenIndex} />
                  ))}
                </span>
              </div>
            ))}
          </pre>
        )}
      </Highlight>
    </figure>
  );
}

function normalizeLanguage(language: string): string {
  const normalized = language.trim().toLowerCase();
  const aliases: Record<string, string> = {
    csharp: "csharp",
    cs: "csharp",
    html: "markup",
    js: "javascript",
    jsx: "jsx",
    md: "markdown",
    py: "python",
    rs: "rust",
    sh: "bash",
    shell: "bash",
    ts: "typescript",
    tsx: "tsx",
    yml: "yaml",
  };
  return (aliases[normalized] ?? normalized) || "text";
}
