import { describe, expect, it } from "vitest";

import type { PromptPruningFacts } from "../lib/rove-types";
import {
  KNOWN_PRUNING_POLICIES,
  formatPruningBytes,
  promptPruningSummary,
} from "./prompt-pruning";

/**
 * The pruning facts are R9's own record of what a bounded compaction request
 * left out. Projecting them is where the panel can go wrong in two directions:
 * claiming a loss the server never reported, or hiding one it did. These cases
 * pin both, plus the arithmetic the reader sees.
 */

function pruning(
  overrides: Partial<PromptPruningFacts> = {},
): PromptPruningFacts {
  return {
    pruned_tool_results: 0,
    pruned_payload_bytes: 0,
    pruned_excerpt_bytes: 0,
    pruned_omitted_messages: 0,
    pruned_omitted_bytes: 0,
    pruned_payload_digests: [],
    pruning_policy: null,
    ...overrides,
  };
}

describe("prompt pruning summary", () => {
  it("shows nothing when the build carries no pruning facts", () => {
    // Absent is the wire saying "this turn pruned nothing", so there is no row
    // to render — an empty panel is a claim, not a neutral state.
    expect(promptPruningSummary(undefined)).toBeNull();
  });

  it("shows nothing when every fact is zero", () => {
    expect(promptPruningSummary(pruning())).toBeNull();
  });

  it("reads the wire shape a live turn hands it, where unpruned members are absent", () => {
    // `lib/rove-state.ts` decodes the live `prompt_built` metadata with the same
    // reader the restored path uses, and the runtime omits every member that is
    // zero or empty. This is the omission-only shape its own compaction test
    // covers: no elision counts, no digest list, and a policy. Reading it must
    // not produce `undefined` counts, a `NaN` loss, or an absent digest list that
    // the panel would index into.
    const summary = promptPruningSummary({
      pruned_omitted_messages: 9,
      pruned_omitted_bytes: 30_720,
      pruning_policy: "rove.pruning.v1",
    });
    expect(summary).not.toBeNull();
    expect(summary?.elidedToolResults).toBe(0);
    expect(summary?.elidedBytes).toBe(0);
    expect(summary?.omittedMessages).toBe(9);
    expect(summary?.omittedBytes).toBe(30_720);
    expect(summary?.digests).toEqual([]);
    expect(summary?.policy).toBe("rove.pruning.v1");
    expect(summary?.policyKnown).toBe(true);
  });

  it("reads a wire object that names no rule at all", () => {
    const summary = promptPruningSummary({ pruned_tool_results: 1 });
    expect(summary?.policy).toBeNull();
    expect(summary?.policyKnown).toBe(false);
    expect(summary?.elidedToolResults).toBe(1);
  });

  it("reads the bytes the request did not carry as payload minus excerpt", () => {
    const summary = promptPruningSummary(
      pruning({
        pruned_tool_results: 3,
        pruned_payload_bytes: 90_000,
        pruned_excerpt_bytes: 6_000,
        pruning_policy: "rove.pruning.v1",
      }),
    );
    expect(summary?.elidedToolResults).toBe(3);
    expect(summary?.elidedBytes).toBe(84_000);
  });

  it("clamps a negative loss instead of printing one", () => {
    // An excerpt larger than the payload it replaced is a server contradiction.
    // Showing "-512 B not sent" would turn that into a figure the reader trusts.
    const summary = promptPruningSummary(
      pruning({ pruned_tool_results: 1, pruned_payload_bytes: 512, pruned_excerpt_bytes: 1_024 }),
    );
    expect(summary?.elidedBytes).toBe(0);
  });

  it("reports omitted messages without inventing elided ones", () => {
    const summary = promptPruningSummary(
      pruning({ pruned_omitted_messages: 161, pruned_omitted_bytes: 220_000 }),
    );
    expect(summary?.omittedMessages).toBe(161);
    expect(summary?.omittedBytes).toBe(220_000);
    expect(summary?.elidedToolResults).toBe(0);
    expect(summary?.elidedBytes).toBe(0);
    expect(summary?.digests).toEqual([]);
  });

  it("keeps the digest order the server published", () => {
    const summary = promptPruningSummary(
      pruning({
        pruned_tool_results: 2,
        pruned_payload_digests: ["call-9:sha256:aaaa", "call-4:sha256:bbbb"],
        pruning_policy: "rove.pruning.v1",
      }),
    );
    expect(summary?.digests).toEqual(["call-9:sha256:aaaa", "call-4:sha256:bbbb"]);
  });

  it("names the policy it knows and refuses to name one it does not", () => {
    const known = promptPruningSummary(
      pruning({ pruned_tool_results: 1, pruning_policy: "rove.pruning.v1" }),
    );
    expect(known?.policyKnown).toBe(true);
    expect(known?.policy).toBe("rove.pruning.v1");

    const unknown = promptPruningSummary(
      pruning({ pruned_tool_results: 1, pruning_policy: "rove.pruning.v9" }),
    );
    // The rule is still reported: the build says it cannot name it, not that
    // nothing was pruned.
    expect(unknown?.policyKnown).toBe(false);
    expect(unknown?.policy).toBe("rove.pruning.v9");

    const unnamed = promptPruningSummary(pruning({ pruned_tool_results: 1 }));
    expect(unnamed?.policyKnown).toBe(false);
    expect(unnamed?.policy).toBeNull();
  });

  it("knows exactly the policies it lists", () => {
    expect(KNOWN_PRUNING_POLICIES).toContain("rove.pruning.v1");
    const summary = promptPruningSummary(
      pruning({ pruned_tool_results: 1, pruning_policy: "rove.pruning.v1" }),
    );
    expect(summary?.policyKnown).toBe(true);
  });
});

describe("pruning byte formatting", () => {
  it("uses the shell's own byte sizes", () => {
    expect(formatPruningBytes(0)).toBe("0 B");
    expect(formatPruningBytes(1024)).toBe("1.0 KiB");
    expect(formatPruningBytes(84_000)).toBe("82.0 KiB");
    expect(formatPruningBytes(5 * 1024 * 1024)).toBe("5.0 MiB");
  });
});
