/**
 * Stored shortcut overrides.
 *
 * Only overrides that differ from the default are stored, so a future change
 * to a default reaches everyone who had not customized it. An override of
 * null unbinds the shortcut. A retired action id folds into its replacement
 * instead of being dropped.
 */

export interface ShortcutBinding {
  key: string;
  /** Ctrl on Windows/Linux, Cmd on macOS — the shell's "primary" modifier. */
  primary: boolean;
  shift: boolean;
  alt: boolean;
}

export type ShortcutOverride = ShortcutBinding | null;

export interface ShortcutDefaults {
  id: string;
  binding: ShortcutBinding;
}

/** Ctrl/Cmd+W, Ctrl/Cmd+N, Ctrl/Cmd+T belong to the browser. */
const RESERVED = new Set(["w", "n", "t"]);

export function isReservedBinding(binding: ShortcutBinding): boolean {
  return (
    RESERVED.has(binding.key.toLowerCase()) &&
    binding.primary &&
    !binding.shift &&
    !binding.alt
  );
}

export function bindingsEqual(left: ShortcutBinding, right: ShortcutBinding): boolean {
  return (
    left.key.toLowerCase() === right.key.toLowerCase() &&
    left.primary === right.primary &&
    left.shift === right.shift &&
    left.alt === right.alt
  );
}

export interface OverrideRejection {
  reason: "reserved" | "conflict";
  conflictsWith?: string;
}

/**
 * Decide whether an override should be stored. Returns null when it repeats
 * the default, so the caller deletes any stored value.
 */
export function normalizeOverride(
  binding: ShortcutOverride,
  defaults: ShortcutDefaults,
  others: readonly { id: string; binding: ShortcutBinding }[],
): { store: ShortcutOverride | undefined; rejection?: OverrideRejection } {
  if (binding === null) {
    return { store: null };
  }
  if (isReservedBinding(binding)) {
    return { store: undefined, rejection: { reason: "reserved" } };
  }
  const conflict = others.find((other) => other.id !== defaults.id && bindingsEqual(other.binding, binding));
  if (conflict) {
    return { store: undefined, rejection: { reason: "conflict", conflictsWith: conflict.id } };
  }
  if (bindingsEqual(binding, defaults.binding)) {
    return { store: undefined };
  }
  return { store: binding };
}

/** Fold overrides keyed by a retired id into the id that replaced it. */
export function migrateOverrides(
  overrides: Readonly<Record<string, ShortcutOverride>>,
  retired: Readonly<Record<string, string>>,
): Record<string, ShortcutOverride> {
  const migrated: Record<string, ShortcutOverride> = {};
  for (const [id, override] of Object.entries(overrides)) {
    migrated[retired[id] ?? id] = override;
  }
  return migrated;
}

/** Display form of a stored binding, matching the defaults' vocabulary. */
export function formatShortcutBinding(binding: ShortcutBinding): string {
  const parts: string[] = [];
  if (binding.primary) {
    parts.push("Ctrl / Cmd");
  }
  if (binding.alt) {
    parts.push("Alt");
  }
  if (binding.shift) {
    parts.push("Shift");
  }
  const key =
    binding.key === " "
      ? "Space"
      : binding.key.length === 1
        ? binding.key.toUpperCase()
        : binding.key;
  parts.push(key);
  return parts.join(" + ");
}
