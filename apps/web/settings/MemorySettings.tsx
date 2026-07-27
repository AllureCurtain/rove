"use client";

import { ReloadIcon, TrashIcon } from "@radix-ui/react-icons";
import {
  useCallback,
  useEffect,
  useReducer,
  useRef,
  type CSSProperties,
} from "react";

import {
  createInitialMemorySettingsState,
  memorySettingsReducer,
  memoryTopicDisplayTitle,
} from "./memory-settings-model";
import type { ProductMemoryTopic } from "./settings-platform-api-types";
import type { SettingsPlatformClient } from "./settings-platform-client";

export interface MemorySettingsProps {
  client: SettingsPlatformClient;
}

const mutedTextStyle: CSSProperties = {
  margin: 0,
  color: "var(--muted)",
  lineHeight: 1.5,
};

const cardHeadingStyle: CSSProperties = {
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "space-between",
  gap: 12,
  flexWrap: "wrap",
};

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  return fallback;
}

function topicMetadata(topic: ProductMemoryTopic): string {
  const confidence = `${Math.round(topic.confidence * 100)}% confidence`;
  return `${topic.memory_type} · ${topic.scope} · ${confidence}`;
}

function metadataValue(value: string | undefined): string {
  return value?.trim() || "Not recorded";
}

export function MemorySettings({ client }: MemorySettingsProps) {
  const [state, dispatch] = useReducer(
    memorySettingsReducer,
    undefined,
    createInitialMemorySettingsState,
  );
  const mountedRef = useRef(false);
  const clientRef = useRef(client);
  const listGenerationRef = useRef(0);
  const detailGenerationRef = useRef(0);
  const deleteGenerationRef = useRef(0);
  const listAbortRef = useRef<AbortController | null>(null);
  const detailAbortRef = useRef<AbortController | null>(null);
  const deleteAbortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      listGenerationRef.current += 1;
      detailGenerationRef.current += 1;
      deleteGenerationRef.current += 1;
      listAbortRef.current?.abort();
      detailAbortRef.current?.abort();
      deleteAbortRef.current?.abort();
    };
  }, []);

  useEffect(() => {
    if (clientRef.current === client) {
      return;
    }
    clientRef.current = client;
    deleteGenerationRef.current += 1;
    deleteAbortRef.current?.abort();
    deleteAbortRef.current = null;
    dispatch({ type: "delete_reset" });
  }, [client]);

  const refreshTopics = useCallback(async (): Promise<boolean> => {
    const generation = listGenerationRef.current + 1;
    listGenerationRef.current = generation;
    listAbortRef.current?.abort();
    const controller = new AbortController();
    listAbortRef.current = controller;
    dispatch({ type: "list_load_started" });

    try {
      const response = await client.listMemoryTopics({
        signal: controller.signal,
      });
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== listGenerationRef.current
      ) {
        return false;
      }
      dispatch({ type: "list_loaded", topics: response.topics });
      return true;
    } catch (error) {
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== listGenerationRef.current
      ) {
        return false;
      }
      dispatch({
        type: "list_load_failed",
        error: errorMessage(error, "Memory topics could not be loaded."),
      });
      return false;
    } finally {
      if (listAbortRef.current === controller) {
        listAbortRef.current = null;
      }
    }
  }, [client]);

  useEffect(() => {
    void refreshTopics();
  }, [refreshTopics]);

  useEffect(() => {
    detailGenerationRef.current += 1;
    const generation = detailGenerationRef.current;
    detailAbortRef.current?.abort();
    detailAbortRef.current = null;

    const slug = state.selectedSlug;
    if (slug === null) {
      return;
    }

    const controller = new AbortController();
    detailAbortRef.current = controller;
    dispatch({ type: "detail_request_started", slug });

    void client
      .getMemoryTopic(slug, { signal: controller.signal })
      .then((detail) => {
        if (
          !mountedRef.current ||
          controller.signal.aborted ||
          generation !== detailGenerationRef.current
        ) {
          return;
        }
        dispatch({ type: "detail_loaded", slug, detail });
      })
      .catch((error: unknown) => {
        if (
          !mountedRef.current ||
          controller.signal.aborted ||
          generation !== detailGenerationRef.current
        ) {
          return;
        }
        dispatch({
          type: "detail_load_failed",
          slug,
          error: errorMessage(error, "The memory topic could not be loaded."),
        });
      })
      .finally(() => {
        if (detailAbortRef.current === controller) {
          detailAbortRef.current = null;
        }
      });

    return () => {
      controller.abort();
    };
  }, [client, state.detailRequestVersion, state.selectedSlug]);

  async function confirmDelete(): Promise<void> {
    const slug = state.pendingDeleteSlug;
    if (
      slug === null ||
      state.deleteStatus === "deleting" ||
      deleteAbortRef.current !== null
    ) {
      return;
    }

    // A pre-delete list/detail response must not resurrect the removed topic.
    listGenerationRef.current += 1;
    listAbortRef.current?.abort();
    listAbortRef.current = null;
    detailGenerationRef.current += 1;
    detailAbortRef.current?.abort();
    detailAbortRef.current = null;

    const generation = deleteGenerationRef.current + 1;
    deleteGenerationRef.current = generation;
    const controller = new AbortController();
    deleteAbortRef.current = controller;
    dispatch({ type: "delete_started", slug });

    try {
      await client.deleteMemoryTopic(slug, { signal: controller.signal });
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== deleteGenerationRef.current
      ) {
        return;
      }
      dispatch({ type: "delete_succeeded", slug });
      await refreshTopics();
    } catch (error) {
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== deleteGenerationRef.current
      ) {
        return;
      }
      dispatch({
        type: "delete_failed",
        slug,
        error: errorMessage(error, "The memory topic could not be deleted."),
      });
    } finally {
      if (deleteAbortRef.current === controller) {
        deleteAbortRef.current = null;
      }
    }
  }

  const selectedSummary =
    state.selectedSlug === null
      ? null
      : (state.topics.find(
          (topic) => topic.slug === state.selectedSlug,
        ) ?? null);
  const selectedTopic = state.detail?.topic ?? selectedSummary;
  const listBusy = state.listStatus === "loading";
  const deleteBusy = state.deleteStatus === "deleting";
  const detailBusy = state.detailStatus === "loading";

  return (
    <div className="settings-panel">
      <h1>Memory</h1>
      <p className="lede">
        Durable topics retained by the local runtime across sessions.
      </p>

      <section
        className="settings-card"
        aria-labelledby="memory-topics-heading"
        aria-busy={listBusy}
      >
        <div style={cardHeadingStyle}>
          <div>
            <h2 id="memory-topics-heading">Durable topics</h2>
            {state.topics.length > 0 ? (
              <p style={{ ...mutedTextStyle, marginTop: 4, fontSize: "0.85rem" }}>
                {state.topics.length} {state.topics.length === 1 ? "topic" : "topics"}
              </p>
            ) : null}
          </div>
          <button
            type="button"
            className="secondary"
            disabled={listBusy || deleteBusy}
            onClick={() => void refreshTopics()}
          >
            <ReloadIcon aria-hidden="true" />
            {listBusy && state.topics.length > 0 ? "Refreshing…" : "Refresh"}
          </button>
        </div>

        {state.listError ? (
          <div className="chat-error" role="alert">
            {state.topics.length > 0
              ? `${state.listError} Showing the last loaded topics.`
              : state.listError}
          </div>
        ) : null}

        {listBusy && state.topics.length === 0 ? (
          <div className="placeholder-note" role="status" aria-live="polite">
            Loading durable memory topics…
          </div>
        ) : null}

        {!listBusy && state.listStatus !== "error" && state.topics.length === 0 ? (
          <div className="placeholder-note">No durable memory topics are available.</div>
        ) : null}

        {state.topics.length > 0 ? (
          <ul
            className="profile-list"
            aria-label="Durable memory topics"
            style={{ listStyle: "none", margin: 0, padding: 0 }}
          >
            {state.topics.map((topic) => {
              const selected = topic.slug === state.selectedSlug;
              return (
                <li
                  className="profile-row"
                  key={topic.slug}
                  style={
                    selected
                      ? {
                          borderColor: "var(--accent)",
                          background: "var(--accent-soft)",
                        }
                      : undefined
                  }
                >
                  <div style={{ minWidth: 0 }}>
                    <strong style={{ overflowWrap: "anywhere" }}>
                      {memoryTopicDisplayTitle(topic)}
                    </strong>
                    <span style={{ display: "block", overflowWrap: "anywhere" }}>
                      {topicMetadata(topic)}
                      {topic.metadata_truncated ? " · metadata truncated" : ""}
                    </span>
                  </div>
                  <button
                    type="button"
                    className={selected ? undefined : "secondary"}
                    aria-pressed={selected}
                    disabled={deleteBusy}
                    onClick={() => dispatch({ type: "topic_selected", slug: topic.slug })}
                  >
                    {selected ? "Selected" : "Open"}
                  </button>
                </li>
              );
            })}
          </ul>
        ) : null}
      </section>

      {selectedTopic ? (
        <section
          className="settings-card"
          aria-labelledby="memory-topic-detail-heading"
          aria-busy={detailBusy || deleteBusy}
        >
          <div style={cardHeadingStyle}>
            <div style={{ minWidth: 0 }}>
              <h2 id="memory-topic-detail-heading" style={{ overflowWrap: "anywhere" }}>
                {memoryTopicDisplayTitle(selectedTopic)}
              </h2>
              <p style={{ ...mutedTextStyle, marginTop: 4, fontSize: "0.85rem" }}>
                {selectedTopic.slug}
              </p>
            </div>
            <div className="field-actions">
              <button
                type="button"
                className="secondary"
                disabled={
                  detailBusy || deleteBusy || state.pendingDeleteSlug !== null
                }
                onClick={() => dispatch({ type: "detail_retry_requested" })}
              >
                <ReloadIcon aria-hidden="true" />
                Refresh topic
              </button>
              <button
                type="button"
                className="danger"
                disabled={detailBusy || deleteBusy}
                onClick={() =>
                  dispatch({
                    type: "delete_confirmation_requested",
                    slug: selectedTopic.slug,
                  })
                }
              >
                <TrashIcon aria-hidden="true" />
                Delete topic
              </button>
            </div>
          </div>

          {state.detailStatus === "loading" ? (
            <div className="placeholder-note" role="status" aria-live="polite">
              Loading topic content…
            </div>
          ) : null}

          {state.detailError ? (
            <div className="chat-error" role="alert">
              {state.detailError}
            </div>
          ) : null}

          {state.detailStatus === "ready" && state.detail ? (
            <>
              <div className="inspector-kv" aria-label="Memory topic metadata">
                <div>
                  <span>type</span>
                  <strong>{state.detail.topic.memory_type}</strong>
                </div>
                <div>
                  <span>scope</span>
                  <strong>{state.detail.topic.scope}</strong>
                </div>
                <div>
                  <span>confidence</span>
                  <strong>{Math.round(state.detail.topic.confidence * 100)}%</strong>
                </div>
                <div>
                  <span>created</span>
                  <strong style={{ overflowWrap: "anywhere" }}>
                    {metadataValue(state.detail.topic.created_at)}
                  </strong>
                </div>
                <div>
                  <span>updated</span>
                  <strong style={{ overflowWrap: "anywhere" }}>
                    {metadataValue(state.detail.topic.updated_at)}
                  </strong>
                </div>
              </div>

              <div>
                <h3 style={{ margin: "0 0 6px", fontSize: "0.9rem" }}>Description</h3>
                <p style={mutedTextStyle}>
                  {state.detail.topic.description.trim() || "No description recorded."}
                </p>
              </div>

              {state.detail.topic.metadata_truncated ? (
                <div className="placeholder-note" role="note">
                  Topic metadata was truncated by the API response limit.
                </div>
              ) : null}

              <div>
                <h3 style={{ margin: "0 0 6px", fontSize: "0.9rem" }}>Content</h3>
                {state.detail.content.length > 0 ? (
                  <pre
                    aria-label="Memory topic content"
                    style={{
                      margin: 0,
                      maxHeight: 420,
                      overflow: "auto",
                      whiteSpace: "pre-wrap",
                      overflowWrap: "anywhere",
                      border: "1px solid var(--border)",
                      borderRadius: "var(--radius-md)",
                      background: "var(--surface-soft)",
                      padding: 12,
                      font: "inherit",
                      lineHeight: 1.5,
                    }}
                  >
                    {state.detail.content}
                  </pre>
                ) : (
                  <p style={mutedTextStyle}>No stored body content.</p>
                )}
              </div>

              {state.detail.truncated ? (
                <div className="placeholder-note" role="note">
                  Topic content was truncated by the API response limit.
                </div>
              ) : null}
            </>
          ) : null}

          {state.pendingDeleteSlug === selectedTopic.slug ? (
            <div
              className="placeholder-note"
              role="group"
              aria-labelledby="memory-delete-confirmation-heading"
            >
              <strong id="memory-delete-confirmation-heading">
                Delete “{memoryTopicDisplayTitle(selectedTopic)}”?
              </strong>
              <p style={{ ...mutedTextStyle, marginTop: 6 }}>
                This permanently removes the durable topic. This action cannot be undone.
              </p>
              {state.deleteError ? (
                <div className="chat-error" role="alert" style={{ marginTop: 10 }}>
                  {state.deleteError}
                </div>
              ) : null}
              <div className="field-actions" style={{ marginTop: 10 }}>
                <button
                  type="button"
                  className="secondary"
                  disabled={deleteBusy}
                  onClick={() => dispatch({ type: "delete_confirmation_cancelled" })}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="danger"
                  disabled={deleteBusy}
                  onClick={() => void confirmDelete()}
                >
                  <TrashIcon aria-hidden="true" />
                  {deleteBusy ? "Deleting…" : "Confirm delete"}
                </button>
              </div>
            </div>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}
