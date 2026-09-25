import { describe, expect, it } from "vitest";

import { detectTrigger, fileMentionInsert } from "./composer-trigger";

describe("detectTrigger", () => {
  it("detects a slash only inside the first token", () => {
    expect(detectTrigger("/set", 4)?.kind).toBe("slash");
    expect(detectTrigger("see /src/main.rs", 15)).toBeNull();
  });

  it("does not trigger on an email address", () => {
    expect(detectTrigger("write to a@b.com", 15)).toBeNull();
  });

  it("allows spaces inside a quoted mention", () => {
    const trigger = detectTrigger('@"my file', 9);
    expect(trigger?.kind).toBe("file");
    if (trigger?.kind === "file") {
      expect(trigger.quoted).toBe(true);
      expect(trigger.query).toBe("my file");
    }
  });

  it("closes when the caret leaves the token", () => {
    expect(detectTrigger("/set ", 5)).toBeNull();
  });
});

describe("fileMentionInsert", () => {
  it("adds a trailing space for a file and a slash for a directory", () => {
    expect(fileMentionInsert("src/main.rs", false)).toBe("src/main.rs ");
    expect(fileMentionInsert("src", true)).toBe("src/");
  });

  it("quotes a path that contains whitespace", () => {
    expect(fileMentionInsert("my file.rs", false)).toBe('"my file.rs" ');
  });
});
