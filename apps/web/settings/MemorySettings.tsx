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

import { useCopy } from "../copy/CopyProvider";
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

export function MemorySettings({ client, workspaceId }: MemorySettingsProps) {
  const { t } = useCopy();
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
    } catch {
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== listGenerationRef.current
      ) {
        return false;
      }
      dispatch({
        type: "list_load_failed",
        error: "memory.listFailed",
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
      .catch(() => {
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
          error: "memory.detailFailed",
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
    } catch {
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
        error: "memory.deleteFailed",
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
      setSaveError("memory.contentTruncated");
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
    } catch {
      if (
        !mountedRef.current ||
        controller.signal.aborted ||
        generation !== saveGenerationRef.current
      ) {
        return;
      }
      setSaveError(
        "memory.saveFailed",
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
      <h1>{t("memory.title")}</h1>
      <p className="lede">
        {t("settings.lede")}
      </p>

      <form className="settings-card" onSubmit={applyFilters}>
        <div style={cardHeadingStyle}>
          <h2>{t("memory.find")}</h2>
          {hasActiveFilters ? (
            <button
              type="button"
              className="secondary"
              disabled={listBusy || mutationBusy}
              onClick={clearFilters}
            >
              <Cross2Icon aria-hidden="true" />
              {t("memory.clear")}
            </button>
          ) : null}
        </div>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="memory-search">{t("memory.search")}</label>
            <input
              id="memory-search"
              value={searchDraft}
              disabled={mutationBusy}
              onChange={(event) => setSearchDraft(event.target.value)}
              placeholder={t("memory.searchPlaceholder")}
            />
          </div>
          <div className="field">
            <label htmlFor="memory-type-filter">{t("memory.type")}</label>
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
              <option value="">{t("memory.allTypes")}</option>
              {PRODUCT_MEMORY_TYPES.map((memoryType) => (
                <option key={memoryType} value={memoryType}>
                  {t(`memory.type_${memoryType}`)}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="memory-scope-filter">{t("memory.scopeLabel")}</label>
            <select
              id="memory-scope-filter"
              value={scopeFilter}
              disabled={mutationBusy}
              onChange={(event) =>
                setScopeFilter(event.target.value as ProductMemoryScope | "")
              }
            >
              <option value="">{t("memory.allScopes")}</option>
              {PRODUCT_MEMORY_SCOPES.map((scope) => (
                <option key={scope} value={scope}>
                  {t(`memory.scope_${scope}`)}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="memory-source-filter">{t("memory.source")}</label>
            <select
              id="memory-source-filter"
              value={sourceFilter}
              disabled={mutationBusy}
              onChange={(event) =>
                setSourceFilter(event.target.value as ProductMemorySource | "")
              }
            >
              <option value="">{t("memory.allSources")}</option>
              {PRODUCT_MEMORY_SOURCES.map((source) => (
                <option key={source} value={source}>
                  {t(`memory.source_${source}`)}
                </option>
              ))}
            </select>
          </div>
        </div>
        <div className="field-actions">
          <button type="submit" disabled={listBusy || mutationBusy}>
            <MagnifyingGlassIcon aria-hidden="true" />
            {t("memory.search")}
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
              {editorMode === "create" ? t("memory.newTopic") : t("memory.editTopicTitle")}
            </h2>
            <button
              type="button"
              className="secondary"
              disabled={saving}
              onClick={cancelEditor}
            >
              <Cross2Icon aria-hidden="true" />
              {t("common.cancel")}
            </button>
          </div>
          <form onSubmit={(event) => void saveTopic(event)}>
            <div className="field-grid">
              <div className="field">
                <label htmlFor="memory-editor-slug">{t("memory.slug")}</label>
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
                <label htmlFor="memory-editor-title">{t("memory.topicTitle")}</label>
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
                <label htmlFor="memory-editor-type">{t("memory.type")}</label>
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
                      {t(`memory.type_${memoryType}`)}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="memory-editor-scope">{t("memory.scopeLabel")}</label>
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
                      {t(`memory.scope_${scope}`)}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="memory-editor-confidence">{t("memory.confidence")}</label>
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
                <label htmlFor="memory-editor-description">{t("memory.description")}</label>
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
              <label htmlFor="memory-editor-content">{t("memory.content")}</label>
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
                {t(saveError)}
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
                  ? t("common.saving")
                  : editorMode === "create"
                    ? t("memory.createTopic")
                    : t("memory.saveTopic")}
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
            <h2 id="memory-topics-heading">{t("memory.durableTopics")}</h2>
            {state.topics.length > 0 ? (
              <p style={{ ...mutedTextStyle, marginTop: 4, fontSize: "0.85rem" }}>
                {t("memory.count", { count: state.topics.length })}
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
              {t("memory.newTopic")}
            </button>
            <button
              type="button"
              className="secondary"
              disabled={listBusy || mutationBusy}
              onClick={() => void refreshTopics()}
            >
              <ReloadIcon aria-hidden="true" />
              {listBusy && state.topics.length > 0 ? t("memory.refreshing") : t("memory.refresh")}
            </button>
          </div>
        </div>

        {state.listError ? (
          <div className="chat-error" role="alert">
            {state.topics.length > 0
              ? `${t(state.listError)} ${t("memory.lastLoaded")}`
              : t(state.listError)}
          </div>
        ) : null}

        {listBusy && state.topics.length === 0 ? (
          <div className="placeholder-note" role="status" aria-live="polite">
            {t("memory.loading")}
          </div>
        ) : null}

        {!listBusy && state.listStatus !== "error" && state.topics.length === 0 ? (
          <div className="placeholder-note">
            {hasActiveFilters
              ? t("memory.noMatch")
              : t("memory.empty")}
          </div>
        ) : null}

        {state.topics.length > 0 ? (
          <ul
            className="profile-list"
            aria-label={t("memory.listLabel")}
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
                      {t("memory.summary", { type: t(`memory.type_${topic.memory_type}`), scope: t(`memory.scope_${topic.scope}`), confidence: Math.round(topic.confidence * 100) })}
                      {` · ${t(`memory.source_${topic.source}`)}`}
                      {topic.metadata_truncated ? ` · ${t("memory.metadataTruncated")}` : ""}
                    </span>
                  </div>
                  <button
                    type="button"
                    className={selected ? undefined : "secondary"}
                    aria-pressed={selected}
                    disabled={mutationBusy}
                    onClick={() => dispatch({ type: "topic_selected", slug: topic.slug })}
                  >
                    {selected ? t("memory.selected") : t("memory.open")}
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
                {t("memory.refreshTopic")}
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
                {t("memory.editTopic")}
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
                {t("memory.deleteTopic")}
              </button>
            </div>
          </div>

          {state.detailStatus === "loading" ? (
            <div className="placeholder-note" role="status" aria-live="polite">
              {t("memory.loadingContent")}
            </div>
          ) : null}

          {state.detailError ? (
            <div className="chat-error" role="alert">
              {t(state.detailError)}
            </div>
          ) : null}

          {state.detailStatus === "ready" && state.detail ? (
            <>
              <div className="inspector-kv" aria-label={t("memory.metadata")}>
                <div>
                  <span>{t("memory.layer")}</span>
                  <strong>{t("memory.durable")}</strong>
                </div>
                <div>
                  <span>{t("memory.type")}</span>
                  <strong>{t(`memory.type_${state.detail.topic.memory_type}`)}</strong>
                </div>
                <div>
                  <span>{t("memory.scope")}</span>
                  <strong>{t(`memory.scope_${state.detail.topic.scope}`)}</strong>
                </div>
                <div>
                  <span>{t("memory.source")}</span>
                  <strong>{t(`memory.source_${state.detail.topic.source}`)}</strong>
                </div>
                <div>
                  <span>{t("memory.confidence")}</span>
                  <strong>{Math.round(state.detail.topic.confidence * 100)}%</strong>
                </div>
                <div>
                  <span>{t("memory.created")}</span>
                  <strong style={{ overflowWrap: "anywhere" }}>
                    {state.detail.topic.created_at?.trim() || t("memory.notRecorded")}
                  </strong>
                </div>
                <div>
                  <span>{t("memory.updated")}</span>
                  <strong style={{ overflowWrap: "anywhere" }}>
                    {state.detail.topic.updated_at?.trim() || t("memory.notRecorded")}
                  </strong>
                </div>
              </div>

              <div>
                <h3 style={{ margin: "0 0 6px", fontSize: "0.9rem" }}>{t("memory.description")}</h3>
                <p style={mutedTextStyle}>
                  {state.detail.topic.description.trim() || t("memory.noDescription")}
                </p>
              </div>

              {state.detail.topic.metadata_truncated ? (
                <div className="placeholder-note" role="note">
                  {t("memory.metadataTruncated")}
                </div>
              ) : null}

              <div>
                <h3 style={{ margin: "0 0 6px", fontSize: "0.9rem" }}>{t("memory.content")}</h3>
                {state.detail.content.length > 0 ? (
                  <pre
                    aria-label={t("memory.contentLabel")}
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
                  <p style={mutedTextStyle}>{t("memory.emptyBody")}</p>
                )}
              </div>

              {state.detail.truncated ? (
                <div className="placeholder-note" role="note">
                  {t("memory.contentTruncated")}
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
                {t("memory.deleteConfirm", { title: memoryTopicDisplayTitle(selectedTopic) })}
              </strong>
              <p style={{ ...mutedTextStyle, marginTop: 6 }}>
                {t("memory.deleteWarning")}
              </p>
              {state.deleteError ? (
                <div className="chat-error" role="alert" style={{ marginTop: 10 }}>
                  {t(state.deleteError)}
                </div>
              ) : null}
              <div className="field-actions" style={{ marginTop: 10 }}>
                <button
                  type="button"
                  className="secondary"
                  disabled={mutationBusy}
                  onClick={() => dispatch({ type: "delete_confirmation_cancelled" })}
                >
                  {t("common.cancel")}
                </button>
                <button
                  type="button"
                  className="danger"
                  disabled={mutationBusy}
                  onClick={() => void confirmDelete()}
                >
                  <TrashIcon aria-hidden="true" />
                  {deleteBusy ? t("memory.deleting") : t("memory.confirmDelete")}
                </button>
              </div>
            </div>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}
