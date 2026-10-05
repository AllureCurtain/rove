import { describe, expect, it } from "vitest";

import { codePointLength, decidePaste, plainPasteText } from "./composer-paste";

describe("decidePaste", () => {
  it("inserts text at the threshold and attaches just past it", () => {
    expect(decidePaste("a".repeat(600), { count: 0, bytes: 0 }).kind).toBe("insert");
    expect(decidePaste("a".repeat(601), { count: 0, bytes: 0 }).kind).toBe("attach");
  });

  it("counts code points, not UTF-16 code units", () => {
    const text = "😀".repeat(600);
    expect(text.length).toBeGreaterThan(codePointLength(text));
    expect(decidePaste(text, { count: 0, bytes: 0 }).kind).toBe("insert");
  });

  it("rejects once the attachment cap is reached", () => {
    const decision = decidePaste("a".repeat(601), { count: 10, bytes: 0 });
    expect(decision).toEqual({ kind: "reject", reason: "too-many" });
  });

  it("rejects a paste that would exceed the byte budget", () => {
    const decision = decidePaste("a".repeat(601), { count: 0, bytes: 256 * 1024 });
    expect(decision.reason).toBe("too-large");
  });
});

describe("plainPasteText", () => {
  it("normalizes newlines", () => {
    expect(plainPasteText("a\r\nb")).toBe("a\nb");
  });
});
