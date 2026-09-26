/**
 * Large paste handling for the composer.
 *
 * A paste past the threshold becomes an attachment chip instead of landing in
 * the textarea, counted in Unicode code points rather than UTF-16 code units
 * so a string of CJK characters is measured the way the reader counts it.
 * The caps are small because the content stays in browser memory.
 */

export const LARGE_PASTE_THRESHOLD = 600;
export const MAX_PASTE_ATTACHMENTS = 10;
export const MAX_PASTE_BYTES = 256 * 1024;

export function codePointLength(value: string): number {
  return [...value].length;
}

export interface PasteDecision {
  kind: "insert" | "attach" | "reject";
  reason?: "too-many" | "too-large";
}

export function decidePaste(
  text: string,
  current: { count: number; bytes: number },
): PasteDecision {
  if (codePointLength(text) <= LARGE_PASTE_THRESHOLD) {
    return { kind: "insert" };
  }
  if (current.count >= MAX_PASTE_ATTACHMENTS) {
    return { kind: "reject", reason: "too-many" };
  }
  const bytes = new TextEncoder().encode(text).length;
  if (current.bytes + bytes > MAX_PASTE_BYTES) {
    return { kind: "reject", reason: "too-large" };
  }
  return { kind: "attach" };
}

/** Plain text of a clipboard paste, dropping rich HTML. */
export function plainPasteText(plain: string): string {
  return plain.replace(/\r\n/gu, "\n");
}

/**
 * Fold accepted paste attachments into the outgoing prompt: a marker line per
 * attachment, then the content in a fence. The marker is prompt content, not
 * UI copy, so it stays a fixed ASCII line.
 */
export function composeMessageWithAttachments(
  message: string,
  attachments: readonly { text: string }[],
): string {
  if (attachments.length === 0) {
    return message;
  }
  const blocks = attachments.map(
    (attachment) =>
      `\n\n[pasted content: ${codePointLength(attachment.text)} characters]\n\`\`\`\n${attachment.text}\n\`\`\``,
  );
  return message + blocks.join("");
}
