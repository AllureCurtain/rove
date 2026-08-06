"use client";

import {
  Cross2Icon,
  MagnifyingGlassIcon,
  Pencil2Icon,
  PlusIcon,
  ReloadIcon,
  TrashIcon,
} from "@radix-ui/react-icons";
import {
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
} from "react";

import {
  createInitialMemorySettingsState,
  memorySettingsReducer,
  memoryTopicDisplayTitle,
} from "./memory-settings-model";
import {
  PRODUCT_MEMORY_SCOPES,
  PRODUCT_MEMORY_SOURCES,
  PRODUCT_MEMORY_TYPES,
  type CreateProductMemoryTopicRequest,
  type ProductMemoryListFilters,
  type ProductMemoryScope,
  type ProductMemorySource,
  type ProductMemoryTopic,
  type ProductMemoryTopicContentResponse,
  type ProductMemoryType,
  type UpdateProductMemoryTopicRequest,
} from "./settings-platform-api-types";
import type { SettingsPlatformClient } from "./settings-platform-client";

export interface MemorySettingsProps {
  client: SettingsPlatformClient;
  workspaceId: string;
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

type MemoryEditorMode = "create" | "edit";

export interface MemoryTopicDraft {
  slug: string;
  title: string;
  memoryType: ProductMemoryType;
  scope: ProductMemoryScope;
  confidence: string;
  description: string;
  content: string;
  expectedUpdatedAt?: string;
}

export function createEmptyMemoryTopicDraft(): MemoryTopicDraft {
  return {
    slug: "",
    title: "",
    memoryType: "project",
    scope: "project",
    confidence: "0.8",
    description: "",
    content: "",
  };
}

export function memoryTopicDraftFromDetail(
  detail: ProductMemoryTopicContentResponse,
): MemoryTopicDraft {
  const draft: MemoryTopicDraft = {
    slug: detail.topic.slug,
    title: detail.topic.title,
    memoryType: detail.topic.memory_type,
    scope: detail.topic.scope,
    confidence: String(detail.topic.confidence),
    description: detail.topic.description,
    content: detail.content,
  };
  if (detail.topic.updated_at !== undefined) {
    draft.expectedUpdatedAt = detail.topic.updated_at;
  }
  return draft;
}

function createRequestFromDraft(
  draft: MemoryTopicDraft,
): CreateProductMemoryTopicRequest {
  return {
    slug: draft.slug,
    title: draft.title,
    memory_type: draft.memoryType,
    scope: draft.scope,
    confidence: Number(draft.confidence),
    description: draft.description,
    content: draft.content,
  };
}

function updateRequestFromDraft(
  draft: MemoryTopicDraft,
): UpdateProductMemoryTopicRequest {
  const request: UpdateProductMemoryTopicRequest = {
    title: draft.title,
    memory_type: draft.memoryType,
    scope: draft.scope,
    confidence: Number(draft.confidence),
    description: draft.description,
    content: draft.content,
  };
  if (draft.expectedUpdatedAt !== undefined) {
    request.expected_updated_at = draft.expectedUpdatedAt;
  }
  return request;
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  return fallback;
}

function topicMetadata(topic: ProductMemoryTopic): string {
  const confidence = `${Math.round(topic.confidence * 100)}% confidence`;
  return `Durable · ${topic.memory_type} · ${topic.scope} scope · ${confidence}`;
}

function sourceLabel(source: ProductMemorySource): string {
  switch (source) {
    case "product_settings":
      return "Settings";
    case "llm_tool":
      return "Agent tool";
    case "other":
      return "Other";
    case "unknown":
      return "Unknown";
  }
}

function metadataValue(value: string | undefined): string {
  return value?.trim() || "Not recorded";
}

export function MemorySettings({ client, workspaceId }: MemorySettingsProps) {
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
  const saveGenerationRef = useRef(0);
  const listAbortRef = useRef<AbortController | null>(null);
  const detailAbortRef = useRef<AbortController | null>(null);
  const deleteAbortRef = useRef<AbortController | null>(null);
  const saveAbortRef = useRef<AbortController | null>(null);
  const [searchDraft, setSearchDraft] = useState("");
  const [memoryTypeFilter, setMemoryTypeFilter] = useState<
    ProductMemoryType | ""
  >(
    "",
  );
  const [scopeFilter, setScopeFilter] = useState<ProductMemoryScope | "">("");
  const [sourceFilter, setSourceFilter] = useState<ProductMemorySource | "">(
    "",
  );
  const [activeFilters, setActiveFilters] = useState<ProductMemoryListFilters>(
    {},
  );
  const [editorMode, setEditorMode] = useState<MemoryEditorMode | null>(null);
  const [draft, setDraft] = useState<MemoryTopicDraft>(
    createEmptyMemoryTopicDraft,
  );
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      listGenerationRef.current += 1;
      detailGenerationRef.current += 1;
      deleteGenerationRef.current += 1;
      saveGenerationRef.current += 1;
      listAbortRef.current?.abort();
      detailAbortRef.current?.abort();
      deleteAbortRef.current?.abort();
      saveAbortRef.current?.abort();
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
    saveGenerationRef.current += 1;
    saveAbortRef.current?.abort();
    saveAbortRef.current = null;
    setSaving(false);
    setSaveError(null);
    setEditorMode(null);
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
      const response = await client.listMemoryTopics(
        workspaceId,
        activeFilters,
        { signal: controller.signal },
      );
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
  }, [activeFilters, client, workspaceId]);

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
      .getMemoryTopic(workspaceId, slug, { signal: controller.signal })
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
  }, [client, state.detailRequestVersion, state.selectedSlug, workspaceId]);

  async function confirmDelete(): Promise<void> {
    const slug = state.pendingDeleteSlug;
    if (
      slug === null ||
      state.deleteStatus === "deleting" ||
      saving ||
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
      await client.deleteMemoryTopic(workspaceId, slug, {
        signal: controller.signal,
      });
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

  function applyFilters(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault();
    const filters: ProductMemoryListFilters = {};
    const q = searchDraft.trim();
    if (q.length > 0) {
      filters.q = q;
    }
    if (memoryTypeFilter !== "") {
      filters.memory_type = memoryTypeFilter;
    }
    if (scopeFilter !== "") {
      filters.scope = scopeFilter;
    }
    if (sourceFilter !== "") {
      filters.source = sourceFilter;
    }
    setActiveFilters(filters);
  }

  function clearFilters(): void {
    setSearchDraft("");
    setMemoryTypeFilter("");
    setScopeFilter("");
    setSourceFilter("");
    setActiveFilters({});
  }

  function startCreate(): void {
    setDraft(createEmptyMemoryTopicDraft());
    setSaveError(null);
    setEditorMode("create");
    dispatch({ type: "delete_reset" });
  }

  function startEdit(): void {
    if (state.detailStatus !== "ready" || state.detail === null) {
      return;
    }
    if (state.detail.truncated) {
      setSaveError("Reload the complete topic before editing it.");
      return;
    }
    setDraft(memoryTopicDraftFromDetail(state.detail));
    setSaveError(null);
    setEditorMode("edit");
    dispatch({ type: "delete_reset" });
  }

  function cancelEditor(): void {
    if (saving) {
      return;
    }
    setEditorMode(null);
    setSaveError(null);
  }

  async function saveTopic(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    if (
      editorMode === null ||
      saving ||
      state.deleteStatus === "deleting" ||
      saveAbortRef.current !== null
    ) {
      return;
    }

    listGenerationRef.current += 1;
    listAbortRef.current?.abort();
    listAbortRef.current = null;
    detailGenerationRef.current += 1;
    detailAbortRef.current?.abort();
    detailAbortRef.current = null;

    const generation = saveGenerationRef.current + 1;
    saveGenerationRef.current = generation;
    const controller = new AbortController();
    saveAbortRef.current = controller;
    setSaving(true);
    setSaveError(null);

    try {
      const response =
        editorMode === "create"
          ? await client.createMemoryTopic(
              workspaceId,
              createRequestFromDraft(draft),
              { signal: controller.signal },
            )
          : await client.updateMemoryTopic(
              workspaceId,
              draft.slug,
              updateRequestFromDraft(draft),
              { signal: controller.signal },
            );
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== saveGenerationRef.current
      ) {
        return;
      }
      dispatch({ type: "topic_saved", detail: response });
      setEditorMode(null);
      setSaveError(null);
      await refreshTopics();
    } catch (error) {
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== saveGenerationRef.current
      ) {
        return;
      }
      setSaveError(
        errorMessage(
          error,
          editorMode === "create"
            ? "The memory topic could not be created."
            : "The memory topic could not be updated.",
        ),
      );
    } finally {
      if (saveAbortRef.current === controller) {
        saveAbortRef.current = null;
      }
      if (mountedRef.current && generation === saveGenerationRef.current) {
        setSaving(false);
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
  const mutationBusy = deleteBusy || saving;
  const hasActiveFilters = Object.keys(activeFilters).length > 0;

  return (
    <div className="settings-panel">
      <h1>Memory</h1>
      <p className="lede">
        Durable topics retained by the local runtime across sessions.
      </p>

      <form className="settings-card" onSubmit={applyFilters}>
        <div style={cardHeadingStyle}>
          <h2>Find topics</h2>
          {hasActiveFilters ? (
            <button
              type="button"
              className="secondary"
              disabled={listBusy || mutationBusy}
              onClick={clearFilters}
            >
              <Cross2Icon aria-hidden="true" />
              Clear
            </button>
          ) : null}
        </div>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="memory-search">Search</label>
            <input
              id="memory-search"
              value={searchDraft}
              disabled={mutationBusy}
              onChange={(event) => setSearchDraft(event.target.value)}
              placeholder="Title, slug, or description"
            />
          </div>
          <div className="field">
            <label htmlFor="memory-type-filter">Type</label>
            <select
              id="memory-type-filter"
              value={memoryTypeFilter}
              disabled={mutationBusy}
              onChange={(event) =>
                setMemoryTypeFilter(
                  event.target.value as ProductMemoryType | "",
                )
              }
            >
              <option value="">All types</option>
              {PRODUCT_MEMORY_TYPES.map((memoryType) => (
                <option key={memoryType} value={memoryType}>
                  {memoryType}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="memory-scope-filter">Durable scope</label>
            <select
              id="memory-scope-filter"
              value={scopeFilter}
              disabled={mutationBusy}
              onChange={(event) =>
                setScopeFilter(event.target.value as ProductMemoryScope | "")
              }
            >
              <option value="">All scopes</option>
              {PRODUCT_MEMORY_SCOPES.map((scope) => (
                <option key={scope} value={scope}>
                  {scope}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="memory-source-filter">Source</label>
            <select
              id="memory-source-filter"
              value={sourceFilter}
              disabled={mutationBusy}
              onChange={(event) =>
                setSourceFilter(event.target.value as ProductMemorySource | "")
              }
            >
              <option value="">All sources</option>
              {PRODUCT_MEMORY_SOURCES.map((source) => (
                <option key={source} value={source}>
                  {sourceLabel(source)}
                </option>
              ))}
            </select>
          </div>
        </div>
        <div className="field-actions">
          <button type="submit" disabled={listBusy || mutationBusy}>
            <MagnifyingGlassIcon aria-hidden="true" />
            Search
          </button>
        </div>
      </form>

      {editorMode ? (
        <section
          className="settings-card"
          aria-labelledby="memory-editor-heading"
          aria-busy={saving}
        >
          <div style={cardHeadingStyle}>
            <h2 id="memory-editor-heading">
              {editorMode === "create" ? "New durable topic" : "Edit durable topic"}
            </h2>
            <button
              type="button"
              className="secondary"
              disabled={saving}
              onClick={cancelEditor}
            >
              <Cross2Icon aria-hidden="true" />
              Cancel
            </button>
          </div>
          <form onSubmit={(event) => void saveTopic(event)}>
            <div className="field-grid">
              <div className="field">
                <label htmlFor="memory-editor-slug">Slug</label>
                <input
                  id="memory-editor-slug"
                  value={draft.slug}
                  required
                  readOnly={editorMode === "edit"}
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      slug: event.target.value,
                    }))
                  }
                />
              </div>
              <div className="field">
                <label htmlFor="memory-editor-title">Title</label>
                <input
                  id="memory-editor-title"
                  value={draft.title}
                  required
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      title: event.target.value,
                    }))
                  }
                />
              </div>
              <div className="field">
                <label htmlFor="memory-editor-type">Type</label>
                <select
                  id="memory-editor-type"
                  value={draft.memoryType}
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      memoryType: event.target.value as ProductMemoryType,
                    }))
                  }
                >
                  {PRODUCT_MEMORY_TYPES.map((memoryType) => (
                    <option key={memoryType} value={memoryType}>
                      {memoryType}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="memory-editor-scope">Durable scope</label>
                <select
                  id="memory-editor-scope"
                  value={draft.scope}
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      scope: event.target.value as ProductMemoryScope,
                    }))
                  }
                >
                  {PRODUCT_MEMORY_SCOPES.map((scope) => (
                    <option key={scope} value={scope}>
                      {scope}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="memory-editor-confidence">Confidence</label>
                <input
                  id="memory-editor-confidence"
                  type="number"
                  min="0"
                  max="1"
                  step="0.05"
                  value={draft.confidence}
                  required
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      confidence: event.target.value,
                    }))
                  }
                />
              </div>
              <div className="field">
                <label htmlFor="memory-editor-description">Description</label>
                <input
                  id="memory-editor-description"
                  value={draft.description}
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      description: event.target.value,
                    }))
                  }
                />
              </div>
            </div>
            <div className="field" style={{ marginTop: 12 }}>
              <label htmlFor="memory-editor-content">Content</label>
              <textarea
                id="memory-editor-content"
                rows={10}
                value={draft.content}
                disabled={saving}
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    content: event.target.value,
                  }))
                }
              />
            </div>
            {saveError ? (
              <div className="chat-error" role="alert" style={{ marginTop: 12 }}>
                {saveError}
              </div>
            ) : null}
            <div className="field-actions" style={{ marginTop: 12 }}>
              <button type="submit" disabled={saving || deleteBusy}>
                {editorMode === "create" ? (
                  <PlusIcon aria-hidden="true" />
                ) : (
                  <Pencil2Icon aria-hidden="true" />
                )}
                {saving
                  ? "Saving…"
                  : editorMode === "create"
                    ? "Create topic"
                    : "Save changes"}
              </button>
            </div>
          </form>
        </section>
      ) : null}

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
          <div className="field-actions">
            <button
              type="button"
              disabled={mutationBusy}
              onClick={startCreate}
            >
              <PlusIcon aria-hidden="true" />
              New topic
            </button>
            <button
              type="button"
              className="secondary"
              disabled={listBusy || mutationBusy}
              onClick={() => void refreshTopics()}
            >
              <ReloadIcon aria-hidden="true" />
              {listBusy && state.topics.length > 0 ? "Refreshing…" : "Refresh"}
            </button>
          </div>
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
          <div className="placeholder-note">
            {hasActiveFilters
              ? "No durable memory topics match these filters."
              : "No durable memory topics are available."}
          </div>
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
                      {` · ${sourceLabel(topic.source)}`}
                      {topic.metadata_truncated ? " · metadata truncated" : ""}
                    </span>
                  </div>
                  <button
                    type="button"
                    className={selected ? undefined : "secondary"}
                    aria-pressed={selected}
                    disabled={mutationBusy}
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
                  detailBusy || mutationBusy || state.pendingDeleteSlug !== null
                }
                onClick={() => dispatch({ type: "detail_retry_requested" })}
              >
                <ReloadIcon aria-hidden="true" />
                Refresh topic
              </button>
              <button
                type="button"
                className="secondary"
                disabled={
                  detailBusy ||
                  mutationBusy ||
                  state.detailStatus !== "ready" ||
                  state.detail === null ||
                  state.detail.truncated
                }
                onClick={startEdit}
              >
                <Pencil2Icon aria-hidden="true" />
                Edit topic
              </button>
              <button
                type="button"
                className="danger"
                disabled={detailBusy || mutationBusy}
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
                  <span>layer</span>
                  <strong>{state.detail.topic.layer}</strong>
                </div>
                <div>
                  <span>type</span>
                  <strong>{state.detail.topic.memory_type}</strong>
                </div>
                <div>
                  <span>scope</span>
                  <strong>{state.detail.topic.scope}</strong>
                </div>
                <div>
                  <span>source</span>
                  <strong>{sourceLabel(state.detail.topic.source)}</strong>
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
                  Topic content was truncated by the API response limit. Editing is
                  disabled until a complete response is available.
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
                  disabled={mutationBusy}
                  onClick={() => dispatch({ type: "delete_confirmation_cancelled" })}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="danger"
                  disabled={mutationBusy}
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
