import { describe, expect, it } from "vitest";

import { diffLineCounts } from "./TurnChangeCard";

describe("diffLineCounts", () => {
  it("counts added and deleted hunk lines", () => {
    const diff = [
      "@@ -1,3 +1,3 @@",
      " context",
      "-old line",
      "+new line",
      "+another",
    ].join("\n");
    expect(diffLineCounts(diff)).toEqual({ additions: 2, deletions: 1 });
  });

  it("ignores the +++/--- file headers", () => {
    const diff = [
      "--- a/src/main.rs",
      "+++ b/src/main.rs",
      "@@ -1 +1 @@",
      "-old",
      "+new",
    ].join("\n");
    expect(diffLineCounts(diff)).toEqual({ additions: 1, deletions: 1 });
  });

  it("returns zeros for a headerless empty diff", () => {
    expect(diffLineCounts("")).toEqual({ additions: 0, deletions: 0 });
  });
});
