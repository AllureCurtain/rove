"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { describeError } from "../api/run-controller";
import {
  cacheServerConfirmedTheme,
  readServerConfirmedTheme,
} from "../platform/server-theme-cache";
import { applyDocumentTheme } from "../platform/web";
import type {
  ProductApprovalPreference,
  ProductPreferences,
} from "../product/product-api-types";
import {
  createProductApiClient,
  ProductApiError,
} from "../product/product-client";
import {
  isAbsoluteWorkspacePath,
  productCatalogFromApi,
  replaceServerSessions,
  updateSession,
  type ProductCatalog,
} from "./product-catalog";
import {
  listSessionsBounded,
  mergeWorkspaceSnapshot,
  resolveProductTheme,
  selectionFromPreferences,
  toPreferencesRequest,
} from "./server-product-state";
import type {
  ActiveProviderSelection,
  ProviderProfileInput,
  ProviderProfileRecord,
  SessionRecord,
  SessionModelConfig,
  SessionModelConfigInput,
  WorkspaceKind,
} from "./product-types";
import {
  fromProductProviderProfile,
  fromProductSession,
  fromProductSessionModelConfig,
  fromProductWorkspace,
  newId,
} from "./product-types";

export type ProductBootState =
  | { status: "loading" }
  | { status: "ready" }
  | { status: "error"; error: string };

const EMPTY_CATALOG: ProductCatalog = {
  workspaces: [],
  sessions: [],
  active: { workspaceId: null, sessionId: null },
};

