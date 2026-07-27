"use client";

import { CheckIcon, Cross2Icon, Pencil2Icon, TrashIcon } from "@radix-ui/react-icons";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";

import { listProviderModels, testProvider } from "../api/run-controller";
import { BenchmarkPanel } from "../components/benchmark-panel";
import type {
  ProviderModelsResponse,
  ProviderTestResponse,
  ProviderType,
} from "../lib/rove-types";
import type { ProductApprovalPreference } from "../product/product-api-types";
import {
  providerDefaultApiBase,
  providerDefaultKeyEnv,
  providerDisplayName,
  providerRequiresKey,
} from "../state/provider-store";
import type {
  ActiveProviderSelection,
  ProviderProfileInput,
  ProviderProfileRecord,
  SessionRecord,
  WorkspaceRecord,
} from "../state/product-types";
import { toApiProviderProfile } from "../state/product-types";
import { SessionsSettings, WorkspaceSettings } from "./CatalogSettings";
import { KeyboardSettings } from "./KeyboardSettings";
import { MemorySettings } from "./MemorySettings";
import { RuntimeSettings } from "./RuntimeSettings";
import type { SettingsSectionId } from "./sections";
import { SETTINGS_SECTIONS } from "./sections";
import type { SettingsPlatformClient } from "./settings-platform-client";

type MaybePromise = void | Promise<unknown>;

export interface SettingsShellProps {
  section: SettingsSectionId;
  onSectionChange: (section: SettingsSectionId) => void;
  settingsClient: SettingsPlatformClient;
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  defaultApprovalPolicy: ProductApprovalPreference;
  onCreateProfile: (
    profile: ProviderProfileInput,
  ) => Promise<ProviderProfileRecord>;
  onUpdateProfile: (
    profileId: string,
    profile: ProviderProfileInput,
  ) => Promise<ProviderProfileRecord>;
  onDeleteProfile: (profileId: string) => Promise<void>;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
  onDefaultApprovalPolicyChange: (
    policy: ProductApprovalPreference,
  ) => Promise<void>;
  workspaces: readonly WorkspaceRecord[];
  sessions: readonly SessionRecord[];
  activeWorkspaceId: string | null;
  activeSessionId: string | null;
  onSelectWorkspace: (workspaceId: string) => MaybePromise;
  onSelectSession: (workspaceId: string, sessionId: string) => MaybePromise;
  onTogglePin: (workspaceId: string) => MaybePromise;
  onRemoveWorkspace: (workspaceId: string) => MaybePromise;
  onRenameSession: (sessionId: string, title: string) => MaybePromise;
  onDeleteSession: (sessionId: string) => MaybePromise;
  connectionLabel: string;
  theme: "light" | "dark";
  onThemeChange: (theme: "light" | "dark") => void;
  error: string | null;
}

