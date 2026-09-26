import type { SessionRecord } from "../state/product-types";

/**
 * Sidebar presentation helpers shared by the session row and its hover card
 * (design F4). They used to live inside `WorkspaceTree.tsx`; the card needs the
 * same words, so they moved here instead of being copied.
 */

export function sessionStatusLabel(
  status: SessionRecord["status"],
  t: (path: string) => string,
): string {
  switch (status) {
    case "running":
      return t("workspace.running");
    case "needs_attention":
      return t("workspace.needsAttention");
    case "error":
      return t("inspector.statusFailed");
    default:
      return t("inspector.statusQueued");
  }
}

export type SessionResultTone = "success" | "danger" | "neutral";

export interface SessionResultMark {
  tone: SessionResultTone;
  label: string;
}

function outcomeTone(
  outcome: SessionRecord["lastOutcome"],
): SessionResultTone | null {
  switch (outcome) {
    case "success":
      return "success";
    case "failed":
      return "danger";
    case "cancelled":
      return "neutral";
    default:
      return null;
  }
}

/**
 * W4.4 + R3: the row dot reports the outcome of the last finished turn.
 *
 * Before R3 the status field was the only source, so a successful turn and a
 * never-run session were both drawn as nothing and only `error` carried a dot.
 * The outcome is authoritative now; a payload that predates it (no
 * `lastOutcome` at all) still falls back to the pre-R3 `error` rule. Opening a
 * session clears the mark, which is the existing "read it" gesture.
 */
export function sessionResultMark(
  session: Pick<SessionRecord, "status" | "lastOutcome">,
  active: boolean,
  t: (path: string) => string,
): SessionResultMark | null {
  if (active) {
    return null;
  }
  switch (outcomeTone(session.lastOutcome)) {
    case "success":
      return { tone: "success", label: t("workspace.sessionOutcomeSuccess") };
    case "danger":
      return { tone: "danger", label: t("workspace.sessionOutcomeFailed") };
    case "neutral":
      return { tone: "neutral", label: t("workspace.sessionOutcomeCancelled") };
    default:
      return session.status === "error"
        ? { tone: "danger", label: t("workspace.sessionErrorDot") }
        : null;
  }
}

/** The same outcome as one short word, for a text-only surface. */
export function sessionOutcomeLabel(
  outcome: SessionRecord["lastOutcome"],
  t: (path: string) => string,
): string | null {
  switch (outcomeTone(outcome)) {
    case "success":
      return t("workspace.sessionOutcomeSuccess");
    case "danger":
      return t("workspace.sessionOutcomeFailed");
    case "neutral":
      return t("workspace.sessionOutcomeCancelled");
    default:
      return null;
  }
}

/**
 * Command-palette subtitle: the workspace the session belongs to, plus its last
 * outcome when it has one. Joining them keeps the subtitle searchable, so
 * "failed" finds the sessions that failed.
 */
export function sessionSubtitle(
  session: Pick<SessionRecord, "lastOutcome">,
  workspaceName: string,
  t: (path: string) => string,
): string {
  const outcome = sessionOutcomeLabel(session.lastOutcome, t);
  return [workspaceName, outcome].filter((part) => part).join(" · ");
}

export function shortId(value: string): string {
  return value.length <= 10 ? value : value.slice(0, 10);
}

export function formatDisplayPath(path: string): string {
  return path.startsWith("\\\\?\\") ? path.slice(4) : path;
}