export function useServerProductState() {
  const productClient = useMemo(() => createProductApiClient(), []);
  const [catalog, setCatalog] = useState<ProductCatalog>(EMPTY_CATALOG);
  const [preferences, setPreferences] = useState<ProductPreferences | null>(null);
  const [profiles, setProfiles] = useState<ProviderProfileRecord[]>([]);
  const [sessionModelConfig, setSessionModelConfig] =
    useState<SessionModelConfig | null>(null);
  const [sessionModelConfigLoading, setSessionModelConfigLoading] =
    useState(false);
  const [sessionModelConfigMutationBusy, setSessionModelConfigMutationBusy] =
    useState(false);
  const [selection, setSelection] = useState<ActiveProviderSelection>(() => ({
    mode: "default",
    model: "fake",
    approval: "ask",
    maxSteps: 8,
  }));
  const [theme, setTheme] = useState<"light" | "dark">(
    readServerConfirmedTheme,
  );
  const [bootState, setBootState] = useState<ProductBootState>({ status: "loading" });
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogMutationBusy, setCatalogMutationBusy] = useState(false);
  const [preferencesMutationBusy, setPreferencesMutationBusy] = useState(false);
  const [connection, setConnection] = useState<"unknown" | "ok" | "error">(
    "unknown",
  );
  const catalogRef = useRef(catalog);
  const preferencesRef = useRef(preferences);
  const sessionModelConfigRef = useRef(sessionModelConfig);
  const confirmedPreferencesRef = useRef<ProductPreferences | null>(null);
  const bootGenerationRef = useRef(0);
  const catalogGenerationRef = useRef(0);
  const preferencesGenerationRef = useRef(0);
  const sessionModelConfigGenerationRef = useRef(0);
  const mutationGenerationRef = useRef(0);
  const sessionUpdateGenerationsRef = useRef(new Map<string, number>());
  const catalogMutationRef = useRef(false);
  const preferencesQueueRef = useRef<Promise<void>>(Promise.resolve());
  const deletingProviderProfileIdsRef = useRef(new Set<string>());
  const providerCatalogRevisionRef = useRef<string | null>(null);
  const failedActiveRouteTargetRef = useRef<string | null>(null);
  const forkIdempotencyRef = useRef(new Map<string, string>());

  useEffect(() => {
    catalogRef.current = catalog;
  }, [catalog]);

  useEffect(() => {
    preferencesRef.current = preferences;
  }, [preferences]);

  useEffect(() => {
    sessionModelConfigRef.current = sessionModelConfig;
  }, [sessionModelConfig]);

  useEffect(() => {
    applyDocumentTheme(theme);
  }, [theme]);

  const patchCatalog = useCallback(
    (updater: (current: ProductCatalog) => ProductCatalog) => {
      setCatalog((current) => {
        const next = updater(current);
        catalogRef.current = next;
        return next;
      });
    },
    [],
  );

  const beginCatalogMutation = useCallback((): number | null => {
    if (catalogMutationRef.current) {
      return null;
    }
    catalogMutationRef.current = true;
    setCatalogMutationBusy(true);
    ++catalogGenerationRef.current;
    setCatalogError(null);
    return ++mutationGenerationRef.current;
  }, []);

  const finishCatalogMutation = useCallback((generation: number) => {
    if (mutationGenerationRef.current === generation) {
      catalogMutationRef.current = false;
      setCatalogMutationBusy(false);
    }
  }, []);

  const persistPreferences = useCallback(
    (next: ProductPreferences) => {
      preferencesRef.current = next;
      setPreferences(next);
      setPreferencesMutationBusy(true);
      const generation = ++preferencesGenerationRef.current;
      const operation = preferencesQueueRef.current
        .catch(() => undefined)
        .then(() => {
          const confirmed = confirmedPreferencesRef.current;
          const expectedRevision = confirmed?.revision ?? next.revision;
          return productClient.updatePreferences(
            toPreferencesRequest({ ...next, revision: expectedRevision }),
          );
        });
      preferencesQueueRef.current = operation
        .then((saved) => {
          confirmedPreferencesRef.current = saved;
          if (preferencesGenerationRef.current !== generation) {
            return;
          }
          preferencesRef.current = saved;
          setPreferences(saved);
          setSelection(selectionFromPreferences(saved));
          const confirmedTheme = resolveProductTheme(saved.theme);
          cacheServerConfirmedTheme(confirmedTheme);
          setTheme(confirmedTheme);
          setCatalogError(null);
        })
        .catch(async (error) => {
          if (
            error instanceof ProductApiError &&
            error.code === "product_revision_conflict"
          ) {
            try {
              const current = await productClient.getPreferences();
              confirmedPreferencesRef.current = current;
              if (preferencesGenerationRef.current === generation) {
                preferencesRef.current = current;
                setPreferences(current);
                setSelection(selectionFromPreferences(current));
                const confirmedTheme = resolveProductTheme(current.theme);
                cacheServerConfirmedTheme(confirmedTheme);
                setTheme(confirmedTheme);
              }
            } catch {
              // Keep the last confirmed snapshot when conflict recovery cannot read.
            }
          }
          if (preferencesGenerationRef.current === generation) {
            const confirmed = confirmedPreferencesRef.current;
            preferencesRef.current = confirmed;
            setPreferences(confirmed);
            if (confirmed) {
              setSelection(selectionFromPreferences(confirmed));
              const confirmedTheme = resolveProductTheme(confirmed.theme);
              cacheServerConfirmedTheme(confirmedTheme);
              setTheme(confirmedTheme);
            }
            setCatalogError(`Could not persist preferences: ${describeError(error)}`);
          }
        })
        .finally(() => {
          if (preferencesGenerationRef.current === generation) {
            setPreferencesMutationBusy(false);
          }
        });
      return operation;
    },
    [productClient],
  );

  const queuePreferences = useCallback(
    (next: ProductPreferences) => {
      void persistPreferences(next).catch(() => undefined);
    },
    [persistPreferences],
  );

  const loadInitialState = useCallback(async () => {
    const bootGeneration = ++bootGenerationRef.current;
    const catalogGeneration = ++catalogGenerationRef.current;
    const preferencesGeneration = ++preferencesGenerationRef.current;
    ++sessionModelConfigGenerationRef.current;
    sessionModelConfigRef.current = null;
    setSessionModelConfig(null);
    setSessionModelConfigLoading(false);
    setBootState({ status: "loading" });
    setCatalogError(null);

    try {
      const [workspaceResponse, loadedPreferences, profileResponse] = await Promise.all([
        productClient.listWorkspaces(),
        productClient.getPreferences(),
        productClient.listProviderProfiles(),
      ]);
      const sessions = await listSessionsBounded(
        productClient,
        workspaceResponse.workspaces.map((workspace) => workspace.id),
      );
      if (
        bootGenerationRef.current !== bootGeneration ||
        catalogGenerationRef.current !== catalogGeneration ||
        preferencesGenerationRef.current !== preferencesGeneration
      ) {
        return;
      }
      const nextCatalog = productCatalogFromApi(
        workspaceResponse.workspaces,
        sessions,
        loadedPreferences,
      );
      catalogRef.current = nextCatalog;
      preferencesRef.current = loadedPreferences;
      confirmedPreferencesRef.current = loadedPreferences;
      failedActiveRouteTargetRef.current = null;
      setCatalog(nextCatalog);
      setPreferences(loadedPreferences);
      setProfiles(
        profileResponse.provider_profiles.map(fromProductProviderProfile),
      );
      providerCatalogRevisionRef.current = profileResponse.catalog_revision;
      setSelection(selectionFromPreferences(loadedPreferences));
      const confirmedTheme = resolveProductTheme(loadedPreferences.theme);
      cacheServerConfirmedTheme(confirmedTheme);
      setTheme(confirmedTheme);
      setConnection("ok");
      setPreferencesMutationBusy(false);
      setBootState({ status: "ready" });
    } catch (error) {
      if (bootGenerationRef.current !== bootGeneration) {
        return;
      }
      setConnection("error");
      setBootState({ status: "error", error: describeError(error) });
    }
  }, [productClient]);

  const loadSessionModelConfig = useCallback(
    async (sessionId: string | null) => {
      const generation = ++sessionModelConfigGenerationRef.current;
      if (!sessionId) {
        sessionModelConfigRef.current = null;
        setSessionModelConfig(null);
        setSessionModelConfigLoading(false);
        return;
      }
      if (sessionModelConfigRef.current?.sessionId !== sessionId) {
        sessionModelConfigRef.current = null;
        setSessionModelConfig(null);
      }
      setSessionModelConfigLoading(true);
      try {
        const saved = await productClient.getSessionModelConfig(sessionId);
        if (
          sessionModelConfigGenerationRef.current !== generation ||
          catalogRef.current.active.sessionId !== sessionId
        ) {
          return;
        }
        const record = fromProductSessionModelConfig(saved);
        sessionModelConfigRef.current = record;
        setSessionModelConfig(record);
        setConnection("ok");
      } catch (error) {
        if (
          sessionModelConfigGenerationRef.current !== generation ||
          catalogRef.current.active.sessionId !== sessionId
        ) {
          return;
        }
        sessionModelConfigRef.current = null;
        setSessionModelConfig(null);
        setCatalogError(`Could not load session model settings: ${describeError(error)}`);
        setConnection("error");
      } finally {
        if (sessionModelConfigGenerationRef.current === generation) {
          setSessionModelConfigLoading(false);
        }
      }
    },
    [productClient],
  );

  useEffect(() => {
    void loadInitialState();
    return () => {
      ++bootGenerationRef.current;
      ++catalogGenerationRef.current;
      ++preferencesGenerationRef.current;
      ++sessionModelConfigGenerationRef.current;
      ++mutationGenerationRef.current;
      sessionUpdateGenerationsRef.current.clear();
    };
  }, [loadInitialState]);

  useEffect(() => {
    if (bootState.status !== "ready") {
      return;
    }
    void loadSessionModelConfig(catalog.active.sessionId);
  }, [bootState.status, catalog.active.sessionId, loadSessionModelConfig]);

  const refreshSessionStatuses = useCallback(async () => {
    if (catalogMutationRef.current) {
      return false;
    }
    const workspaceIds = catalogRef.current.workspaces.map((workspace) => workspace.id);
    if (workspaceIds.length === 0) {
      return false;
    }
    const generation = ++catalogGenerationRef.current;
    try {
      const sessions = await listSessionsBounded(productClient, workspaceIds);
      if (
        catalogGenerationRef.current !== generation ||
        catalogMutationRef.current
      ) {
        return false;
      }
      patchCatalog((current) =>
        replaceServerSessions(current, workspaceIds, sessions),
      );
      return true;
    } catch (error) {
      if (catalogGenerationRef.current === generation) {
        setCatalogError(`Could not refresh session status: ${describeError(error)}`);
      }
      return false;
    }
  }, [patchCatalog, productClient]);

  useEffect(() => {
    if (
      bootState.status !== "ready" ||
      !catalog.sessions.some(
        (session) =>
          session.status === "running" || session.status === "needs_attention",
      )
    ) {
      return;
    }
    const interval = window.setInterval(() => {
      void refreshSessionStatuses();
    }, 2_500);
    return () => window.clearInterval(interval);
  }, [bootState.status, catalog.sessions, refreshSessionStatuses]);

  const persistActiveRoute = useCallback(
    (workspaceId: string, sessionId: string | undefined) => {
      const target = `${workspaceId}\u0000${sessionId ?? ""}`;
      if (failedActiveRouteTargetRef.current === target) {
        return;
      }
      if (
        failedActiveRouteTargetRef.current &&
        failedActiveRouteTargetRef.current !== target
      ) {
        failedActiveRouteTargetRef.current = null;
      }
      const current = preferencesRef.current;
      if (
        !current ||
        (current.active_workspace_id === workspaceId &&
          current.active_session_id === sessionId)
      ) {
        return;
      }
      failedActiveRouteTargetRef.current = target;
      void persistPreferences({
        ...current,
        active_workspace_id: workspaceId,
        active_session_id: sessionId,
      })
        .then(() => {
          if (failedActiveRouteTargetRef.current === target) {
            failedActiveRouteTargetRef.current = null;
          }
        })
        .catch(() => undefined);
    },
    [persistPreferences],
  );

  const selectCatalogRoute = useCallback(
    (workspaceId: string | null, sessionId: string | null) => {
      patchCatalog((current) => ({
        ...current,
        active: { workspaceId, sessionId },
      }));
    },
    [patchCatalog],
  );

  const markSession = useCallback(
    (sessionId: string, patch: Parameters<typeof updateSession>[2]) => {
      patchCatalog((current) => updateSession(current, sessionId, patch));
    },
    [patchCatalog],
  );

  const openWorkspace = useCallback(
    async (path: string, kind: WorkspaceKind) => {
      if (!isAbsoluteWorkspacePath(path)) {
        setCatalogError("Workspace path must be an absolute local directory.");
        return null;
      }
      if (kind === "task") {
        setCatalogError("The product catalog currently supports Folder or Repo workspaces.");
        return null;
      }
      const mutation = beginCatalogMutation();
      if (mutation === null) {
        return null;
      }
      try {
        const workspace = await productClient.createWorkspace({
          root: path,
          kind,
          pinned: false,
        });
        // A workspace that was just created holds at most a handful of adopted
        // sessions, and we only need one to open, so a single page suffices.
        let sessionResponse = await productClient.listSessions(workspace.id, {
          includeArchived: false,
        });
        let session = sessionResponse.sessions.find((item) => item.status !== "archived");
        if (!session) {
          session = await productClient.createSession({ workspace_id: workspace.id });
          sessionResponse = { sessions: [session] };
        }
        if (mutationGenerationRef.current !== mutation) {
          return null;
        }
        patchCatalog((current) =>
          mergeWorkspaceSnapshot(current, workspace, sessionResponse.sessions),
        );
        return { workspaceId: workspace.id, sessionId: session.id };
      } catch (error) {
        if (mutationGenerationRef.current === mutation) {
          setCatalogError(describeError(error));
        }
        return null;
      } finally {
        finishCatalogMutation(mutation);
      }
    },
    [beginCatalogMutation, finishCatalogMutation, patchCatalog, productClient],
  );

  const createSession = useCallback(
    async (workspaceId: string): Promise<SessionRecord | null> => {
      const mutation = beginCatalogMutation();
      if (mutation === null) {
        return null;
      }
      try {
        const productSession = await productClient.createSession({
          workspace_id: workspaceId,
        });
        if (mutationGenerationRef.current !== mutation) {
          return null;
        }
        const session = fromProductSession(productSession);
        patchCatalog((current) => ({
          ...current,
          sessions: [
            session,
            ...current.sessions.filter((item) => item.id !== session.id),
          ],
        }));
        return session;
      } catch (error) {
        if (mutationGenerationRef.current === mutation) {
          setCatalogError(describeError(error));
        }
        return null;
      } finally {
        finishCatalogMutation(mutation);
      }
    },
    [beginCatalogMutation, finishCatalogMutation, patchCatalog, productClient],
  );

  const forkSession = useCallback(
    async (sessionId: string): Promise<SessionRecord | null> => {
      const parent = catalogRef.current.sessions.find((session) => session.id === sessionId);
      if (!parent?.activeRunId || parent.status !== "idle") {
        setCatalogError("A session can be forked only from its completed latest turn.");
        return null;
      }
      const mutation = beginCatalogMutation();
      if (mutation === null) {
        return null;
      }
      const requestKey = `${sessionId}\u0000${parent.activeRunId}`;
      const idempotencyKey =
        forkIdempotencyRef.current.get(requestKey) ?? newId("fork");
      forkIdempotencyRef.current.set(requestKey, idempotencyKey);
      try {
        const response = await productClient.createFork(sessionId, {
          fork_at_run_id: parent.activeRunId,
          idempotency_key: idempotencyKey,
        });
        if (mutationGenerationRef.current !== mutation) {
          return null;
        }
        forkIdempotencyRef.current.delete(requestKey);
        const session = fromProductSession(response.session);
        patchCatalog((current) => ({
          ...current,
          sessions: [
            session,
            ...current.sessions.filter((item) => item.id !== session.id),
          ],
        }));
        return session;
      } catch (error) {
        if (mutationGenerationRef.current === mutation) {
          setCatalogError(describeError(error));
        }
        return null;
      } finally {
        finishCatalogMutation(mutation);
      }
    },
    [beginCatalogMutation, finishCatalogMutation, patchCatalog, productClient],
  );

  const togglePin = useCallback(
    async (workspaceId: string) => {
      const workspace = catalogRef.current.workspaces.find(
        (item) => item.id === workspaceId,
      );
      if (!workspace || workspace.kind === "task") {
        return;
      }
      const mutation = beginCatalogMutation();
      if (mutation === null) {
        throw new Error("Another catalog change is already in progress.");
      }
      try {
        const saved = await productClient.createWorkspace({
          root: workspace.rootPath,
          kind: workspace.kind,
          display_name: workspace.displayName,
          pinned: !workspace.pinned,
        });
        if (mutationGenerationRef.current !== mutation) {
          return;
        }
        const record = fromProductWorkspace(saved);
        patchCatalog((current) => ({
          ...current,
          workspaces: current.workspaces.map((item) =>
            item.id === record.id ? record : item,
          ),
        }));
      } catch (error) {
        if (mutationGenerationRef.current === mutation) {
          setCatalogError(describeError(error));
        }
        throw error;
      } finally {
        finishCatalogMutation(mutation);
      }
    },
    [beginCatalogMutation, finishCatalogMutation, patchCatalog, productClient],
  );

  const removeWorkspace = useCallback(
    async (workspaceId: string) => {
      const mutation = beginCatalogMutation();
      if (mutation === null) {
        throw new Error("Another catalog change is already in progress.");
      }
      try {
        await productClient.deleteWorkspace(workspaceId);
        if (mutationGenerationRef.current !== mutation) {
          return false;
        }
        const remainsActive =
          catalogRef.current.active.workspaceId === workspaceId;
        patchCatalog((current) => ({
          workspaces: current.workspaces.filter((item) => item.id !== workspaceId),
          sessions: current.sessions.filter(
            (session) => session.workspaceId !== workspaceId,
          ),
          active: remainsActive
            ? { workspaceId: null, sessionId: null }
            : current.active,
        }));
        if (remainsActive && preferencesRef.current) {
          queuePreferences({
            ...preferencesRef.current,
            active_workspace_id: undefined,
            active_session_id: undefined,
          });
        }
        return remainsActive;
      } catch (error) {
        if (mutationGenerationRef.current === mutation) {
          setCatalogError(describeError(error));
        }
        throw error;
      } finally {
        finishCatalogMutation(mutation);
      }
    },
    [
      beginCatalogMutation,
      finishCatalogMutation,
      patchCatalog,
      productClient,
      queuePreferences,
    ],
  );

  const updateSessionTitle = useCallback(
    async (sessionId: string, title: string) => {
      const generation =
        (sessionUpdateGenerationsRef.current.get(sessionId) ?? 0) + 1;
      sessionUpdateGenerationsRef.current.set(sessionId, generation);
      try {
        const saved = await productClient.updateSession(sessionId, { title });
        if (
          sessionUpdateGenerationsRef.current.get(sessionId) !== generation
        ) {
          return;
        }
        const record = fromProductSession(saved);
        patchCatalog((current) => ({
          ...current,
          sessions: current.sessions.map((session) =>
            session.id === record.id ? record : session,
          ),
        }));
      } catch (error) {
        if (
          sessionUpdateGenerationsRef.current.get(sessionId) === generation
        ) {
          setCatalogError(`Could not persist session title: ${describeError(error)}`);
        }
        throw error;
      }
    },
    [patchCatalog, productClient],
  );

  const deleteSession = useCallback(
    async (sessionId: string): Promise<boolean> => {
      const mutation = beginCatalogMutation();
      if (mutation === null) {
        throw new Error("Another catalog change is already in progress.");
      }
      try {
        await productClient.deleteSession(sessionId);
        if (mutationGenerationRef.current !== mutation) {
          return false;
        }
        sessionUpdateGenerationsRef.current.delete(sessionId);
        const remainsActive = catalogRef.current.active.sessionId === sessionId;
        patchCatalog((current) => ({
          ...current,
          sessions: current.sessions.filter((session) => session.id !== sessionId),
          active: remainsActive
            ? { ...current.active, sessionId: null }
            : current.active,
        }));
        if (remainsActive && preferencesRef.current) {
          queuePreferences({
            ...preferencesRef.current,
            active_session_id: undefined,
          });
        }
        return remainsActive;
      } catch (error) {
        if (mutationGenerationRef.current === mutation) {
          setCatalogError(`Could not delete session: ${describeError(error)}`);
        }
        throw error;
      } finally {
        finishCatalogMutation(mutation);
      }
    },
    [
      beginCatalogMutation,
      finishCatalogMutation,
      patchCatalog,
      productClient,
      queuePreferences,
    ],
  );

  const changeTheme = useCallback(
    (nextTheme: "light" | "dark") => {
      setTheme(nextTheme);
      if (preferencesRef.current) {
        queuePreferences({ ...preferencesRef.current, theme: nextTheme });
      }
    },
    [queuePreferences],
  );

  const changeSessionModelConfig = useCallback(
    async (next: SessionModelConfigInput): Promise<boolean> => {
      const sessionId = catalogRef.current.active.sessionId;
      const current = sessionModelConfigRef.current;
      if (!sessionId || !current || current.sessionId !== sessionId) {
        setCatalogError("Session model settings are not loaded.");
        return false;
      }
      if (next.profileId && deletingProviderProfileIdsRef.current.has(next.profileId)) {
        setCatalogError("That provider profile is currently being removed.");
        return false;
      }
      setSessionModelConfigMutationBusy(true);
      try {
        const saved = await productClient.updateSessionModelConfig(sessionId, {
          ...(next.profileId ? { profile_id: next.profileId } : {}),
          model: next.model.trim(),
          reasoning: next.reasoning,
          max_steps: next.maxSteps,
          expected_revision: current.revision,
        });
        if (catalogRef.current.active.sessionId !== sessionId) {
          return false;
        }
        const record = fromProductSessionModelConfig(saved);
        sessionModelConfigRef.current = record;
        setSessionModelConfig(record);
        setCatalogError(null);
        setConnection("ok");
        return true;
      } catch (error) {
        if (
          error instanceof ProductApiError &&
          error.code === "product_session_model_config_conflict"
        ) {
          await loadSessionModelConfig(sessionId);
        }
        setCatalogError(`Could not persist session model settings: ${describeError(error)}`);
        return false;
      } finally {
        setSessionModelConfigMutationBusy(false);
      }
    },
    [catalogRef, loadSessionModelConfig, productClient],
  );

  const changeSelection = useCallback(
    async (next: ActiveProviderSelection): Promise<boolean> => {
      if (
        next.mode === "profile" &&
        next.profileId &&
        deletingProviderProfileIdsRef.current.has(next.profileId)
      ) {
        setCatalogError("That provider profile is currently being removed.");
        return false;
      }
      const current = preferencesRef.current;
      if (!current) {
        setCatalogError("Product preferences are not loaded.");
        return false;
      }
      const synchronized = {
        ...next,
        approval: current.default_approval_policy,
      };
      setSelection(synchronized);
      try {
        await persistPreferences({
          ...current,
          provider_selection: {
            profile_id:
              synchronized.mode === "profile"
                ? synchronized.profileId
                : undefined,
            model: synchronized.model,
            approval: synchronized.approval,
            max_steps: synchronized.maxSteps,
          },
        });
        return true;
      } catch {
        return false;
      }
    },
    [persistPreferences],
  );

  const changeDefaultApprovalPolicy = useCallback(
    async (policy: ProductApprovalPreference): Promise<void> => {
      const current = preferencesRef.current;
      if (!current) {
        throw new Error("Product preferences are not loaded.");
      }
      const next = {
        ...current,
        default_approval_policy: policy,
        provider_selection: current.provider_selection
          ? { ...current.provider_selection, approval: policy }
          : undefined,
      };
      setSelection((active) => ({ ...active, approval: policy }));
      await persistPreferences(next);
    },
    [persistPreferences],
  );

  const refreshProviderProfiles = useCallback(async (): Promise<
    ProviderProfileRecord[]
  > => {
    const refreshed = await productClient.listProviderProfiles();
    const records = refreshed.provider_profiles.map(fromProductProviderProfile);
    providerCatalogRevisionRef.current = refreshed.catalog_revision;
    setProfiles(records);
    setCatalogError(null);
    return records;
  }, [productClient]);

  const createProviderProfile = useCallback(
    async (input: ProviderProfileInput): Promise<ProviderProfileRecord> => {
      const saved = await productClient.createProviderProfile({
        label: input.label,
        provider_type: input.providerType,
        api_base: input.apiBase,
        api_key_env: input.apiKeyEnv,
        default_model: input.defaultModel,
        expected_revision: providerCatalogRevisionRef.current ?? undefined,
      });
      const record = fromProductProviderProfile(saved);
      providerCatalogRevisionRef.current = record.catalogRevision;
      setProfiles((current) => [
        record,
        ...current.filter((profile) => profile.id !== record.id),
      ]);
      setCatalogError(null);
      return record;
    },
    [productClient],
  );

  const updateProviderProfile = useCallback(
    async (
      profileId: string,
      input: ProviderProfileInput,
    ): Promise<ProviderProfileRecord> => {
      if (deletingProviderProfileIdsRef.current.has(profileId)) {
        throw new Error("That provider profile is currently being removed.");
      }
      try {
        const saved = await productClient.updateProviderProfile(profileId, {
          label: input.label,
          provider_type: input.providerType,
          api_base: input.apiBase,
          api_key_env: input.apiKeyEnv,
          default_model: input.defaultModel,
          expected_revision:
            profiles.find((profile) => profile.id === profileId)?.catalogRevision ??
            providerCatalogRevisionRef.current ??
            undefined,
        });
        const record = fromProductProviderProfile(saved);
        providerCatalogRevisionRef.current = record.catalogRevision;
        setProfiles((current) =>
          current.map((profile) =>
            profile.id === record.id ? record : profile,
          ),
        );
        setCatalogError(null);
        return record;
      } catch (error) {
        setCatalogError(`Could not update provider profile: ${describeError(error)}`);
        throw error;
      }
    },
    [productClient, profiles],
  );

  const deleteProviderProfile = useCallback(
    async (profileId: string): Promise<void> => {
      if (deletingProviderProfileIdsRef.current.has(profileId)) {
        throw new Error("That provider profile is already being removed.");
      }
      deletingProviderProfileIdsRef.current.add(profileId);
      try {
        await preferencesQueueRef.current.catch(() => undefined);
        const current = preferencesRef.current;
        if (current?.provider_selection?.profile_id === profileId) {
          const cleared = {
            ...current,
            provider_selection: {
              ...current.provider_selection,
              profile_id: undefined,
            },
          };
          setSelection(selectionFromPreferences(cleared));
          await persistPreferences(cleared);
        }
        const expectedRevision =
          profiles.find((profile) => profile.id === profileId)?.catalogRevision ??
          providerCatalogRevisionRef.current ??
          undefined;
        await productClient.deleteProviderProfile(profileId, expectedRevision);
        const refreshed = await productClient.listProviderProfiles();
        providerCatalogRevisionRef.current = refreshed.catalog_revision;
        setProfiles(refreshed.provider_profiles.map(fromProductProviderProfile));
        setCatalogError(null);
      } finally {
        deletingProviderProfileIdsRef.current.delete(profileId);
      }
    },
    [persistPreferences, productClient, profiles],
  );

  return {
    productClient,
    bootState,
    reload: loadInitialState,
    catalog,
    catalogRef,
    preferences,
    profiles,
    sessionModelConfig,
    sessionModelConfigLoading,
    sessionModelConfigMutationBusy,
    changeSessionModelConfig,
    createProviderProfile,
    updateProviderProfile,
    deleteProviderProfile,
    refreshProviderProfiles,
    selection,
    changeSelection,
    changeDefaultApprovalPolicy,
    theme,
    changeTheme,
    connection,
    setConnection,
    catalogError,
    catalogMutationBusy,
    preferencesMutationBusy,
    clearCatalogError: () => setCatalogError(null),
    persistActiveRoute,
    selectCatalogRoute,
    markSession,
    refreshSessionStatuses,
    openWorkspace,
    createSession,
    forkSession,
    togglePin,
    removeWorkspace,
    updateSessionTitle,
    deleteSession,
  };
}
