import { describe, expect, it } from "vitest";

import {
  AUTO_TITLE_MAX_CODE_POINTS,
  DEFAULT_SESSION_TITLE,
  autoSessionTitle,
  hasDefaultSessionTitle,
} from "./session-auto-title";

describe("session auto title", () => {
  it("recognises the server's placeholder title, including an empty one", () => {
    expect(hasDefaultSessionTitle(DEFAULT_SESSION_TITLE)).toBe(true);
    expect(hasDefaultSessionTitle("")).toBe(true);
    expect(hasDefaultSessionTitle("   ")).toBe(true);
    expect(hasDefaultSessionTitle(null)).toBe(true);
    expect(hasDefaultSessionTitle(undefined)).toBe(true);
    expect(hasDefaultSessionTitle("Fix the flaky test")).toBe(false);
    // The approximation the design records: a rename back to the template is
    // indistinguishable from never having named the session.
    expect(hasDefaultSessionTitle(` ${DEFAULT_SESSION_TITLE} `)).toBe(true);
  });

  it("keeps a short first message as written", () => {
    expect(autoSessionTitle("Why is the rail collapsing?")).toBe(
      "Why is the rail collapsing?",
    );
  });

  it("collapses whitespace because the title is rendered on one line", () => {
    expect(autoSessionTitle("  Fix\n\nthe   title  ")).toBe("Fix the title");
  });

  it("returns null for a message with no usable text", () => {
    expect(autoSessionTitle("")).toBeNull();
    expect(autoSessionTitle(" \n\t ")).toBeNull();
  });

  it("truncates to the design's 48 code points with an ellipsis", () => {
    const message = "a".repeat(AUTO_TITLE_MAX_CODE_POINTS + 10);
    const title = autoSessionTitle(message);

    expect(title).toBe(`${"a".repeat(AUTO_TITLE_MAX_CODE_POINTS)}...`);
    expect(Array.from(title!)).toHaveLength(AUTO_TITLE_MAX_CODE_POINTS + 3);
  });

  it("does not add an ellipsis at exactly the limit", () => {
    const message = "b".repeat(AUTO_TITLE_MAX_CODE_POINTS);

    expect(autoSessionTitle(message)).toBe(message);
  });

  it("counts code points, so an astral character is never split", () => {
    // 47 ASCII characters then an emoji that is two UTF-16 code units wide.
    const message = `${"c".repeat(AUTO_TITLE_MAX_CODE_POINTS - 1)}🧭 tail`;
    const title = autoSessionTitle(message);

    expect(title).toBe(`${"c".repeat(AUTO_TITLE_MAX_CODE_POINTS - 1)}🧭...`);
    expect(title).not.toContain("\uFFFD");
    expect(title!.startsWith("c".repeat(AUTO_TITLE_MAX_CODE_POINTS - 1))).toBe(
      true,
    );
  });
});
