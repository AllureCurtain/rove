import { describe, expect, it } from "vitest";

import {
  createInitialMemorySettingsState,
  memorySettingsReducer,
  removeDeletedMemoryTopic,
  retainMemorySelection,
  sortMemoryTopics,
  type MemorySettingsState,
} from "./memory-settings-model";
import type {
  ProductMemoryTopic,
  ProductMemoryTopicContentResponse,
} from "./settings-platform-api-types";

function topic(
  slug: string,
  overrides: Partial<ProductMemoryTopic> = {},
): ProductMemoryTopic {
  return {
    slug,
    title: slug,
    memory_type: "project",
    scope: "project",
    confidence: 0.8,
    description: `${slug} description`,
    metadata_truncated: false,
    ...overrides,
  };
}

function detail(
  memoryTopic: ProductMemoryTopic,
): ProductMemoryTopicContentResponse {
  return {
    topic: memoryTopic,
    content: `${memoryTopic.slug} content`,
    truncated: false,
  };
}

function readyState(topics: ProductMemoryTopic[]): MemorySettingsState {
  return memorySettingsReducer(createInitialMemorySettingsState(), {
    type: "list_loaded",
    topics,
  });
}

describe("memory settings model", () => {
  it("sorts by valid update time, then deterministic title and slug keys", () => {
    const missingDateZulu = topic("zulu", { title: "Zulu" });
    const tiedBeta = topic("beta", {
      title: "beta",
      updated_at: "2026-07-26T10:00:00Z",
    });
    const newest = topic("newest", {
      updated_at: "2026-07-27T10:00:00Z",
    });
    const tiedAlpha = topic("alpha", {
      title: "Alpha",
      updated_at: "2026-07-26T10:00:00Z",
    });
    const invalidDateAble = topic("able", {
      title: "Able",
      updated_at: "not-a-date",
    });
    const input = [missingDateZulu, tiedBeta, newest, tiedAlpha, invalidDateAble];

    expect(sortMemoryTopics(input).map(({ slug }) => slug)).toEqual([
      "newest",
      "alpha",
      "beta",
      "able",
      "zulu",
    ]);
    expect(input.map(({ slug }) => slug)).toEqual([
      "zulu",
      "beta",
      "newest",
      "alpha",
      "able",
    ]);
  });

  it("retains only selections present in the refreshed topic list", () => {
    const topics = [topic("alpha"), topic("beta")];

    expect(retainMemorySelection("beta", topics)).toBe("beta");
    expect(retainMemorySelection("removed", topics)).toBeNull();
    expect(retainMemorySelection(null, topics)).toBeNull();
  });

  it("removes a deleted topic and clears only the matching selection", () => {
    const topics = [topic("alpha"), topic("beta")];

    expect(removeDeletedMemoryTopic(topics, "alpha", "alpha")).toEqual({
      topics: [topics[1]],
      selectedSlug: null,
    });
    expect(removeDeletedMemoryTopic(topics, "beta", "alpha")).toEqual({
      topics: [topics[1]],
      selectedSlug: "beta",
    });
  });

  it("preserves a valid selection across refresh and clears a removed one", () => {
    const alpha = topic("alpha");
    const beta = topic("beta");
    let state = readyState([alpha, beta]);
    state = memorySettingsReducer(state, {
      type: "topic_selected",
      slug: "alpha",
    });
    state = memorySettingsReducer(state, {
      type: "detail_loaded",
      slug: "alpha",
      detail: detail(alpha),
    });

    const retained = memorySettingsReducer(state, {
      type: "list_loaded",
      topics: [beta, alpha],
    });
    expect(retained.selectedSlug).toBe("alpha");
    expect(retained.detail?.topic.slug).toBe("alpha");

    const removed = memorySettingsReducer(retained, {
      type: "list_loaded",
      topics: [beta],
    });
    expect(removed.selectedSlug).toBeNull();
    expect(removed.detail).toBeNull();
    expect(removed.detailStatus).toBe("idle");
  });

  it("ignores stale detail transitions after another topic is selected", () => {
    const alpha = topic("alpha");
    const beta = topic("beta");
    let state = readyState([alpha, beta]);
    state = memorySettingsReducer(state, {
      type: "topic_selected",
      slug: "alpha",
    });
    state = memorySettingsReducer(state, {
      type: "topic_selected",
      slug: "beta",
    });

    const staleSuccess = memorySettingsReducer(state, {
      type: "detail_loaded",
      slug: "alpha",
      detail: detail(alpha),
    });
    const staleFailure = memorySettingsReducer(staleSuccess, {
      type: "detail_load_failed",
      slug: "alpha",
      error: "late failure",
    });

    expect(staleFailure).toBe(state);
    expect(staleFailure.selectedSlug).toBe("beta");
    expect(staleFailure.detail).toBeNull();
    expect(staleFailure.detailError).toBeNull();
  });

  it("keeps loaded topics visible when a refresh fails", () => {
    const alpha = topic("alpha");
    let state = readyState([alpha]);
    state = memorySettingsReducer(state, {
      type: "topic_selected",
      slug: "alpha",
    });
    state = memorySettingsReducer(state, { type: "list_load_started" });
    state = memorySettingsReducer(state, {
      type: "list_load_failed",
      error: "offline",
    });

    expect(state.listStatus).toBe("error");
    expect(state.listError).toBe("offline");
    expect(state.topics).toEqual([alpha]);
    expect(state.selectedSlug).toBe("alpha");
  });

  it("requires confirmation and clears selected detail only after delete succeeds", () => {
    const alpha = topic("alpha");
    let state = readyState([alpha]);
    state = memorySettingsReducer(state, {
      type: "topic_selected",
      slug: "alpha",
    });
    state = memorySettingsReducer(state, {
      type: "detail_loaded",
      slug: "alpha",
      detail: detail(alpha),
    });

    const requested = memorySettingsReducer(state, {
      type: "delete_confirmation_requested",
      slug: "alpha",
    });
    expect(requested.deleteStatus).toBe("confirming");
    expect(requested.topics).toEqual([alpha]);
    expect(requested.detail).not.toBeNull();

    const deleting = memorySettingsReducer(requested, {
      type: "delete_started",
      slug: "alpha",
    });
    const failed = memorySettingsReducer(deleting, {
      type: "delete_failed",
      slug: "alpha",
      error: "conflict",
    });
    expect(failed.deleteStatus).toBe("error");
    expect(failed.pendingDeleteSlug).toBe("alpha");
    expect(failed.selectedSlug).toBe("alpha");

    const succeeded = memorySettingsReducer(failed, {
      type: "delete_succeeded",
      slug: "alpha",
    });
    expect(succeeded.topics).toEqual([]);
    expect(succeeded.selectedSlug).toBeNull();
    expect(succeeded.detail).toBeNull();
    expect(succeeded.detailStatus).toBe("idle");
    expect(succeeded.pendingDeleteSlug).toBeNull();
    expect(succeeded.deleteStatus).toBe("idle");
  });
});
