import type { ToolMutationOperation } from "../lib/rove-types";
import type { ToolCallView } from "../lib/rove-state";

/**
 * Typed presentation of one tool call.
 *
 * The head (title, subtitle, chips) is cheap and always computed. The body
 * blocks are computed only while the card is expanded, so a collapsed tool
 * never pays for diff slicing or output truncation. Unknown tools degrade to
 * a note instead of throwing.
 */

export const DIFF_CONTEXT_LINES = 2;
export const DIFF_MAX_LINES = 400;
export const MATCH_MAX_ENTRIES = 200;
export const CODE_MAX_LINES = 2_000;
export const NOTE_MAX_CHARACTERS = 4_000;

const SENSITIVE_KEY = /token|secret|authorization|password|credential/iu;

export type ToolBlock =
  | { kind: "fields"; rows: { label: string; value: string }[] }
  | { kind: "code"; language: string; text: string; truncated: boolean }
  | {
      kind: "diff";
      path: string;
      operation: ToolMutationOperation;
      text: string;
      truncated: boolean;
    }
  | { kind: "files"; entries: { path: string; annotation?: string }[]; truncated: boolean }
  | { kind: "matches"; entries: { path: string; line?: number; text: string }[]; truncated: boolean }
  | { kind: "note"; text: string };

export interface ToolChip {
  label: string;
  tone: "neutral" | "ok" | "warn" | "danger";
}

export interface ToolPresentation {
  title: string;
  subtitle: string;
  chips: ToolChip[];
  blocks: ToolBlock[];
}

export function buildToolPresentation(
  tool: ToolCallView,
  expanded: boolean,
): ToolPresentation {
  const head = {
    title: tool.name,
    subtitle: subtitleFor(tool),
    chips: chipsFor(tool),
  };
  if (!expanded) {
    return { ...head, blocks: [] };
  }
  return { ...head, blocks: blocksFor(tool) };
}

function subtitleFor(tool: ToolCallView): string {
  const mutations = tool.mutations ?? [];
  if (mutations.length > 0) {
    const first = mutations[0]!;
    return mutations.length === 1 ? first.path : `${first.path} (+${mutations.length - 1})`;
  }
  const paths = tool.metadata?.affected_paths ?? [];
  if (paths.length > 0) {
    return paths.length === 1 ? paths[0]! : `${paths[0]} (+${paths.length - 1})`;
  }
  if (tool.error?.code) {
    return tool.error.code;
  }
  return "";
}

function chipsFor(tool: ToolCallView): ToolChip[] {
  const chips: ToolChip[] = [];
  const metadata = tool.metadata;
  if (metadata) {
    chips.push({ label: metadata.status, tone: toneForStatus(metadata.status) });
    if (metadata.risk_level === "high") {
      chips.push({ label: "high risk", tone: "danger" });
    }
    chips.push({
      label: metadata.read_only ? "read only" : "mutation capable",
      tone: metadata.read_only ? "neutral" : "warn",
    });
    if (metadata.workspace_changed) {
      chips.push({ label: "workspace changed", tone: "warn" });
    }
    if (metadata.error_code) {
      chips.push({ label: metadata.error_code, tone: "danger" });
    }
  } else {
    chips.push({ label: tool.status, tone: toneForStatus(tool.status) });
  }
  return chips;
}

function toneForStatus(status: string): ToolChip["tone"] {
  if (status === "ok" || status === "done") {
    return "ok";
  }
  if (status === "error" || status === "rejected") {
    return "danger";
  }
  if (status === "partial_success" || status === "waiting" || status === "running") {
    return "warn";
  }
  return "neutral";
}

