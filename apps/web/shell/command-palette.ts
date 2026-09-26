import type {
  KeyboardShortcutActionId,
} from "../settings/keyboard-settings-model";
import type { SettingsSectionId } from "../settings/sections";

/**
 * Command palette model, ported from open-vetta's `domains/command-menu`.
 *
 * Three of the reference's decisions are carried over because they are load
 * bearing rather than cosmetic:
 *
 * 1. Actions are **data**, not closures. The builder stays a pure function that
 *    can be asserted without a DOM, and the one place that performs a navigation
 *    side effect is the host.
 * 2. The group order is a constant and never re-ranks by relevance. Sources have
 *    different latencies, so a relevance-sorted list can move the row under the
 *    cursor between the keypress and the Enter — the user would open something
 *    they never saw.
 * 3. Matching is **substring**, not fuzzy subsequence, and every whitespace token
 *    must hit (AND). Subsequence matching is especially wrong for CJK: with no
 *    word boundaries, `设置` would match `设计与配置` and "search does not find it"
 *    stops being believable. A subtitle-only hit still counts, at a lower score.
 */

export const COMMAND_PALETTE_GROUP_ORDER = [
  "actions",
  "workspaces",
  "sessions",
  "settings",
] as const;

export type CommandPaletteGroupKey =
  (typeof COMMAND_PALETTE_GROUP_ORDER)[number];

/** Per-group limit: a palette is a picker, not a list. */
export const COMMAND_PALETTE_GROUP_LIMIT = 6;

export type CommandPaletteAction =
  | { kind: "shortcut"; action: KeyboardShortcutActionId }
  | { kind: "toggle-theme" }
  | { kind: "open-settings-section"; section: SettingsSectionId }
  | { kind: "open-workspace"; workspaceId: string }
  | { kind: "open-session"; workspaceId: string; sessionId: string };

export interface CommandPaletteEntry {
  /** Globally stable: selection is anchored here, never on a list index. */
  id: string;
  groupKey: CommandPaletteGroupKey;
  title: string;
  subtitle?: string;
  /** Shown and disabled rather than silently filtered out. */
  disabled?: boolean;
  disabledReason?: string;
  /** Within-group order for equal scores. */
  order: number;
  action: CommandPaletteAction;
}

export interface CommandPaletteRange {
  start: number;
  end: number;
}

export interface CommandPaletteMatch {
  score: number;
  titleRanges: CommandPaletteRange[];
  subtitleRanges: CommandPaletteRange[];
}

/** Case- and whitespace-insensitive, enough for the labels this shell ships. */
export function normalizeCommandText(text: string): string {
  return text.trim().toLowerCase().replace(/\s+/g, " ");
}

/** Empty query → empty tokens, which the caller treats as "no filter". */
export function tokenizeCommandQuery(query: string): string[] {
  const normalized = normalizeCommandText(query);
  return normalized ? normalized.split(" ").filter(Boolean) : [];
}

function findRanges(text: string, token: string): CommandPaletteRange[] {
  const haystack = text.toLowerCase();
  const ranges: CommandPaletteRange[] = [];
  let index = haystack.indexOf(token);
  while (index !== -1) {
    ranges.push({ start: index, end: index + token.length });
    index = haystack.indexOf(token, index + token.length);
  }
  return ranges;
}

/**
 * Merges overlapping or touching ranges: two tokens hitting the same span would
 * otherwise render nested highlight nodes.
 */
export function mergeHighlightRanges(
  ranges: readonly CommandPaletteRange[],
): CommandPaletteRange[] {
  if (ranges.length <= 1) {
    return [...ranges];
  }
  const sorted = [...ranges].sort((a, b) => a.start - b.start || a.end - b.end);
  const merged: CommandPaletteRange[] = [];
  for (const range of sorted) {
    const previous = merged.at(-1);
    if (previous && range.start <= previous.end) {
      if (range.end > previous.end) {
        merged[merged.length - 1] = { start: previous.start, end: range.end };
      }
      continue;
    }
    merged.push({ ...range });
  }
  return merged;
}

/** Word boundaries for the mid-score tier; CJK has none, which prefix hits cover. */
const WORD_BOUNDARY = /[\s\-_/.:@[\]()]/;
const SCORE_PREFIX = 3;
const SCORE_WORD_START = 2;
const SCORE_ANYWHERE = 1;
/** Subtitle-only hit: findable, but clearly less relevant than a title hit. */
const SCORE_SUBTITLE = 0.5;

function positionScore(text: string, start: number): number {
  if (start === 0) {
    return SCORE_PREFIX;
  }
  const previous = text[start - 1];
  return previous && WORD_BOUNDARY.test(previous)
    ? SCORE_WORD_START
    : SCORE_ANYWHERE;
}

