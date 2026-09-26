import { describe, expect, it } from "vitest";

import {
  newBackgroundFailures,
  providerTestToastKind,
  type BackgroundSessionLike,
} from "./notifications";

function session(
  id: string,
  status: string,
  title = `Session ${id}`,
): BackgroundSessionLike {
  return { id, title, status };
}

describe("background session failures", () => {
  it("stays quiet about a session that was already failed when it was first seen", () => {
    const { failures, known } = newBackgroundFailures(
      new Map(),
      [session("a", "error")],
      null,
    );
    expect(failures).toEqual([]);
    expect(known.get("a")).toBe("error");
  });

  it("reports the change from running to error", () => {
    const first = newBackgroundFailures(new Map(), [session("a", "running")], null);
    const second = newBackgroundFailures(first.known, [session("a", "error")], null);
    expect(second.failures.map((item) => item.id)).toEqual(["a"]);
  });

  it("does not repeat a failure that is still the current status", () => {
    const known = new Map([["a", "error"]]);
    expect(newBackgroundFailures(known, [session("a", "error")], null).failures).toEqual([]);
  });

  it("reports a later failure after the session ran again", () => {
    const known = new Map([["a", "error"]]);
    const recovered = newBackgroundFailures(known, [session("a", "running")], null);
    const failedAgain = newBackgroundFailures(
      recovered.known,
      [session("a", "error")],
      null,
    );
    expect(failedAgain.failures.map((item) => item.id)).toEqual(["a"]);
  });

  it("leaves the active session to its own inline error", () => {
    const known = new Map([["a", "running"]]);
    expect(
      newBackgroundFailures(known, [session("a", "error")], "a").failures,
    ).toEqual([]);
  });

  it("reports every changed session and forgets the ones that are gone", () => {
    const known = new Map([
      ["a", "running"],
      ["gone", "running"],
    ]);
    const { failures, known: next } = newBackgroundFailures(
      known,
      [session("a", "error"), session("b", "error"), session("c", "idle")],
      null,
    );
    // `b` is new, so it is recorded rather than announced.
    expect(failures.map((item) => item.id)).toEqual(["a"]);
    expect([...next.keys()].sort()).toEqual(["a", "b", "c"]);
  });
});

describe("provider test notifications", () => {
  it("leaves the inline result alone while its view is mounted", () => {
    expect(providerTestToastKind(true, "pass")).toBeNull();
    expect(providerTestToastKind(true, "incomplete")).toBeNull();
    expect(providerTestToastKind(true, "fail")).toBeNull();
  });

  it("reports the result once the view is gone", () => {
    expect(providerTestToastKind(false, "pass")).toBe("success");
    expect(providerTestToastKind(false, "incomplete")).toBe("info");
    expect(providerTestToastKind(false, "fail")).toBe("error");
  });
});
