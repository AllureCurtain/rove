/**
 * Recognition and segmentation of workspace file references and URLs inside
 * tool-call text (args, outputs, summaries). A reference is detected, not
 * trusted: the click handler decides whether it can open it, and the file
 * viewer stays the surface that proves the path exists in the workspace.
 */

/**
 * The portion of a file reference that may carry a workspace path: no
 * whitespace, no quotes/brackets/backticks, no query or fragment. A trailing
 * `:line[:column]` ride along with the match so `src/a.ts:10:4` parses.
 */
const FILE_REF_PATTERN =
  /(?:[A-Za-z]:)?(?:[\w.@~-]+[/\\])+[\w.@~+#-]+(?::\d+(?::\d+)?)?|\.[\w.@~+-]+(?:[/\\][\w.@~+#-]+)+(?::\d+(?::\d+)?)?/gu;

const URL_PATTERN = /https?:\/\/[^\s<>"'`()\[\]{}]+/giu;

export interface FileReference {
  /** The matched text without the `file:` prefix or :line[:column] suffix. */
  path: string;
  line?: number;
  column?: number;
}

export type FileRefSegment =
  | { kind: "text"; text: string }
  | { kind: "file"; text: string; reference: FileReference }
  | { kind: "url"; text: string; url: string };

/**
 * A matched reference is dropped as soon as it cannot resolve to a plausible
 * workspace-relative path: absolute or drive-rooted paths, home-relative
 * paths, and anything that escapes the root all stay plain text.
 */
export function parseFileReference(raw: string): FileReference | null {
  let body = raw.trim();
  if (body.startsWith("file:")) {
    body = body.slice("file:".length);
  }
  if (body.startsWith("//") || body.startsWith("\\\\")) {
    // UNC or scheme-relative authority — never a workspace path.
    return null;
  }
  let line: number | undefined;
  let column: number | undefined;
  const position = /:(\d+)(?::(\d+))?$/.exec(body);
  if (position) {
    line = Number(position[1]);
    column = position[2] !== undefined ? Number(position[2]) : undefined;
    body = body.slice(0, position.index);
  }
  if (body === "" || body.includes("..")) {
    return null;
  }
  const isWindowsAbsolute = /^[A-Za-z]:[\\/]/.test(body);
  if (
    isWindowsAbsolute ||
    body.startsWith("/") ||
    body.startsWith("\\") ||
    body.startsWith("~")
  ) {
    return null;
  }
  const path = body.replace(/\\/g, "/").replace(/^\.\//, "");
  if (path === "" || path.includes("//")) {
    return null;
  }
  return { path, line, column };
}

/**
 * Splits free text into file references, URLs, and the text between them. A
 * span that parses as neither stays text, so rendering is always lossless.
 */
export function segmentFileReferences(text: string): FileRefSegment[] {
  const matches: { start: number; end: number; segment: FileRefSegment }[] = [];
  for (const match of text.matchAll(FILE_REF_PATTERN)) {
    const raw = match[0];
    const start = match.index ?? 0;
    const reference = parseFileReference(raw);
    if (!reference) {
      continue;
    }
    matches.push({
      start,
      end: start + raw.length,
      segment: { kind: "file", text: raw, reference },
    });
  }
  for (const match of text.matchAll(URL_PATTERN)) {
    const raw = match[0];
    const start = match.index ?? 0;
    matches.push({
      start,
      end: start + raw.length,
      segment: { kind: "url", text: raw, url: raw },
    });
  }
  matches.sort((a, b) => a.start - b.start || b.end - a.end);
  const segments: FileRefSegment[] = [];
  let cursor = 0;
  for (const match of matches) {
    if (match.start < cursor) {
      // Overlaps resolve to whichever match begins earlier (a URL inside a
      // longer pattern span keeps the outer match's span).
      continue;
    }
    if (match.start > cursor) {
      segments.push({ kind: "text", text: text.slice(cursor, match.start) });
    }
    segments.push(match.segment);
    cursor = match.end;
  }
  if (cursor < text.length) {
    segments.push({ kind: "text", text: text.slice(cursor) });
  }
  return segments;
}

/** Whether a string contains at least one openable reference. */
export function hasFileReference(text: string): boolean {
  return segmentFileReferences(text).some((segment) => segment.kind !== "text");
}
