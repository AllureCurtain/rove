import {
  PRODUCT_REASONING_PREFERENCES,
  type ProductReasoningPreference,
} from "../product/product-api-types";

/**
 * Per-model reasoning memory (design F10.1).
 *
 * The session model config stays server-owned; this only remembers which
 * reasoning effort was last chosen *for each model id*, so switching models in
 * the quick control brings that choice back instead of resetting it to the
 * provider default. open-vetta keeps the same map in a jotai atom
 * (`reasoningByModelAtom`); rove has no jotai, so it is a small `rove.ui-*`
 * localStorage preference next to the layout ones.
 */
export const REASONING_BY_MODEL_KEY = "rove.ui-reasoning-by-model";

/** Bounded so a long model history cannot grow a browser preference forever. */
export const REASONING_BY_MODEL_LIMIT = 50;

export type ReasoningByModel = Record<string, ProductReasoningPreference>;

type ReasoningStorage = Pick<Storage, "getItem" | "setItem">;

function isReasoningPreference(
  value: unknown,
): value is ProductReasoningPreference {
  return (
    typeof value === "string" &&
    (PRODUCT_REASONING_PREFERENCES as readonly string[]).includes(value)
  );
}

/**
 * Parse a stored map. Anything that is not a non-empty model id mapped to a
 * known reasoning value is dropped rather than trusted, so a hand-edited or
 * older value can never reach the server request.
 */
export function parseReasoningByModel(
  raw: string | null | undefined,
): ReasoningByModel {
  if (!raw) {
    return {};
  }
  let decoded: unknown;
  try {
    decoded = JSON.parse(raw);
  } catch {
    return {};
  }
  if (decoded === null || typeof decoded !== "object" || Array.isArray(decoded)) {
    return {};
  }
  const entries = Object.entries(decoded as Record<string, unknown>).filter(
    (entry): entry is [string, ProductReasoningPreference] =>
      entry[0].trim().length > 0 && isReasoningPreference(entry[1]),
  );
  return Object.fromEntries(entries.slice(-REASONING_BY_MODEL_LIMIT));
}

/**
 * The map with one model's choice recorded. The newest choice is re-inserted at
 * the end and the oldest entries leave once the map is full.
 */
export function rememberReasoningInMap(
  map: ReasoningByModel,
  model: string,
  reasoning: ProductReasoningPreference,
): ReasoningByModel {
  const key = model.trim();
  if (!key) {
    return map;
  }
  const next: ReasoningByModel = { ...map };
  delete next[key];
  next[key] = reasoning;
  const keys = Object.keys(next);
  for (const stale of keys.slice(
    0,
    Math.max(0, keys.length - REASONING_BY_MODEL_LIMIT),
  )) {
    delete next[stale];
  }
  return next;
}

/** The remembered choice for `model`, or `null` when there is none. */
export function reasoningForModel(
  map: ReasoningByModel,
  model: string,
): ProductReasoningPreference | null {
  const key = model.trim();
  return key ? (map[key] ?? null) : null;
}

/** Read the map from storage; an unreadable or blocked storage reads as empty. */
export function readReasoningByModel(
  storage: Pick<Storage, "getItem"> | null | undefined = currentStorage(),
): ReasoningByModel {
  if (!storage) {
    return {};
  }
  try {
    return parseReasoningByModel(storage.getItem(REASONING_BY_MODEL_KEY));
  } catch {
    return {};
  }
}

/**
 * Record one model's choice and return the new map. A blocked or full storage is
 * swallowed: this is a convenience preference and must never fail a save.
 */
export function rememberReasoning(
  model: string,
  reasoning: ProductReasoningPreference,
  storage: ReasoningStorage | null | undefined = currentStorage(),
): ReasoningByModel {
  const next = rememberReasoningInMap(
    readReasoningByModel(storage),
    model,
    reasoning,
  );
  if (storage) {
    try {
      storage.setItem(REASONING_BY_MODEL_KEY, JSON.stringify(next));
    } catch {
      // A blocked storage quota must not break model selection.
    }
  }
  return next;
}

function currentStorage(): Storage | null {
  return typeof window === "undefined" ? null : window.localStorage;
}