export function matchCommandEntry(
  entry: Pick<CommandPaletteEntry, "title" | "subtitle">,
  tokens: readonly string[],
): CommandPaletteMatch | null {
  if (tokens.length === 0) {
    return { score: 0, titleRanges: [], subtitleRanges: [] };
  }

  const titleRanges: CommandPaletteRange[] = [];
  const subtitleRanges: CommandPaletteRange[] = [];
  let score = 0;

  for (const token of tokens) {
    const inTitle = findRanges(entry.title, token);
    if (inTitle.length > 0) {
      titleRanges.push(...inTitle);
      score += positionScore(entry.title, inTitle[0]!.start);
      continue;
    }
    const inSubtitle = entry.subtitle ? findRanges(entry.subtitle, token) : [];
    if (inSubtitle.length > 0) {
      subtitleRanges.push(...inSubtitle);
      score += SCORE_SUBTITLE;
      continue;
    }
    return null;
  }

  return {
    score,
    titleRanges: mergeHighlightRanges(titleRanges),
    subtitleRanges: mergeHighlightRanges(subtitleRanges),
  };
}

export interface CommandPaletteItem {
  entry: CommandPaletteEntry;
  match: CommandPaletteMatch;
}

export interface CommandPaletteGroup {
  key: CommandPaletteGroupKey;
  items: CommandPaletteItem[];
  /** Entries hidden by the per-group limit, for an "and N more" line. */
  hiddenCount: number;
}

/**
 * Groups in the constant order, each bounded by `COMMAND_PALETTE_GROUP_LIMIT`.
 * An empty query keeps the builder's order, because the palette's first job on
 * open is to show the few things worth doing now.
 */
export function buildCommandGroups(
  entries: readonly CommandPaletteEntry[],
  query: string,
): CommandPaletteGroup[] {
  const tokens = tokenizeCommandQuery(query);
  const groups: CommandPaletteGroup[] = [];

  for (const key of COMMAND_PALETTE_GROUP_ORDER) {
    const matched: CommandPaletteItem[] = [];
    for (const entry of entries) {
      if (entry.groupKey !== key) {
        continue;
      }
      const match = matchCommandEntry(entry, tokens);
      if (match) {
        matched.push({ entry, match });
      }
    }
    matched.sort(
      (a, b) =>
        b.match.score - a.match.score ||
        a.entry.order - b.entry.order ||
        a.entry.title.localeCompare(b.entry.title),
    );
    groups.push({
      key,
      items: matched.slice(0, COMMAND_PALETTE_GROUP_LIMIT),
      hiddenCount: Math.max(0, matched.length - COMMAND_PALETTE_GROUP_LIMIT),
    });
  }

  return groups.filter((group) => group.items.length > 0);
}

/** The flat order the keyboard walks; disabled entries stay in it but are skipped. */
export function flattenCommandGroups(
  groups: readonly CommandPaletteGroup[],
): CommandPaletteItem[] {
  return groups.flatMap((group) => group.items);
}

export function firstEnabledCommandId(
  items: readonly CommandPaletteItem[],
): string | null {
  return items.find((item) => !item.entry.disabled)?.entry.id ?? null;
}

/**
 * Moves the selection by one row, wrapping, and skipping disabled entries. The
 * reference anchors on the id for the same reason: an async arrival must never
 * leave the selection pointing at an index that now means a different command.
 */
export function moveCommandSelection(
  items: readonly CommandPaletteItem[],
  selectedId: string | null,
  delta: number,
): string | null {
  const enabled = items.filter((item) => !item.entry.disabled);
  if (enabled.length === 0) {
    return null;
  }
  const current = enabled.findIndex((item) => item.entry.id === selectedId);
  const next = (current + delta + enabled.length) % enabled.length;
  return enabled[next]!.entry.id;
}

/** Splits a title into highlighted and plain runs for rendering. */
export function splitCommandHighlight(
  text: string,
  ranges: readonly CommandPaletteRange[],
): Array<{ text: string; highlighted: boolean }> {
  if (ranges.length === 0) {
    return [{ text, highlighted: false }];
  }
  const runs: Array<{ text: string; highlighted: boolean }> = [];
  let cursor = 0;
  for (const range of ranges) {
    const start = Math.max(cursor, Math.min(range.start, text.length));
    const end = Math.max(start, Math.min(range.end, text.length));
    if (start > cursor) {
      runs.push({ text: text.slice(cursor, start), highlighted: false });
    }
    if (end > start) {
      runs.push({ text: text.slice(start, end), highlighted: true });
    }
    cursor = end;
  }
  if (cursor < text.length) {
    runs.push({ text: text.slice(cursor), highlighted: false });
  }
  return runs;
}

