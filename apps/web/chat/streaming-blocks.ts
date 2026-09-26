/**
 * Split markdown into alternating prose and closed-fence segments.
 *
 * A fence that never closes stays prose, so text the model writes after an
 * opening fence during streaming is not swallowed into a code block. Only
 * fences at column zero count; indented fences are left untouched.
 */

export type MarkdownSegment =
  | { kind: "prose"; text: string }
  | { kind: "code"; text: string };

const FENCE = /^(`{3,}|~{3,})/;

export function segmentMarkdown(markdown: string): MarkdownSegment[] {
  const lines = markdown.split("\n");
  const segments: MarkdownSegment[] = [];
  let prose: string[] = [];
  let openMarker: string | null = null;
  let code: string[] = [];

  const flushProse = () => {
    if (prose.length > 0) {
      segments.push({ kind: "prose", text: prose.join("\n") });
      prose = [];
    }
  };

  for (const line of lines) {
    const match: RegExpExecArray | null = openMarker === null ? FENCE.exec(line) : null;
    if (openMarker === null && match) {
      flushProse();
      openMarker = match[1]![0] === "`" ? "`" : "~";
      code = [line];
      continue;
    }
    if (openMarker !== null) {
      code.push(line);
      const closing = FENCE.exec(line);
      if (closing && closing[1]!.startsWith(openMarker) && code.length > 1) {
        segments.push({ kind: "code", text: code.join("\n") });
        openMarker = null;
        code = [];
      }
      continue;
    }
    prose.push(line);
  }

  if (openMarker !== null) {
    // Unclosed fence: render it as prose so the trailing text stays visible.
    prose = [...code, ...prose];
  }
  flushProse();
  return segments;
}
