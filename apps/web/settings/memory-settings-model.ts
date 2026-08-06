import type {
  ProductMemoryTopic,
  ProductMemoryTopicContentResponse,
} from "./settings-platform-api-types";

export type MemoryLoadStatus = "idle" | "loading" | "ready" | "error";
export type MemoryDeleteStatus =
  | "idle"
  | "confirming"
  | "deleting"
  | "error";

export interface MemorySettingsState {
  topics: readonly ProductMemoryTopic[];
  listStatus: MemoryLoadStatus;
  listError: string | null;
  selectedSlug: string | null;
  detail: ProductMemoryTopicContentResponse | null;
  detailStatus: MemoryLoadStatus;
  detailError: string | null;
  detailRequestVersion: number;
  pendingDeleteSlug: string | null;
  deleteStatus: MemoryDeleteStatus;
  deleteError: string | null;
}

export type MemorySettingsAction =
  | { type: "list_load_started" }
  | { type: "list_loaded"; topics: readonly ProductMemoryTopic[] }
  | { type: "list_load_failed"; error: string }
  | { type: "topic_selected"; slug: string }
  | { type: "detail_request_started"; slug: string }
  | {
      type: "detail_loaded";
      slug: string;
      detail: ProductMemoryTopicContentResponse;
    }
  | { type: "detail_load_failed"; slug: string; error: string }
  | { type: "detail_retry_requested" }
  | {
      type: "topic_saved";
      detail: ProductMemoryTopicContentResponse;
    }
  | { type: "delete_confirmation_requested"; slug: string }
  | { type: "delete_confirmation_cancelled" }
  | { type: "delete_started"; slug: string }
  | { type: "delete_failed"; slug: string; error: string }
  | { type: "delete_succeeded"; slug: string }
  | { type: "delete_reset" };

export function createInitialMemorySettingsState(): MemorySettingsState {
  return {
    topics: [],
    listStatus: "loading",
    listError: null,
    selectedSlug: null,
    detail: null,
    detailStatus: "idle",
    detailError: null,
    detailRequestVersion: 0,
    pendingDeleteSlug: null,
    deleteStatus: "idle",
    deleteError: null,
  };
}

function topicTimestamp(topic: ProductMemoryTopic): number {
  const value = topic.updated_at ?? topic.created_at;
  if (value === undefined) {
    return Number.NEGATIVE_INFINITY;
  }
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : Number.NEGATIVE_INFINITY;
}

function normalizedTopicLabel(topic: ProductMemoryTopic): string {
  return (topic.title.trim() || topic.slug).toLowerCase();
}

function compareText(left: string, right: string): number {
  if (left < right) {
    return -1;
  }
  if (left > right) {
    return 1;
  }
  return 0;
}

/** Newest topics lead; stable textual keys make ties deterministic. */
export function sortMemoryTopics(
  topics: readonly ProductMemoryTopic[],
): ProductMemoryTopic[] {
  return topics
    .map((topic, originalIndex) => ({ topic, originalIndex }))
    .sort((left, right) => {
      const leftTimestamp = topicTimestamp(left.topic);
      const rightTimestamp = topicTimestamp(right.topic);
      if (leftTimestamp !== rightTimestamp) {
        return rightTimestamp > leftTimestamp ? 1 : -1;
      }
      const labelDifference = compareText(
        normalizedTopicLabel(left.topic),
        normalizedTopicLabel(right.topic),
      );
      if (labelDifference !== 0) {
        return labelDifference;
      }
      const slugDifference = compareText(left.topic.slug, right.topic.slug);
      return slugDifference !== 0
        ? slugDifference
        : left.originalIndex - right.originalIndex;
    })
    .map(({ topic }) => topic);
}

export function retainMemorySelection(
  selectedSlug: string | null,
  topics: readonly ProductMemoryTopic[],
): string | null {
  if (selectedSlug === null) {
    return null;
  }
  return topics.some((topic) => topic.slug === selectedSlug)
    ? selectedSlug
    : null;
}

export interface DeletedMemoryTopicResult {
  topics: ProductMemoryTopic[];
  selectedSlug: string | null;
}

export function removeDeletedMemoryTopic(
  topics: readonly ProductMemoryTopic[],
  selectedSlug: string | null,
  deletedSlug: string,
): DeletedMemoryTopicResult {
  return {
    topics: topics.filter((topic) => topic.slug !== deletedSlug),
    selectedSlug: selectedSlug === deletedSlug ? null : selectedSlug,
  };
}

export function memoryTopicDisplayTitle(topic: ProductMemoryTopic): string {
  return topic.title.trim() || topic.slug;
}

