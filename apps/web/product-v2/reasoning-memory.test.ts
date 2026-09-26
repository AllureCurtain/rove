import { describe, expect, it } from "vitest";

import {
  REASONING_BY_MODEL_KEY,
  REASONING_BY_MODEL_LIMIT,
  parseReasoningByModel,
  reasoningForModel,
  readReasoningByModel,
  rememberReasoning,
  rememberReasoningInMap,
} from "./reasoning-memory";

function memoryStorage(initial?: string): Pick<Storage, "getItem" | "setItem"> & {
  value: string | null;
} {
  return {
    value: initial ?? null,
    getItem(key: string) {
      return key === REASONING_BY_MODEL_KEY ? this.value : null;
    },
    setItem(key: string, value: string) {
      if (key === REASONING_BY_MODEL_KEY) {
        this.value = value;
      }
    },
  };
}

describe("reasoning memory", () => {
  it("remembers one model's choice without touching another model's", () => {
    const map = rememberReasoningInMap(
      rememberReasoningInMap({}, "gpt-5", "high"),
      "claude-sonnet-4",
      "low",
    );

    expect(map).toEqual({ "gpt-5": "high", "claude-sonnet-4": "low" });
    expect(reasoningForModel(map, "gpt-5")).toBe("high");
    expect(reasoningForModel(map, "claude-sonnet-4")).toBe("low");
    expect(reasoningForModel(map, "never-seen")).toBeNull();
  });

  it("matches model ids as written, and ignores an empty model", () => {
    const map = rememberReasoningInMap({}, " gpt-5 ", "medium");

    expect(reasoningForModel(map, "gpt-5")).toBe("medium");
    expect(reasoningForModel(map, "")).toBeNull();
    expect(rememberReasoningInMap(map, "   ", "low")).toBe(map);
  });

  it("keeps the newest choice for a model and stays bounded", () => {
    let map: Record<string, "default" | "low" | "medium" | "high"> = {};
    for (let index = 0; index < REASONING_BY_MODEL_LIMIT + 3; index += 1) {
      map = rememberReasoningInMap(map, `model-${index}`, "low");
    }

    expect(Object.keys(map)).toHaveLength(REASONING_BY_MODEL_LIMIT);
    // The oldest entries left, the newest is kept.
    expect(reasoningForModel(map, "model-0")).toBeNull();
    expect(reasoningForModel(map, `model-${REASONING_BY_MODEL_LIMIT + 2}`)).toBe(
      "low",
    );
  });

  it("re-recording a model moves it to the newest position", () => {
    let map = rememberReasoningInMap({}, "a", "low");
    for (let index = 0; index < REASONING_BY_MODEL_LIMIT - 1; index += 1) {
      map = rememberReasoningInMap(map, `filler-${index}`, "low");
    }
    map = rememberReasoningInMap(map, "a", "high");
    map = rememberReasoningInMap(map, "overflow", "low");

    expect(reasoningForModel(map, "a")).toBe("high");
    expect(Object.keys(map)).toHaveLength(REASONING_BY_MODEL_LIMIT);
  });

  it("drops stored entries that are not known reasoning values", () => {
    const parsed = parseReasoningByModel(
      JSON.stringify({
        "gpt-5": "high",
        "bad-value": "ultra",
        "": "low",
        number: 3,
        nested: { reasoning: "low" },
      }),
    );

    expect(parsed).toEqual({ "gpt-5": "high" });
  });

  it("treats unusable stored text as no memory at all", () => {
    expect(parseReasoningByModel(null)).toEqual({});
    expect(parseReasoningByModel("not json")).toEqual({});
    expect(parseReasoningByModel("[1,2,3]")).toEqual({});
    expect(parseReasoningByModel('"high"')).toEqual({});
  });

  it("round-trips through storage and survives a blocked one", () => {
    const storage = memoryStorage();
    rememberReasoning("gpt-5", "high", storage);
    expect(readReasoningByModel(storage)).toEqual({ "gpt-5": "high" });
    expect(reasoningForModel(readReasoningByModel(storage), "gpt-5")).toBe(
      "high",
    );

    const throwing = {
      getItem() {
        throw new Error("blocked");
      },
      setItem() {
        throw new Error("blocked");
      },
    };
    expect(readReasoningByModel(throwing)).toEqual({});
    expect(() => rememberReasoning("gpt-5", "high", throwing)).not.toThrow();
    expect(readReasoningByModel(null)).toEqual({});
    expect(rememberReasoning("gpt-5", "high", null)).toEqual({ "gpt-5": "high" });
  });
});
