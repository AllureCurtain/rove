import { describe, expect, it } from "vitest";

import {
  EMPTY_SELECTION,
  applySelectionClick,
  pruneSelection,
} from "./rail-selection";

const ORDER = ["a", "b", "c", "d", "e"];

describe("applySelectionClick", () => {
  it("an additive click toggles the row and sets the anchor", () => {
    const next = applySelectionClick(EMPTY_SELECTION, ORDER, "b", {
      additive: true,
      range: false,
    });
    expect([...next.selected]).toEqual(["b"]);
    expect(next.anchor).toBe("b");
    const toggled = applySelectionClick(next, ORDER, "b", {
      additive: true,
      range: false,
    });
    expect(toggled.selected.size).toBe(0);
  });

  it("a range click selects anchor to target in visible order", () => {
    const first = applySelectionClick(EMPTY_SELECTION, ORDER, "b", {
      additive: true,
      range: false,
    });
    const ranged = applySelectionClick(first, ORDER, "e", {
      additive: false,
      range: true,
    });
    expect([...ranged.selected].sort()).toEqual(["b", "c", "d", "e"]);
  });

  it("a reverse range selects the same span", () => {
    const first = applySelectionClick(EMPTY_SELECTION, ORDER, "d", {
      additive: true,
      range: false,
    });
    const ranged = applySelectionClick(first, ORDER, "b", {
      additive: false,
      range: true,
    });
    expect([...ranged.selected].sort()).toEqual(["b", "c", "d"]);
  });

  it("a range click without an anchor selects just the row", () => {
    const ranged = applySelectionClick(EMPTY_SELECTION, ORDER, "c", {
      additive: false,
      range: true,
    });
    expect([...ranged.selected]).toEqual(["c"]);
    expect(ranged.anchor).toBe("c");
  });
});

describe("pruneSelection", () => {
  it("drops ids that left the catalog and clears a dead anchor", () => {
    const selection = applySelectionClick(EMPTY_SELECTION, ORDER, "b", {
      additive: true,
      range: false,
    });
    const ranged = applySelectionClick(selection, ORDER, "d", {
      additive: false,
      range: true,
    });
    const pruned = pruneSelection(ranged, new Set(["c", "d"]));
    expect([...pruned.selected].sort()).toEqual(["c", "d"]);
    expect(pruned.anchor).toBeNull();
    expect(pruneSelection(EMPTY_SELECTION, new Set())).toBe(EMPTY_SELECTION);
  });
});