export function SettingsShell(props: SettingsShellProps) {
  const {
    section,
    onSectionChange,
    settingsClient,
    profiles,
    selection,
    defaultApprovalPolicy,
    onCreateProfile,
    onUpdateProfile,
    onDeleteProfile,
    onSelectionChange,
    onDefaultApprovalPolicyChange,
    workspaces,
    sessions,
    activeWorkspaceId,
    activeSessionId,
    onSelectWorkspace,
    onSelectSession,
    onTogglePin,
    onRemoveWorkspace,
    onRenameSession,
    onDeleteSession,
    connectionLabel,
    theme,
    onThemeChange,
    error,
  } = props;
  const activeSectionRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    activeSectionRef.current?.scrollIntoView({
      block: "nearest",
      inline: "center",
    });
  }, [section]);

  return (
    <div className="settings-shell">
      <nav className="settings-nav" aria-label="Settings sections">
        <h2>Settings</h2>
        {SETTINGS_SECTIONS.map((item) => (
          <button
            ref={item.id === section ? activeSectionRef : undefined}
            key={item.id}
            type="button"
            data-active={item.id === section}
            aria-current={item.id === section ? "page" : undefined}
            onClick={() => onSectionChange(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>
      <div className="settings-content">
        {error ? (
          <div className="shell-alert" role="alert">
            {error}
          </div>
        ) : null}
        {section === "general" ? (
          <GeneralSettings theme={theme} onThemeChange={onThemeChange} />
        ) : null}
        {section === "providers" ? (
          <ProvidersSettings
            profiles={profiles}
            selection={selection}
            onCreateProfile={onCreateProfile}
            onUpdateProfile={onUpdateProfile}
            onDeleteProfile={onDeleteProfile}
            onSelectionChange={onSelectionChange}
          />
        ) : null}
        {section === "tools" ? (
          <ToolsSettings
            selection={selection}
            defaultApprovalPolicy={defaultApprovalPolicy}
            onSelectionChange={onSelectionChange}
            onDefaultApprovalPolicyChange={onDefaultApprovalPolicyChange}
          />
        ) : null}
        {section === "workspace" ? (
          <WorkspaceSettings
            workspaces={workspaces}
            activeWorkspaceId={activeWorkspaceId}
            onSelectWorkspace={onSelectWorkspace}
            onTogglePin={onTogglePin}
            onRemoveWorkspace={onRemoveWorkspace}
          />
        ) : null}
        {section === "memory" ? <MemorySettings client={settingsClient} /> : null}
        {section === "sessions" ? (
          <SessionsSettings
            workspaces={workspaces}
            sessions={sessions}
            activeSessionId={activeSessionId}
            onSelectSession={onSelectSession}
            onRenameSession={onRenameSession}
            onDeleteSession={onDeleteSession}
          />
        ) : null}
        {section === "keyboard" ? <KeyboardSettings /> : null}
        {section === "advanced" ? <AdvancedSettings /> : null}
        {section === "about" ? (
          <RuntimeSettings
            client={settingsClient}
            connectionLabel={connectionLabel}
            theme={theme}
          />
        ) : null}
      </div>
    </div>
  );
}

function GeneralSettings({
  theme,
  onThemeChange,
}: {
  theme: "light" | "dark";
  onThemeChange: (theme: "light" | "dark") => void;
}) {
  return (
    <section className="settings-panel" aria-labelledby="general-settings-title">
      <h1 id="general-settings-title">General</h1>
      <p className="lede">Appearance and product defaults.</p>
      <div className="settings-card">
        <h2>Theme</h2>
        <div className="settings-segmented" role="group" aria-label="Theme">
          <button
            type="button"
            aria-pressed={theme === "light"}
            data-active={theme === "light"}
            onClick={() => onThemeChange("light")}
          >
            Light
          </button>
          <button
            type="button"
            aria-pressed={theme === "dark"}
            data-active={theme === "dark"}
            onClick={() => onThemeChange("dark")}
          >
            Dark
          </button>
        </div>
      </div>
    </section>
  );
}

function ToolsSettings({
  selection,
  defaultApprovalPolicy,
  onSelectionChange,
  onDefaultApprovalPolicyChange,
}: {
  selection: ActiveProviderSelection;
  defaultApprovalPolicy: ProductApprovalPreference;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
  onDefaultApprovalPolicyChange: (
    policy: ProductApprovalPreference,
  ) => Promise<void>;
}) {
  const [maxSteps, setMaxSteps] = useState(String(selection.maxSteps));
  const [approvalBusy, setApprovalBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setMaxSteps(String(selection.maxSteps));
  }, [selection.maxSteps]);

  async function handleApprovalChange(policy: ProductApprovalPreference) {
    setApprovalBusy(true);
    setError(null);
    try {
      await onDefaultApprovalPolicyChange(policy);
    } catch (approvalError) {
      setError(
        approvalError instanceof Error
          ? approvalError.message
          : String(approvalError),
      );
    } finally {
      setApprovalBusy(false);
    }
  }

  function handleMaxSteps(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const parsed = Number(maxSteps);
    if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 4096) {
      setError("Maximum steps must be an integer from 1 to 4096.");
      return;
    }
    setError(null);
    onSelectionChange({ ...selection, maxSteps: parsed });
  }

  return (
    <section className="settings-panel" aria-labelledby="tools-settings-title">
      <h1 id="tools-settings-title">Tools &amp; Approvals</h1>
      <p className="lede">Default tool authorization and execution limits.</p>

      <div className="settings-card">
        <h2>Default approval policy</h2>
        <div className="settings-segmented" role="group" aria-label="Default approval policy">
          {(["ask", "auto", "never"] as const).map((policy) => (
            <button
              key={policy}
              type="button"
              data-active={defaultApprovalPolicy === policy}
              aria-pressed={defaultApprovalPolicy === policy}
              disabled={approvalBusy}
              onClick={() => void handleApprovalChange(policy)}
            >
              {policy === "ask" ? "Ask" : policy === "auto" ? "Auto" : "Never"}
            </button>
          ))}
        </div>
        <div className="settings-policy-grid">
          <div>
            <strong>Ask</strong>
            <span>Pause when a tool requires approval.</span>
          </div>
          <div>
            <strong>Auto</strong>
            <span>Allow runtime-approved tool execution.</span>
          </div>
          <div>
            <strong>Never</strong>
            <span>Reject tool calls that require approval.</span>
          </div>
        </div>
      </div>

      <form className="settings-card" onSubmit={handleMaxSteps}>
        <h2>Execution limit</h2>
        <div className="field settings-number-field">
          <label htmlFor="settings-max-steps">Maximum steps per job</label>
          <input
            id="settings-max-steps"
            type="number"
            min={1}
            max={4096}
            step={1}
            inputMode="numeric"
            value={maxSteps}
            onChange={(event) => setMaxSteps(event.target.value)}
          />
        </div>
        <div className="field-actions">
          <button type="submit">
            <CheckIcon /> Save limit
          </button>
        </div>
      </form>

      {error ? (
        <div className="chat-error" role="alert">
          {error}
        </div>
      ) : null}
    </section>
  );
}

function AdvancedSettings() {
  const [showBenchmark, setShowBenchmark] = useState(false);

  return (
    <section className="settings-panel" aria-labelledby="advanced-settings-title">
      <h1 id="advanced-settings-title">Advanced / Developer</h1>
      <p className="lede">Developer surfaces and migration escape hatches.</p>
      <div className="settings-card">
        <h2>Developer tools</h2>
        <div className="advanced-links">
          <button
            type="button"
            className="advanced-link-card"
            onClick={() => setShowBenchmark((value) => !value)}
            aria-expanded={showBenchmark}
          >
            Benchmark runner
            <span>Deterministic evaluation suites against the API.</span>
          </button>
          <a className="advanced-link-card" href="/dev/workbench">
            Legacy workbench
            <span>Open the temporary migration and diagnostics surface.</span>
          </a>
        </div>
      </div>
      {showBenchmark ? (
        <div className="advanced-benchmark" aria-label="Benchmark runner">
          <BenchmarkPanel />
        </div>
      ) : null}
    </section>
  );
}

function ProvidersSettings({
  profiles,
  selection,
  onCreateProfile,
  onUpdateProfile,
  onDeleteProfile,
  onSelectionChange,
}: {
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  onCreateProfile: (
    profile: ProviderProfileInput,
  ) => Promise<ProviderProfileRecord>;
  onUpdateProfile: (
    profileId: string,
    profile: ProviderProfileInput,
  ) => Promise<ProviderProfileRecord>;
  onDeleteProfile: (profileId: string) => Promise<void>;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
}) {
  const [label, setLabel] = useState("Local OpenAI");
  const [providerType, setProviderType] = useState<ProviderType>("openai");
  const [apiBase, setApiBase] = useState(providerDefaultApiBase("openai"));
  const [apiKeyEnv, setApiKeyEnv] = useState(providerDefaultKeyEnv("openai"));
  const [defaultModel, setDefaultModel] = useState("gpt-4.1-mini");
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null);
  const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [modelsBusy, setModelsBusy] = useState(false);
  const [saveBusy, setSaveBusy] = useState(false);
  const [deletingProfileId, setDeletingProfileId] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ProviderTestResponse | null>(null);
  const [modelsResult, setModelsResult] = useState<ProviderModelsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const profileDeleteBusy = deletingProfileId !== null;

  const activeProfile = useMemo(
    () => profiles.find((profile) => profile.id === selection.profileId) ?? null,
    [profiles, selection.profileId],
  );

  function profileInput(): ProviderProfileInput {
    return {
      label: label.trim() || providerDisplayName(providerType),
      providerType,
      apiBase: apiBase.trim(),
      apiKeyEnv: providerRequiresKey(providerType)
        ? apiKeyEnv.trim() || providerDefaultKeyEnv(providerType)
        : undefined,
      defaultModel: defaultModel.trim() || undefined,
    };
  }

  function draftProfile(): ProviderProfileRecord {
    return {
      id: editingProfileId ?? "draft",
      ...profileInput(),
      updatedAt: new Date().toISOString(),
    };
  }

  function resetDraft() {
    setEditingProfileId(null);
    setLabel("Local OpenAI");
    setProviderType("openai");
    setApiBase(providerDefaultApiBase("openai"));
    setApiKeyEnv(providerDefaultKeyEnv("openai"));
    setDefaultModel("gpt-4.1-mini");
    setTestResult(null);
    setModelsResult(null);
    setError(null);
  }

  function startEdit(profile: ProviderProfileRecord) {
    setEditingProfileId(profile.id);
    setLabel(profile.label);
    setProviderType(profile.providerType);
    setApiBase(profile.apiBase);
    setApiKeyEnv(profile.apiKeyEnv ?? providerDefaultKeyEnv(profile.providerType));
    setDefaultModel(profile.defaultModel ?? "");
    setTestResult(null);
    setModelsResult(null);
    setError(null);
  }

  function handleTypeChange(next: ProviderType) {
    setProviderType(next);
    setApiBase(providerDefaultApiBase(next));
    setApiKeyEnv(providerDefaultKeyEnv(next));
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaveBusy(true);
    setError(null);
    try {
      const saved = editingProfileId
        ? await onUpdateProfile(editingProfileId, profileInput())
        : await onCreateProfile(profileInput());
      if (!editingProfileId) {
        onSelectionChange({
          ...selection,
          mode: "profile",
          profileId: saved.id,
          model: saved.defaultModel || selection.model,
        });
      } else if (selection.profileId === saved.id && saved.defaultModel) {
        onSelectionChange({ ...selection, model: saved.defaultModel });
      }
      resetDraft();
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : String(saveError));
    } finally {
      setSaveBusy(false);
    }
  }

  async function handleDelete(profileId: string) {
    setDeletingProfileId(profileId);
    setError(null);
    try {
      await onDeleteProfile(profileId);
      if (editingProfileId === profileId) {
        resetDraft();
      }
      setConfirmingDeleteId(null);
    } catch (deleteError) {
      setError(
        deleteError instanceof Error ? deleteError.message : String(deleteError),
      );
    } finally {
      setDeletingProfileId(null);
    }
  }

  async function handleTest() {
    setTestBusy(true);
    setError(null);
    setTestResult(null);
    try {
      setTestResult(
        await testProvider({
          provider: toApiProviderProfile(draftProfile()),
          model: defaultModel.trim() || undefined,
        }),
      );
    } catch (testError) {
      setError(testError instanceof Error ? testError.message : String(testError));
    } finally {
      setTestBusy(false);
    }
  }

  async function handleListModels() {
    setModelsBusy(true);
    setError(null);
    setModelsResult(null);
    try {
      const result = await listProviderModels({
        provider: toApiProviderProfile(draftProfile()),
      });
      setModelsResult(result);
      if (result.models[0] && !defaultModel.trim()) {
        setDefaultModel(result.models[0]);
      }
    } catch (modelsError) {
      setError(
        modelsError instanceof Error ? modelsError.message : String(modelsError),
      );
    } finally {
      setModelsBusy(false);
    }
  }

  return (
    <section className="settings-panel" aria-labelledby="providers-settings-title">
      <h1 id="providers-settings-title">Providers &amp; Models</h1>
      <p className="lede">
        Durable provider profiles store environment variable names, never raw keys.
      </p>

      <div className="settings-card">
        <h2>Active selection</h2>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="provider-mode">Mode</label>
            <select
              id="provider-mode"
              value={selection.mode}
              disabled={profileDeleteBusy}
              onChange={(event) =>
                onSelectionChange({
                  ...selection,
                  mode: event.target.value as ActiveProviderSelection["mode"],
                  profileId:
                    event.target.value === "profile"
                      ? selection.profileId ?? profiles[0]?.id
                      : undefined,
                })
              }
            >
              <option value="default">Runtime default</option>
              <option value="profile">Saved profile</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="provider-profile">Profile</label>
            <select
              id="provider-profile"
              value={selection.profileId ?? ""}
              disabled={profileDeleteBusy || selection.mode !== "profile"}
              onChange={(event) => {
                const profile = profiles.find((item) => item.id === event.target.value);
                onSelectionChange({
                  ...selection,
                  mode: "profile",
                  profileId: profile?.id,
                  model: profile?.defaultModel || selection.model,
                });
              }}
            >
              <option value="">Select profile…</option>
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.label}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="provider-model">Model</label>
            <input
              id="provider-model"
              value={selection.model}
              disabled={profileDeleteBusy}
              onChange={(event) =>
                onSelectionChange({ ...selection, model: event.target.value })
              }
              placeholder="fake"
            />
          </div>
        </div>
        <p className="settings-inline-note">
          {selection.mode === "profile" && activeProfile
            ? `${activeProfile.label} · ${activeProfile.apiBase}${activeProfile.apiKeyEnv ? ` · env ${activeProfile.apiKeyEnv}` : ""}`
            : "Using the API process default provider configuration."}
        </p>
      </div>

      <form className="settings-card" onSubmit={handleSave}>
        <div className="settings-card__heading">
          <h2>{editingProfileId ? "Edit profile" : "Add profile"}</h2>
          {editingProfileId ? (
            <button type="button" className="secondary" onClick={resetDraft} disabled={saveBusy}>
              <Cross2Icon /> Cancel edit
            </button>
          ) : null}
        </div>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="profile-label">Label</label>
            <input id="profile-label" value={label} onChange={(event) => setLabel(event.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="profile-type">Type</label>
            <select
              id="profile-type"
              value={providerType}
              onChange={(event) => handleTypeChange(event.target.value as ProviderType)}
            >
              <option value="openai">OpenAI</option>
              <option value="openai-responses">OpenAI Responses</option>
              <option value="anthropic">Anthropic</option>
              <option value="ollama">Ollama</option>
              <option value="fake">Fake</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="profile-base">API base</label>
            <input id="profile-base" value={apiBase} onChange={(event) => setApiBase(event.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="profile-key-env">API key env name</label>
            <input
              id="profile-key-env"
              value={apiKeyEnv}
              onChange={(event) => setApiKeyEnv(event.target.value)}
              disabled={!providerRequiresKey(providerType)}
              placeholder={providerRequiresKey(providerType) ? "OPENAI_API_KEY" : "not required"}
            />
          </div>
          <div className="field">
            <label htmlFor="profile-default-model">Default model</label>
            <input
              id="profile-default-model"
              value={defaultModel}
              onChange={(event) => setDefaultModel(event.target.value)}
            />
          </div>
        </div>
        <div className="field-actions">
          <button type="submit" disabled={saveBusy || profileDeleteBusy}>
            <CheckIcon /> {saveBusy ? "Saving…" : editingProfileId ? "Update profile" : "Save profile"}
          </button>
          <button type="button" className="secondary" disabled={testBusy} onClick={() => void handleTest()}>
            {testBusy ? "Testing…" : "Test"}
          </button>
          <button type="button" className="secondary" disabled={modelsBusy} onClick={() => void handleListModels()}>
            {modelsBusy ? "Loading…" : "List models"}
          </button>
        </div>
        {error ? <div className="chat-error" role="alert">{error}</div> : null}
        {testResult ? (
          <div className="placeholder-note">
            Test: {testResult.status} · key_present={String(testResult.key_present)} · models={testResult.models_count}
            {testResult.wire_protocol ? ` · wire ${testResult.wire_protocol}` : ""}
          </div>
        ) : null}
        {modelsResult ? (
          <div className="placeholder-note">
            Models ({modelsResult.models_count}): {modelsResult.models.slice(0, 12).join(", ") || "(none)"}
            {modelsResult.models.length > 12 ? "…" : ""}
          </div>
        ) : null}
      </form>

      <div className="settings-card">
        <h2>Saved profiles</h2>
        {profiles.length === 0 ? (
          <p className="settings-inline-note">No saved profiles yet.</p>
        ) : (
          <div className="profile-list">
            {profiles.map((profile) => (
              <div className="profile-row" key={profile.id}>
                <div>
                  <strong>{profile.label}</strong>
                  <span>{providerDisplayName(profile.providerType)} · {profile.apiBase}</span>
                  {confirmingDeleteId === profile.id ? (
                    <div className="settings-inline-confirm" role="alert">
                      <span>Remove this durable provider profile?</span>
                      <div className="field-actions">
                        <button type="button" className="secondary" disabled={profileDeleteBusy} onClick={() => setConfirmingDeleteId(null)}>
                          <Cross2Icon /> Cancel
                        </button>
                        <button type="button" className="danger" disabled={profileDeleteBusy} onClick={() => void handleDelete(profile.id)}>
                          <TrashIcon /> {deletingProfileId === profile.id ? "Removing…" : "Confirm remove"}
                        </button>
                      </div>
                    </div>
                  ) : null}
                </div>
                <div className="field-actions">
                  <button
                    type="button"
                    className="secondary"
                    disabled={profileDeleteBusy}
                    onClick={() => onSelectionChange({
                      ...selection,
                      mode: "profile",
                      profileId: profile.id,
                      model: profile.defaultModel || selection.model,
                    })}
                  >
                    Use
                  </button>
                  <button type="button" className="secondary" disabled={profileDeleteBusy} onClick={() => startEdit(profile)}>
                    <Pencil2Icon /> Edit
                  </button>
                  <button type="button" className="danger" disabled={profileDeleteBusy || confirmingDeleteId === profile.id} onClick={() => setConfirmingDeleteId(profile.id)}>
                    <TrashIcon /> Remove
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
