import { describe, expect, it } from "vitest";

import { LatestWinsGate } from "./latest-wins";

describe("LatestWinsGate", () => {
  it("keeps only the newest token current", () => {
    const gate = new LatestWinsGate();

    const first = gate.begin();
    const second = gate.begin();

    expect(gate.isCurrent(first)).toBe(false);
    expect(gate.isCurrent(second)).toBe(true);
  });

  it("drops an in-flight request that resolves after a newer one", () => {
    const gate = new LatestWinsGate();
    const committed: string[] = [];

    const requestA = gate.begin();
    const requestB = gate.begin();

    // B answers first, then A's late response arrives and must not land.
    if (gate.isCurrent(requestB)) {
      committed.push("B");
    }
    if (gate.isCurrent(requestA)) {
      committed.push("A");
    }

    expect(committed).toEqual(["B"]);
  });

  it("invalidates every in-flight request on teardown", () => {
    const gate = new LatestWinsGate();
    const inFlight = gate.begin();

    gate.invalidate();

    expect(gate.isCurrent(inFlight)).toBe(false);
    // A request started after teardown is current again.
    expect(gate.isCurrent(gate.begin())).toBe(true);
  });
});
