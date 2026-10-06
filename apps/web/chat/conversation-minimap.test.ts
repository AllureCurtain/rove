import { describe, expect, it } from "vitest";

import {
  CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS,
  alignMinimapMarkers,
  buildConversationMinimapMarkers,
  shouldRenderConversationMinimap,
  type ConversationMinimapMessage,
} from "./conversation-minimap";

function message(
  id: string,
  role: ConversationMinimapMessage["role"],
  content: string,
): ConversationMinimapMessage {
  return { id, role, content };
}

describe("conversation minimap", () => {
  it("builds one user and one assistant marker per turn", () => {
    const markers = buildConversationMinimapMarkers([
      message("u1", "user", "First question"),
      message("a1", "assistant", "First answer"),
      message("u2", "user", "Second question"),
      message("a2", "assistant", "Second answer"),
    ]);

    expect(markers).toEqual([
      { id: "u1", role: "user", preview: "First question" },
      { id: "a1", role: "assistant", preview: "First answer" },
      { id: "u2", role: "user", preview: "Second question" },
      { id: "a2", role: "assistant", preview: "Second answer" },
    ]);
  });

  it("merges a turn's assistant messages into its single marker", () => {
    const markers = buildConversationMinimapMarkers([
      message("u1", "user", "Question"),
      message("a1", "assistant", "Thinking out loud"),
      message("a2", "assistant", "Final answer"),
    ]);

    expect(markers).toHaveLength(2);
    expect(markers[1]).toEqual({
      id: "a1",
      role: "assistant",
      preview: "Thinking out loud\n\nFinal answer",
    });
  });

  it("keeps the first non-empty assistant message as the turn's anchor", () => {
    const markers = buildConversationMinimapMarkers([
      message("u1", "user", "Question"),
      // A streaming turn publishes an empty assistant bubble before any prose.
      message("a-empty", "assistant", "   "),
      message("a1", "assistant", "Answer"),
    ]);

    expect(markers).toEqual([
      { id: "u1", role: "user", preview: "Question" },
      { id: "a1", role: "assistant", preview: "Answer" },
    ]);
  });

  it("trims previews and caps a merged preview", () => {
    const long = "x".repeat(CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS + 50);
    const markers = buildConversationMinimapMarkers([
      message("u1", "user", `  ${long}  `),
      message("a1", "assistant", long),
      message("a2", "assistant", long),
    ]);

    expect(markers[0].preview).toHaveLength(CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS);
    expect(markers[1].preview).toHaveLength(CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS);
    expect(markers[1].preview).not.toContain("\n\n");
  });

  it("anchors assistant prose that arrives without a user marker", () => {
    const markers = buildConversationMinimapMarkers([
      message("a1", "assistant", "Restored answer"),
    ]);

    expect(markers).toEqual([
      { id: "a1", role: "assistant", preview: "Restored answer" },
    ]);
  });

  it("renders only when there is something to navigate", () => {
    expect(
      shouldRenderConversationMinimap({ markerCount: 4, overflows: true, hasEarlier: false }),
    ).toBe(true);
    expect(
      shouldRenderConversationMinimap({ markerCount: 2, overflows: true, hasEarlier: false }),
    ).toBe(true);
    expect(
      shouldRenderConversationMinimap({ markerCount: 1, overflows: true, hasEarlier: false }),
    ).toBe(false);
    expect(
      shouldRenderConversationMinimap({ markerCount: 3, overflows: false, hasEarlier: false }),
    ).toBe(false);
    // Earlier turns are not loaded, so the rail also signals history above.
    expect(
      shouldRenderConversationMinimap({ markerCount: 1, overflows: false, hasEarlier: true }),
    ).toBe(true);
  });
});

describe("alignMinimapMarkers", () => {
  // Track 400px, marker 14px, gap 3px → usable 386, pitch 17.
  it("places each marker at its turn's fraction of the track", () => {
    const tops = alignMinimapMarkers([0.1, 0.5, 0.9], 400, 14, 3);
    expect(tops).toHaveLength(3);
    [38.6, 193, 347.4].forEach((expected, index) => {
      expect(tops[index]).toBeCloseTo(expected, 6);
    });
  });

  it("spreads a dense cluster to the minimum pitch without reordering", () => {
    const tops = alignMinimapMarkers([0.4, 0.41, 0.42, 0.43], 400, 14, 3);
    for (let index = 1; index < tops.length; index += 1) {
      expect(tops[index]).toBeGreaterThanOrEqual(tops[index - 1] + 17);
    }
  });

  it("keeps the tail on the track when the spread would overflow", () => {
    const tops = alignMinimapMarkers([0.5, 0.9, 0.95, 1], 400, 14, 3);
    expect(tops.at(-1)).toBeLessThanOrEqual(386);
    // Sorted even after the backward pass.
    for (let index = 1; index < tops.length; index += 1) {
      expect(tops[index]).toBeGreaterThanOrEqual(tops[index - 1]);
    }
  });

  it("clamps degenerate density onto the track instead of losing markers", () => {
    const fractions = Array.from({ length: 40 }, () => 0.5);
    const tops = alignMinimapMarkers(fractions, 400, 14, 3);
    expect(tops).toHaveLength(40);
    for (const top of tops) {
      expect(top).toBeGreaterThanOrEqual(0);
      expect(top).toBeLessThanOrEqual(386);
    }
  });
});
