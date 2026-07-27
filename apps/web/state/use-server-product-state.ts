"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { describeError } from "../api/run-controller";
import { applyDocumentTheme, webPlatform } from "../platform/web";
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
  WorkspaceKind,
} from "./product-types";
import {
  fromProductProviderProfile,
  fromProductSession,
  fromProductWorkspace,
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
  const [selection, setSelection] = useState<ActiveProviderSelection>(() => ({
    mode: "default",
    model: "fake",
    approval: "ask",
    maxSteps: 8,
  }));
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [bootState, setBootState] = useState<ProductBootState>({ status: "loading" });
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogMutationBusy, setCatalogMutationBusy] = useState(false);
  const [connection, setConnection] = useState<"unknown" | "ok" | "error">(
    "unknown",
  );
  const catalogRef = useRef(catalog);
  const preferencesRef = useRef(preferences);
  const confirmedPreferencesRef = useRef<ProductPreferences | null>(null);
  const bootGenerationRef = useRef(0);
  const catalogGenerationRef = useRef(0);
  const preferencesGenerationRef = useRef(0);
  const mutationGenerationRef = useRef(0);
  const sessionUpdateGenerationsRef = useRef(new Map<string, number>());
  const catalogMutationRef = useRef(false);
  const preferencesQueueRef = useRef<Promise<void>>(Promise.resolve());
  const deletingProviderProfileIdsRef = useRef(new Set<string>());
  const failedActiveRouteTargetRef = useRef<string | null>(null);

  useEffect(() => {
    catalogRef.current = catalog;
  }, [catalog]);

  useEffect(() => {
    preferencesRef.current = preferences;
  }, [preferences]);

  useEffect(() => {
    applyDocumentTheme(theme);
    webPlatform.setThemePreference(theme);
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
          setTheme(resolveProductTheme(saved.theme));
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
                setTheme(resolveProductTheme(current.theme));
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
              setTheme(resolveProductTheme(confirmed.theme));
            }
            setCatalogError(`Could not persist preferences: ${describeError(error)}`);
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
      setSelection(selectionFromPreferences(loadedPreferences));
      setTheme(resolveProductTheme(loadedPreferences.theme));
      setConnection("ok");
      setBootState({ status: "ready" });
    } catch (error) {
      if (bootGenerationRef.current !== bootGeneration) {
        return;
      }
      setConnection("error");
      setBootState({ status: "error", error: describeError(error) });
    }
  }, [productClient]);

  useEffect(() => {
    void loadInitialState();
    return () => {
      ++bootGenerationRef.current;
      ++catalogGenerationRef.current;
      ++preferencesGenerationRef.current;
      ++mutationGenerationRef.current;
      sessionUpdateGenerationsRef.current.clear();
    };
  }, [loadInitialState]);

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
        let sessionResponse = await productClient.listSessions(workspace.id);
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

  const changeSelection = useCallback(
    (next: ActiveProviderSelection) => {
      if (
        next.mode === "profile" &&
        next.profileId &&
        deletingProviderProfileIdsRef.current.has(next.profileId)
      ) {
        setCatalogError("That provider profile is currently being removed.");
        return;
      }
      const current = preferencesRef.current;
      if (!current) {
        return;
      }
      const synchronized = {
        ...next,
        approval: current.default_approval_policy,
      };
      setSelection(synchronized);
      queuePreferences({
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
    },
    [queuePreferences],
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

  const createProviderProfile = useCallback(
    async (input: ProviderProfileInput): Promise<ProviderProfileRecord> => {
      const saved = await productClient.createProviderProfile({
        label: input.label,
        provider_type: input.providerType,
        api_base: input.apiBase,
        api_key_env: input.apiKeyEnv,
        default_model: input.defaultModel,
      });
      const record = fromProductProviderProfile(saved);
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
        });
        const record = fromProductProviderProfile(saved);
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
    [productClient],
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
        await productClient.deleteProviderProfile(profileId);
        setProfiles((currentProfiles) =>
          currentProfiles.filter((profile) => profile.id !== profileId),
        );
        setCatalogError(null);
      } finally {
        deletingProviderProfileIdsRef.current.delete(profileId);
      }
    },
    [persistPreferences, productClient],
  );

  return {
    productClient,
    bootState,
    reload: loadInitialState,
    catalog,
    catalogRef,
    preferences,
    profiles,
    createProviderProfile,
    updateProviderProfile,
    deleteProviderProfile,
    selection,
    changeSelection,
    changeDefaultApprovalPolicy,
    theme,
    changeTheme,
    connection,
    setConnection,
    catalogError,
    catalogMutationBusy,
    clearCatalogError: () => setCatalogError(null),
    persistActiveRoute,
    selectCatalogRoute,
    markSession,
    refreshSessionStatuses,
    openWorkspace,
    createSession,
    togglePin,
    removeWorkspace,
    updateSessionTitle,
    deleteSession,
  };
}