function blocksFor(tool: ToolCallView): ToolBlock[] {
  const blocks: ToolBlock[] = [];
  const mutations = tool.mutations ?? [];

  if (mutations.length > 0) {
    for (const mutation of mutations) {
      if (!mutation.diff) {
        continue;
      }
      const trimmed = trimDiff(mutation.diff);
      blocks.push({
        kind: "diff",
        path: mutation.path,
        operation: mutation.operation,
        text: trimmed.text,
        truncated: trimmed.truncated,
      });
    }
  }

  const matches = matchEntries(tool);
  if (matches) {
    blocks.push(matches);
  }

  const paths = tool.metadata?.affected_paths ?? [];
  if (mutations.length === 0 && paths.length > 0) {
    blocks.push({
      kind: "files",
      entries: paths.map((path) => ({ path })),
      truncated: false,
    });
  }

  if (blocks.length === 0 && tool.output) {
    const lines = tool.output.split("\n");
    blocks.push({
      kind: "code",
      language: "text",
      text: lines.slice(0, CODE_MAX_LINES).join("\n"),
      truncated: lines.length > CODE_MAX_LINES,
    });
  }

  const rows = fieldRows(tool);
  if (rows.length > 0) {
    blocks.push({ kind: "fields", rows });
  }

  if (blocks.length === 0) {
    blocks.push({
      kind: "note",
      text: (tool.details || tool.output || "").slice(0, NOTE_MAX_CHARACTERS),
    });
  }
  return blocks;
}

function fieldRows(tool: ToolCallView): { label: string; value: string }[] {
  const metadata = tool.metadata;
  if (!metadata) {
    return [];
  }
  const rows: { label: string; value: string }[] = [];
  if (metadata.diff_summary) {
    for (const summary of metadata.diff_summary) {
      rows.push({ label: "summary", value: summary });
    }
  }
  return rows.filter((row) => !SENSITIVE_KEY.test(row.label) && !SENSITIVE_KEY.test(row.value));
}

/**
 * Keep two lines of context around every changed line and stop at
 * DIFF_MAX_LINES. Hunk headers are preserved so the result still reads as a
 * diff. Returns the input untouched when it is already small.
 */
export function trimDiff(diff: string): { text: string; truncated: boolean } {
  const lines = diff.split("\n");
  if (lines.length <= DIFF_MAX_LINES) {
    return { text: diff, truncated: false };
  }
  const changed = lines.map((line) => /^[+-]/.test(line) && !/^(\+\+\+|---)/.test(line));
  const keep = lines.map(() => false);
  changed.forEach((isChanged, index) => {
    if (!isChanged) {
      return;
    }
    for (
      let cursor = Math.max(0, index - DIFF_CONTEXT_LINES);
      cursor <= Math.min(lines.length - 1, index + DIFF_CONTEXT_LINES);
      cursor += 1
    ) {
      keep[cursor] = true;
    }
  });
  lines.forEach((line, index) => {
    if (line.startsWith("@@") || line.startsWith("diff ") || line.startsWith("index ")) {
      keep[index] = true;
    }
  });

  const kept: string[] = [];
  let skipped = 0;
  const flushSkipped = () => {
    if (skipped > 0) {
      kept.push(`… ${skipped} lines omitted`);
      skipped = 0;
    }
  };
  lines.forEach((line, index) => {
    if (kept.length >= DIFF_MAX_LINES) {
      skipped += 1;
      return;
    }
    if (keep[index]) {
      flushSkipped();
      kept.push(line);
    } else {
      skipped += 1;
    }
  });
  flushSkipped();
  return { text: kept.join("\n"), truncated: true };
}

function matchEntries(tool: ToolCallView): ToolBlock | null {
  const summary = tool.metadata?.diff_summary ?? [];
  const looksLikeSearch = summary.some((line) => /^\S+:\d+:/.test(line));
  if (!looksLikeSearch) {
    return null;
  }
  const entries = summary.slice(0, MATCH_MAX_ENTRIES).map((line) => {
    const match = /^(\S+):(\d+):(.*)$/.exec(line);
    return match
      ? { path: match[1]!, line: Number(match[2]), text: match[3]! }
      : { path: "", text: line };
  });
  return { kind: "matches", entries, truncated: summary.length > MATCH_MAX_ENTRIES };
}
