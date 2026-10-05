import type { SessionRecord } from "../state/product-types";

/**
 * §7.1: the Sessions zone files every session under one of five buckets.
 * `archived` is a state bucket, not a time bucket — an archived session leaves
 * the timeline wherever it sat and lands here regardless of `updatedAt`.
 */
export type SessionTimeGroup =
  | "today"
  | "yesterday"
  | "week"
  | "older"
  | "archived";

export const SESSION_TIME_GROUPS: readonly SessionTimeGroup[] = [
  "today",
  "yesterday",
  "week",
  "older",
  "archived",
];

/** Sort modes for the Sessions toolbar. */
export type SessionSort = "recent" | "title";

const DAY_MS = 86_400_000;

function startOfLocalDay(now: Date): number {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
}

export function sessionTimeGroup(
  session: Pick<SessionRecord, "archived" | "updatedAt">,
  now: Date,
): SessionTimeGroup {
  if (session.archived) {
    return "archived";
  }
  const updated = Date.parse(session.updatedAt);
  if (Number.isNaN(updated)) {
    return "older";
  }
  const dayStart = startOfLocalDay(now);
  if (updated >= dayStart) {
    return "today";
  }
  if (updated >= dayStart - DAY_MS) {
    return "yesterday";
  }
  if (updated >= dayStart - 6 * DAY_MS) {
    return "week";
  }
  return "older";
}

export function compareSessions(
  sort: SessionSort,
): (left: SessionRecord, right: SessionRecord) => number {
  if (sort === "title") {
    return (left, right) =>
      left.title.localeCompare(right.title) || left.id.localeCompare(right.id);
  }
  return (left, right) =>
    right.updatedAt.localeCompare(left.updatedAt) ||
    left.id.localeCompare(right.id);
}

/** Bucket a flat session list into the five fixed groups, each sorted. */
export function groupSessionsByTime(
  sessions: readonly SessionRecord[],
  now: Date,
  sort: SessionSort,
): Map<SessionTimeGroup, SessionRecord[]> {
  const groups = new Map<SessionTimeGroup, SessionRecord[]>(
    SESSION_TIME_GROUPS.map((group) => [group, []]),
  );
  for (const session of sessions) {
    groups.get(sessionTimeGroup(session, now))!.push(session);
  }
  const comparator = compareSessions(sort);
  for (const list of groups.values()) {
    list.sort(comparator);
  }
  return groups;
}
