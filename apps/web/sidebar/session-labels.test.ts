import { describe, expect, it } from "vitest";

import { createTranslator } from "../copy";
import { sessionOutcomeLabel, sessionResultMark, sessionSubtitle } from "./session-labels";

const t = createTranslator("zh-CN");

describe("session result mark", () => {
  it("reports the last finished turn's outcome, not the session status", () => {
    // Both sessions are idle; only the outcome separates a finished turn from a
    // session that never ran, which is the R3 reason the field exists.
    expect(sessionResultMark({ status: "idle", lastOutcome: "success" }, false, t)).toEqual({
      tone: "success",
      label: t("workspace.sessionOutcomeSuccess"),
    });
    expect(sessionResultMark({ status: "idle", lastOutcome: null }, false, t)).toBeNull();
  });

  it("gives failure and cancellation their own tones", () => {
    expect(sessionResultMark({ status: "needs_attention", lastOutcome: "failed" }, false, t)).toEqual({
      tone: "danger",
      label: t("workspace.sessionOutcomeFailed"),
    });
    expect(sessionResultMark({ status: "idle", lastOutcome: "cancelled" }, false, t)).toEqual({
      tone: "neutral",
      label: t("workspace.sessionOutcomeCancelled"),
    });
  });

  it("clears the mark for the session the user is looking at", () => {
    expect(sessionResultMark({ status: "error", lastOutcome: "failed" }, true, t)).toBeNull();
  });

  it("keeps the pre-R3 failure rule for a payload without an outcome", () => {
    expect(sessionResultMark({ status: "error" }, false, t)).toEqual({
      tone: "danger",
      label: t("workspace.sessionErrorDot"),
    });
    expect(sessionResultMark({ status: "idle" }, false, t)).toBeNull();
  });
});

describe("session outcome subtitle", () => {
  it("appends the outcome to the workspace name so it stays searchable", () => {
    expect(
      sessionSubtitle({ lastOutcome: "failed" }, "server", t),
    ).toBe(`server · ${t("workspace.sessionOutcomeFailed")}`);
  });

  it("leaves the workspace name alone when there is no outcome", () => {
    expect(sessionSubtitle({ lastOutcome: null }, "server", t)).toBe("server");
    expect(sessionSubtitle({ lastOutcome: "success" }, "", t)).toBe(
      t("workspace.sessionOutcomeSuccess"),
    );
  });

  it("has no wording for an unknown outcome", () => {
    expect(sessionOutcomeLabel(undefined, t)).toBeNull();
  });
});