export function memorySettingsReducer(
  state: MemorySettingsState,
  action: MemorySettingsAction,
): MemorySettingsState {
  switch (action.type) {
    case "list_load_started":
      return {
        ...state,
        listStatus: "loading",
        listError: null,
      };

    case "list_loaded": {
      const topics = sortMemoryTopics(action.topics);
      const selectedSlug = retainMemorySelection(state.selectedSlug, topics);
      const selectionWasRemoved =
        state.selectedSlug !== null && selectedSlug === null;
      const pendingDeleteSlug = retainMemorySelection(
        state.pendingDeleteSlug,
        topics,
      );
      return {
        ...state,
        topics,
        listStatus: "ready",
        listError: null,
        selectedSlug,
        detail: selectionWasRemoved ? null : state.detail,
        detailStatus: selectionWasRemoved ? "idle" : state.detailStatus,
        detailError: selectionWasRemoved ? null : state.detailError,
        pendingDeleteSlug,
        deleteStatus:
          pendingDeleteSlug === null ? "idle" : state.deleteStatus,
        deleteError: pendingDeleteSlug === null ? null : state.deleteError,
      };
    }

    case "list_load_failed":
      return {
        ...state,
        listStatus: "error",
        listError: action.error,
      };

    case "topic_selected":
      if (!state.topics.some((topic) => topic.slug === action.slug)) {
        return state;
      }
      if (state.selectedSlug === action.slug) {
        return state;
      }
      return {
        ...state,
        selectedSlug: action.slug,
        detail: null,
        detailStatus: "loading",
        detailError: null,
        detailRequestVersion: state.detailRequestVersion + 1,
        pendingDeleteSlug: null,
        deleteStatus: "idle",
        deleteError: null,
      };

    case "detail_request_started":
      if (state.selectedSlug !== action.slug) {
        return state;
      }
      return {
        ...state,
        detail: null,
        detailStatus: "loading",
        detailError: null,
      };

    case "detail_loaded":
      if (
        state.selectedSlug !== action.slug ||
        action.detail.topic.slug !== action.slug
      ) {
        return state;
      }
      return {
        ...state,
        detail: action.detail,
        detailStatus: "ready",
        detailError: null,
      };

    case "detail_load_failed":
      if (state.selectedSlug !== action.slug) {
        return state;
      }
      return {
        ...state,
        detail: null,
        detailStatus: "error",
        detailError: action.error,
      };

    case "detail_retry_requested":
      if (state.selectedSlug === null || state.deleteStatus === "deleting") {
        return state;
      }
      return {
        ...state,
        detail: null,
        detailStatus: "loading",
        detailError: null,
        detailRequestVersion: state.detailRequestVersion + 1,
      };

    case "topic_saved": {
      const saved = action.detail.topic;
      const topics = sortMemoryTopics([
        ...state.topics.filter((topic) => topic.slug !== saved.slug),
        saved,
      ]);
      return {
        ...state,
        topics,
        listStatus: "ready",
        listError: null,
        selectedSlug: saved.slug,
        detail: action.detail,
        detailStatus: "ready",
        detailError: null,
        pendingDeleteSlug: null,
        deleteStatus: "idle",
        deleteError: null,
      };
    }

    case "delete_confirmation_requested":
      if (!state.topics.some((topic) => topic.slug === action.slug)) {
        return state;
      }
      return {
        ...state,
        pendingDeleteSlug: action.slug,
        deleteStatus: "confirming",
        deleteError: null,
      };

    case "delete_confirmation_cancelled":
      if (state.deleteStatus === "deleting") {
        return state;
      }
      return {
        ...state,
        pendingDeleteSlug: null,
        deleteStatus: "idle",
        deleteError: null,
      };

    case "delete_started":
      if (state.pendingDeleteSlug !== action.slug) {
        return state;
      }
      return {
        ...state,
        listStatus: "ready",
        listError: null,
        deleteStatus: "deleting",
        deleteError: null,
      };

    case "delete_failed":
      if (state.pendingDeleteSlug !== action.slug) {
        return state;
      }
      return {
        ...state,
        deleteStatus: "error",
        deleteError: action.error,
      };

    case "delete_succeeded": {
      if (state.pendingDeleteSlug !== action.slug) {
        return state;
      }
      const deletion = removeDeletedMemoryTopic(
        state.topics,
        state.selectedSlug,
        action.slug,
      );
      const selectionWasRemoved =
        state.selectedSlug !== null && deletion.selectedSlug === null;
      return {
        ...state,
        topics: deletion.topics,
        listStatus: "ready",
        listError: null,
        selectedSlug: deletion.selectedSlug,
        detail: selectionWasRemoved ? null : state.detail,
        detailStatus: selectionWasRemoved ? "idle" : state.detailStatus,
        detailError: selectionWasRemoved ? null : state.detailError,
        pendingDeleteSlug: null,
        deleteStatus: "idle",
        deleteError: null,
      };
    }

    case "delete_reset":
      return {
        ...state,
        pendingDeleteSlug: null,
        deleteStatus: "idle",
        deleteError: null,
      };
  }
}
