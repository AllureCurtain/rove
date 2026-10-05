"use client";

import parseDiff from "parse-diff";

const MAX_DIFF_CHARACTERS = 200_000;
const MAX_DIFF_LINES = 2_000;

const UNIFIED_DIFF_HEADER = /^(?:---\s|\+\+\+\s|@@\s|diff\s|index\s)/mu;

export function DiffView({
  diff,
  label = "Unified diff",
  sourcePath,
}: {
  diff: string;
  label?: string;
  /** Canonical mutation path; synthesized headers use it instead of fabricating names. */
  sourcePath?: string;
}) {
  const bounded = diff.slice(0, MAX_DIFF_CHARACTERS);
  const sourceTruncated = bounded.length !== diff.length;
  const hasUnifiedHeader = UNIFIED_DIFF_HEADER.test(bounded);
  const synthesized = synthesizeUnifiedDiff(bounded, hasUnifiedHeader, sourcePath);
  const fallbackPath = sourcePath ?? "Canonical mutation";
  let files: ReturnType<typeof parseDiff>;

  try {
    files = parseDiff(synthesized ?? bounded);
  } catch {
    return <DiffFallback diff={bounded} label={label} reason="Diff could not be parsed" />;
  }

  if (files.length === 0) {
    return <DiffFallback diff={bounded} label={label} reason="Unstructured diff output" />;
  }

  let renderedLines = 0;
  let lineLimitReached = false;

  return (
    <figure className="diff-view" aria-label={label}>
      {files.map((file, fileIndex) => (
        <section className="diff-file" key={`${file.from ?? "new"}-${file.to ?? fileIndex}`}>
          <header>
            <strong>{displayPath(file.to ?? file.from, fallbackPath)}</strong>
            <span>
              +{file.additions} / -{file.deletions}
            </span>
          </header>
          {file.chunks.map((chunk, chunkIndex) => (
            <div className="diff-hunk" key={`${chunk.content}-${chunkIndex}`}>
              <div className="diff-hunk__header">{chunk.content}</div>
              {chunk.changes.map((change, changeIndex) => {
                if (renderedLines >= MAX_DIFF_LINES) {
                  lineLimitReached = true;
                  return null;
                }
                renderedLines += 1;
                return (
                  <div
                    className="diff-line"
                    data-kind={change.type}
                    key={`${change.type}-${changeIndex}-${change.content}`}
                  >
                    <span>{change.type === "del" ? change.ln : change.type === "normal" ? change.ln1 : ""}</span>
                    <span>{change.type === "add" ? change.ln : change.type === "normal" ? change.ln2 : ""}</span>
                    <code>{change.content}</code>
                  </div>
                );
              })}
            </div>
          ))}
        </section>
      ))}
      {sourceTruncated || lineLimitReached ? (
        <figcaption role="note">
          Diff view truncated at the browser safety limit. Complete large-Diff retrieval requires the assigned control-capabilities contract.
        </figcaption>
      ) : null}
      {synthesized ? (
        <figcaption role="note">
          Headerless canonical diff lines shown with a synthesized header; the file identity comes from the mutation record.
        </figcaption>
      ) : null}
    </figure>
  );
}

function DiffFallback({
  diff,
  label,
  reason,
}: {
  diff: string;
  label: string;
  reason: string;
}) {
  return (
    <figure className="diff-view" aria-label={label} data-fallback="true">
      <figcaption>{reason}</figcaption>
      <pre tabIndex={0}>{diff}</pre>
    </figure>
  );
}

function displayPath(path: string | undefined, fallback: string): string {
  if (!path || path === "/dev/null") {
    return fallback;
  }
  return path.replace(/^[ab]\//u, "");
}

/**
 * Canonical mutation Diffs can be headerless `+`/`-`/space lines. Synthesize
 * an honest unified envelope from the real counts so the structural renderer
 * can show them; the exact line counts become the hunk header. Returns null
 * when synthesis is impossible or unnecessary.
 */
function synthesizeUnifiedDiff(
  diff: string,
  hasUnifiedHeader: boolean,
  sourcePath: string | undefined,
): string | null {
  if (hasUnifiedHeader) {
    return null;
  }
  const trimmedPath = sourcePath?.trim();
  if (!trimmedPath) {
    return null;
  }
  const lines = diff.split("\n");
  const lastEmpty = lines.at(-1) === "";
  const contentLines = lastEmpty ? lines.slice(0, -1) : lines;
  if (contentLines.length === 0 || contentLines.length > MAX_DIFF_LINES) {
    return null;
  }
  if (!contentLines.every((line) => /^\s*$|^[\s+-]/u.test(line))) {
    return null;
  }
  const additions = contentLines.filter((line) => line.startsWith("+")).length;
  const deletions = contentLines.filter((line) => line.startsWith("-")).length;
  if (additions === 0 && deletions === 0) {
    return null;
  }
  const path = /^[ab]\//u.test(trimmedPath) ? trimmedPath : `a/${trimmedPath}`;
  const hunk = `@@ -1,${deletions} +1,${additions} @@`;
  const body = lastEmpty ? `${contentLines.join("\n")}\n` : contentLines.join("\n");
  return `--- ${path}\n+++ ${path}\n${hunk}\n${body}`;
}
