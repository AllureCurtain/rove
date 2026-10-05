import { describe, expect, it } from "vitest";

import type { SessionRecord } from "../state/product-types";
import {
  compareSessions,
  groupSessionsByTime,
  sessionTimeGroup,
} from "./rail-time-groups";

const NOW = new Date("2026-10-05T15:00:00"); // a Monday, local time

function session(
  id: string,
  updatedAt: string,
  overrides: Partial<SessionRecord> = {},
): SessionRecord {
  return {
    id,
    workspaceId: "ws",
    title: id,
    createdAt: updatedAt,
    updatedAt,
    status: "idle",
    archived: false,
    hasDurableTurn: false,
    ...overrides,
  };
}

describe("sessionTimeGroup", () => {
  it("buckets by local calendar day and state", () => {
    expect(sessionTimeGroup(session("a", "2026-10-05T08:00:00"), NOW)).toBe("today");
    expect(sessionTimeGroup(session("b", "2026-10-04T23:59:00"), NOW)).toBe("yesterday");
    expect(sessionTimeGroup(session("c", "2026-10-01T00:00:00"), NOW)).toBe("week");
    expect(sessionTimeGroup(session("d", "2026-09-01T00:00:00"), NOW)).toBe("older");
    expect(sessionTimeGroup(session("e", "bad-date"), NOW)).toBe("older");
  });

  it("files archived sessions under archived regardless of time", () => {
    expect(
      sessionTimeGroup(
        session("a", "2026-10-05T14:00:00", { archived: true }),
        NOW,
      ),
    ).toBe("archived");
  });
});

describe("groupSessionsByTime", () => {
  it("orders each bucket by the chosen sort", () => {
    const sessions = [
      session("b", "2026-10-05T01:00:00"),
      session("a", "2026-10-05T09:00:00"),
      session("z", "2026-10-05T05:00:00"),
    ];
    const recent = groupSessionsByTime(sessions, NOW, "recent").get("today")!;
    expect(recent.map((item) => item.id)).toEqual(["a", "z", "b"]);
    const byTitle = groupSessionsByTime(
      sessions.map((item, index) => ({ ...item, title: `t${index}` })),
      NOW,
      "title",
    ).get("today")!;
    expect(byTitle.map((item) => item.title)).toEqual(["t0", "t1", "t2"]);
  });

  it("always produces every group, including the empty ones", () => {
    const groups = groupSessionsByTime([], NOW, "recent");
    expect([...groups.keys()]).toEqual([
      "today",
      "yesterday",
      "week",
      "older",
      "archived",
    ]);
  });
});

describe("compareSessions", () => {
  it("breaks recent ties by id so the order is stable", () => {
    const compare = compareSessions("recent");
    const a = session("a", "2026-10-05T00:00:00Z");
    const b = session("b", "2026-10-05T00:00:00Z");
    expect([b, a].sort(compare).map((item) => item.id)).toEqual(["a", "b"]);
  });
});
