import { describe, expect, it } from "vitest";

import {
  formatShortcutBinding,
  migrateOverrides,
  normalizeOverride,
  type ShortcutBinding,
} from "./keybindings";

const defaults = {
  id: "focus",
  binding: { key: "/", primary: false, shift: false, alt: false },
};
const none: { id: string; binding: ShortcutBinding }[] = [];

describe("normalizeOverride", () => {
  it("stores nothing when the override repeats the default", () => {
    const result = normalizeOverride(
      { key: "/", primary: false, shift: false, alt: false },
      defaults,
      none,
    );
    expect(result.store).toBeUndefined();
    expect(result.rejection).toBeUndefined();
  });

  it("stores null to unbind", () => {
    expect(normalizeOverride(null, defaults, none).store).toBeNull();
  });

  it("rejects a reserved browser binding", () => {
    const result = normalizeOverride(
      { key: "w", primary: true, shift: false, alt: false },
      defaults,
      none,
    );
    expect(result.rejection?.reason).toBe("reserved");
  });

  it("accepts the same key without the primary modifier", () => {
    const result = normalizeOverride(
      { key: "w", primary: false, shift: false, alt: false },
      defaults,
      none,
    );
    expect(result.rejection).toBeUndefined();
    expect(result.store).toEqual({ key: "w", primary: false, shift: false, alt: false });
  });

  it("rejects a binding another action already uses", () => {
    const result = normalizeOverride(
      { key: "k", primary: true, shift: false, alt: false },
      defaults,
      [{ id: "palette", binding: { key: "k", primary: true, shift: false, alt: false } }],
    );
    expect(result.rejection).toEqual({ reason: "conflict", conflictsWith: "palette" });
  });
});

describe("migrateOverrides", () => {
  it("folds a retired id into its replacement", () => {
    const migrated = migrateOverrides(
      { "old-focus": { key: "j", primary: false, shift: false, alt: false } },
      { "old-focus": "focus" },
    );
    expect(migrated.focus).toEqual({ key: "j", primary: false, shift: false, alt: false });
    expect(migrated["old-focus"]).toBeUndefined();
  });
});

describe("formatShortcutBinding", () => {
  it("formats modifiers in a stable order", () => {
    expect(formatShortcutBinding({ key: "k", primary: true, shift: false, alt: false })).toBe(
      "Ctrl / Cmd + K",
    );
    expect(formatShortcutBinding({ key: "/", primary: false, shift: false, alt: false })).toBe("/");
    expect(
      formatShortcutBinding({ key: "ArrowUp", primary: false, shift: true, alt: true }),
    ).toBe("Alt + Shift + ArrowUp");
  });
});
