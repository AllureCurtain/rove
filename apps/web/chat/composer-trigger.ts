/**
 * Composer trigger detection for slash commands and file mentions.
 *
 * A slash only counts inside the draft's first token, so a path mentioned
 * mid-sentence never opens the menu. An @ only counts when it starts a token,
 * which keeps email addresses from triggering it. A @" quote holds one token
 * open across spaces until it closes. Detection is a pure function of the text
 * and caret; the caller freezes it while an IME composition is in progress.
 */

export type ComposerTrigger =
  | { kind: "slash"; query: string; range: [number, number] }
  | { kind: "file"; query: string; range: [number, number]; quoted: boolean }
  | null;

const TOKEN_BOUNDARY = /[\s=]/;

export function detectTrigger(text: string, caret: number): ComposerTrigger {
  const bounded = Math.max(0, Math.min(caret, text.length));
  const { token, start } = tokenAtCaret(text, bounded);
  if (token.length === 0) {
    return null;
  }

  if (token.startsWith("/")) {
    const firstToken = text.slice(0, bounded).trimStart();
    if (!firstToken.startsWith("/") || /\s/u.test(token.slice(1))) {
      return null;
    }
    return { kind: "slash", query: token.slice(1), range: [start, bounded] };
  }

  if (token.startsWith("@")) {
    const before = start > 0 ? text[start - 1]! : "";
    if (before !== "" && !TOKEN_BOUNDARY.test(before)) {
      return null;
    }
    const quoted = token.startsWith('@"');
    const query = quoted ? token.slice(2) : token.slice(1);
    return { kind: "file", query, range: [start, bounded], quoted };
  }

  return null;
}

/** The token under the caret. An unclosed @" quote keeps spaces inside it. */
function tokenAtCaret(text: string, caret: number): { token: string; start: number } {
  const uptoCaret = text.slice(0, caret);
  const quoteAt = uptoCaret.lastIndexOf('@"');
  if (quoteAt >= 0 && !uptoCaret.slice(quoteAt + 2).includes('"')) {
    return { token: uptoCaret.slice(quoteAt), start: quoteAt };
  }
  const start = lastBoundary(uptoCaret) + 1;
  return { token: uptoCaret.slice(start), start };
}

function lastBoundary(value: string): number {
  for (let index = value.length - 1; index >= 0; index -= 1) {
    if (TOKEN_BOUNDARY.test(value[index]!)) {
      return index;
    }
  }
  return -1;
}

/** Text inserted for an accepted file mention. Directories keep a trailing slash. */
export function fileMentionInsert(path: string, directory: boolean): string {
  const needsQuotes = /\s/u.test(path);
  const body = directory ? `${path}/` : path;
  const quoted = needsQuotes ? `"${body}"` : body;
  return directory ? quoted : `${quoted} `;
}

export const COMPOSER_MENU_LIMIT = 8;
