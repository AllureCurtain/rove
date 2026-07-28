"use client";

import { useCallback, useEffect, useId, useRef, useState } from "react";

const MAX_MERMAID_CHARACTERS = 20_000;

export const MERMAID_TEXT_LABEL_CONFIG = {
  htmlLabels: false,
  flowchart: { htmlLabels: false },
} as const;

export default function MermaidDiagram({ source }: { source: string }) {
  const reactId = useId();
  const hostRef = useRef<HTMLElement | null>(null);
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [state, setState] = useState<
    | { status: "loading" }
    | { status: "ready"; svg: string }
    | { status: "error"; error: string }
  >({ status: "loading" });
  const bindHost = useCallback((node: HTMLElement | null) => {
    hostRef.current = node;
  }, []);

  useEffect(() => {
    const root = document.documentElement;
    const syncTheme = () => setTheme(root.dataset.theme === "dark" ? "dark" : "light");
    syncTheme();
    const observer = new MutationObserver(syncTheme);
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let active = true;
    if (source.length > MAX_MERMAID_CHARACTERS) {
      setState({
        status: "error",
        error: "Diagram source exceeds the browser rendering limit.",
      });
      return () => {
        active = false;
      };
    }

    const renderId = `rove-mermaid-${reactId.replace(/[^A-Za-z0-9_-]/gu, "")}`;
    void import("mermaid")
      .then(async ({ default: mermaid }) => {
        const themeRoot = hostRef.current?.closest<HTMLElement>(".product-app-frame");
        if (!themeRoot) {
          throw new Error("Product theme tokens are unavailable.");
        }
        const styles = getComputedStyle(themeRoot);
        const themeVariables = {
          background: readThemeToken(styles, "--v2-surface-raised", "--surface-strong"),
          primaryColor: readThemeToken(styles, "--v2-surface-raised", "--surface-strong"),
          primaryTextColor: readThemeToken(styles, "--v2-ink", "--text"),
          primaryBorderColor: readThemeToken(styles, "--v2-signal-strong", "--accent-strong"),
          lineColor: readThemeToken(styles, "--v2-ink-3", "--muted"),
          secondaryColor: readThemeToken(styles, "--v2-surface", "--surface"),
          tertiaryColor: readThemeToken(styles, "--v2-surface-sunken", "--surface-soft"),
        };
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          suppressErrorRendering: true,
          theme: "base",
          ...MERMAID_TEXT_LABEL_CONFIG,
          themeVariables,
        });
        const rendered = await mermaid.render(renderId, source);
        if (active) {
          setState({ status: "ready", svg: sanitizeMermaidSvg(rendered.svg) });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setState({
            status: "error",
            error: error instanceof Error ? error.message : "Diagram could not be rendered.",
          });
        }
      });

    return () => {
      active = false;
    };
  }, [reactId, source, theme]);

  if (state.status === "loading") {
    return <div ref={bindHost} className="mermaid-state" role="status">Rendering diagram…</div>;
  }
  if (state.status === "error") {
    return (
      <div ref={bindHost} className="mermaid-state" data-tone="error" role="note">
        <strong>Diagram unavailable</strong>
        <span>{state.error}</span>
        <pre>{source.slice(0, MAX_MERMAID_CHARACTERS)}</pre>
      </div>
    );
  }
  return (
    <figure
      ref={bindHost}
      className="mermaid-diagram"
      aria-label="Mermaid diagram"
      dangerouslySetInnerHTML={{ __html: state.svg }}
    />
  );
}

export function sanitizeMermaidSvg(svg: string): string {
  const document = new DOMParser().parseFromString(svg, "image/svg+xml");
  document
    .querySelectorAll("script, foreignObject, image, iframe, object, embed, a")
    .forEach((node) => node.remove());
  document.querySelectorAll("style").forEach((node) => {
    const css = node.textContent ?? "";
    if (/@import\b/iu.test(css) || hasUnsafeMermaidCssUrl(css)) {
      node.remove();
    }
  });
  for (const element of document.querySelectorAll("*")) {
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.trim().toLowerCase();
      if (
        name.startsWith("on") ||
        name === "href" ||
        name === "xlink:href" ||
        value.includes("javascript:") ||
        hasUnsafeMermaidCssUrl(value)
      ) {
        element.removeAttribute(attribute.name);
      }
    }
  }
  return new XMLSerializer().serializeToString(document.documentElement);
}

export function hasUnsafeMermaidCssUrl(value: string): boolean {
  const cssUrl = /url\s*\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)/giu;
  for (const match of value.matchAll(cssUrl)) {
    const target = (match[1] ?? match[2] ?? match[3] ?? "").trim();
    if (!target.startsWith("#")) {
      return true;
    }
  }
  return false;
}

function readThemeToken(
  styles: CSSStyleDeclaration,
  primary: string,
  fallback: string,
): string {
  const value = styles.getPropertyValue(primary).trim()
    || styles.getPropertyValue(fallback).trim();
  if (!value) {
    throw new Error(`Product theme token ${primary} is unavailable.`);
  }
  return value;
}
