import { describe, expect, it } from "vitest";

import { initialDisclosure, reduceDisclosure } from "./disclosure";

describe("reduceDisclosure", () => {
  it("follows automatic changes until the reader acts", () => {
    const opened = reduceDisclosure(initialDisclosure(false), { type: "auto", wantOpen: true });
    expect(opened).toEqual({ open: true, userControlled: false });
    const closed = reduceDisclosure(opened, { type: "auto", wantOpen: false });
    expect(closed.open).toBe(false);
  });

  it("ignores automatic changes after the reader takes over", () => {
    const taken = reduceDisclosure(initialDisclosure(true), { type: "user", open: false });
    const ignored = reduceDisclosure(taken, { type: "auto", wantOpen: true });
    expect(ignored).toEqual({ open: false, userControlled: true });
  });

  it("lets the reader change their mind", () => {
    const closed = reduceDisclosure(initialDisclosure(true), { type: "user", open: false });
    const reopened = reduceDisclosure(closed, { type: "user", open: true });
    expect(reopened).toEqual({ open: true, userControlled: true });
  });
});
