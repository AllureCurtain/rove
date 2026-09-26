"use client";

import { CheckIcon, Cross2Icon, Pencil2Icon, TrashIcon } from "@radix-ui/react-icons";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";

import { listProviderModels, testProvider } from "../api/run-controller";
import { useCopy } from "../copy/CopyProvider";
import { LatestWinsGate } from "../lib/latest-wins";
import { useUiSkin } from "../shell/ui-skin";
import type {
  ProviderModelsResponse,
  ProviderTestResponse,
  ProviderType,
} from "../lib/rove-types";
import {
  desktopProviderCredentialPromptAvailable,
  probeDesktopProvider,
  promptDesktopProviderCredential,
  useDesktopProvider,
  type DesktopProviderProbe,
} from "../platform/desktop-commands";
import { createProductApiClient } from "../product/product-client";
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
import { MCPSettings } from "./MCPSettings";
import { ProjectTrustSettings } from "./ProjectTrustSettings";
import { RuntimeSettings } from "./RuntimeSettings";
import { describeProviderProbeFailure } from "./provider-settings-model";
import type { SettingsSectionId } from "./sections";
import { SETTINGS_SECTION_COPY_KEYS, SETTINGS_SECTIONS } from "./sections";
import type { SettingsPlatformClient } from "./settings-platform-client";

type MaybePromise = void | Promise<unknown>;

const SILICONFLOW_PROFILE_ID = "siliconflow-deepseek-v3-2";
const SILICONFLOW_LABEL = "SiliconFlow DeepSeek V3.2";
const SILICONFLOW_API_BASE = "https://api.siliconflow.cn/v1";
const SILICONFLOW_MODEL = "deepseek-ai/DeepSeek-V3.2";

type NativeCredentialProviderType = Extract<
  ProviderType,
  "openai" | "openai-responses" | "anthropic"
>;

function isNativeCredentialProvider(
  providerType: ProviderType,
): providerType is NativeCredentialProviderType {
  return providerRequiresKey(providerType);
}

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
  onRefreshProviderProfiles: () => Promise<ProviderProfileRecord[]>;
  onSelectionChange: (selection: ActiveProviderSelection) => Promise<boolean>;
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
    onRefreshProviderProfiles,
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
  const { t } = useCopy();
  const activeSectionRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    activeSectionRef.current?.scrollIntoView({
      block: "nearest",
      inline: "center",
    });
  }, [section]);

  const sectionLabels = Object.fromEntries(
    SETTINGS_SECTIONS.map((item) => [
      item.id,
      t(SETTINGS_SECTION_COPY_KEYS[item.id]),
    ]),
  ) as Record<SettingsSectionId, string>;

  return (
    <div className="settings-shell">
      <nav className="settings-nav" aria-label={t("settings.title")}>
        <h2>{t("settings.title")}</h2>
        {SETTINGS_SECTIONS.filter((item) => item.id !== "advanced").map((item) => (
          <button
            ref={item.id === (section === "advanced" ? "general" : section) ? activeSectionRef : undefined}
            key={item.id}
            type="button"
            data-active={item.id === (section === "advanced" ? "general" : section)}
            aria-current={item.id === (section === "advanced" ? "general" : section) ? "page" : undefined}
            onClick={() => onSectionChange(item.id)}
          >
            {sectionLabels[item.id]}
          </button>
        ))}
      </nav>
      <div className="settings-content">
        {error ? (
          <div className="shell-alert" role="alert">
            {t("chrome.settingsError")}
          </div>
        ) : null}
        {(section === "general" || section === "advanced") ? (
          <GeneralSettings theme={theme} onThemeChange={onThemeChange} />
        ) : null}
        {section === "providers" ? (
          <ProvidersSettings
            profiles={profiles}
            selection={selection}
            onCreateProfile={onCreateProfile}
            onUpdateProfile={onUpdateProfile}
            onDeleteProfile={onDeleteProfile}
            onRefreshProviderProfiles={onRefreshProviderProfiles}
            onSelectionChange={onSelectionChange}
          />
        ) : null}
        {section === "tools" ? (
          <ToolsSettings
            client={settingsClient}
            workspaceId={activeWorkspaceId}
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
            projectTrust={
              <ProjectTrustSettings
                client={settingsClient}
                workspaceId={activeWorkspaceId}
              />
            }
          />
        ) : null}
        {section === "memory" ? (
          activeWorkspaceId ? (
            <MemorySettings
              key={activeWorkspaceId}
              client={settingsClient}
              workspaceId={activeWorkspaceId}
            />
          ) : (
            <section className="settings-section" aria-labelledby="memory-heading">
              <h2 id="memory-heading">{t("settings.sectionMemory")}</h2>
              <p className="placeholder-note">
                {t("memory.needWorkspace")}
              </p>
            </section>
          )
        ) : null}
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
  const { t, locale, setLocale } = useCopy();
  const { skin, setSkin } = useUiSkin();
  return (
    <section className="settings-panel" aria-labelledby="general-settings-title">
      <h1 id="general-settings-title">{t("settings.sectionGeneral")}</h1>
      <p className="lede">{t("settings.generalLede")}</p>
      <div className="settings-card">
        <h2>{t("settings.general.theme")}</h2>
        <div className="settings-segmented" role="group" aria-label={t("settings.general.theme")}>
          <button
            type="button"
            aria-pressed={theme === "light"}
            data-active={theme === "light"}
            onClick={() => onThemeChange("light")}
          >
            {t("settings.general.themeLight")}
          </button>
          <button
            type="button"
            aria-pressed={theme === "dark"}
            data-active={theme === "dark"}
            onClick={() => onThemeChange("dark")}
          >
            {t("settings.general.themeDark")}
          </button>
        </div>
      </div>
      <div className="settings-card">
        <h2>{t("uiSkin.label")}</h2>
        <p className="settings-inline-note">{t("uiSkin.desc")}</p>
        <div className="settings-segmented" role="group" aria-label={t("uiSkin.label")}>
          <button
            type="button"
            aria-pressed={skin === "cool"}
            data-active={skin === "cool"}
            onClick={() => setSkin("cool")}
          >
            {t("uiSkin.cool")}
          </button>
          <button
            type="button"
            aria-pressed={skin === "warm"}
            data-active={skin === "warm"}
            onClick={() => setSkin("warm")}
          >
            {t("uiSkin.warm")}
          </button>
        </div>
      </div>
      <div className="settings-card">
        <h2>{t("settings.general.language")}</h2>
        <p className="settings-inline-note">{t("settings.general.languageDesc")}</p>
        <div className="settings-segmented" role="group" aria-label={t("settings.general.language")}>
          <button
            type="button"
            aria-pressed={locale === "zh-CN"}
            data-active={locale === "zh-CN"}
            onClick={() => setLocale("zh-CN")}
          >
            {t("locale.zhCN")}
          </button>
          <button
            type="button"
            aria-pressed={locale === "en-US"}
            data-active={locale === "en-US"}
            onClick={() => setLocale("en-US")}
          >
            {t("locale.enUS")}
          </button>
        </div>
      </div>
    </section>
  );
}

function ToolsSettings({
  client,
  workspaceId,
  selection,
  defaultApprovalPolicy,
  onSelectionChange,
  onDefaultApprovalPolicyChange,
}: {
  client: SettingsPlatformClient;
  workspaceId: string | null;
  selection: ActiveProviderSelection;
  defaultApprovalPolicy: ProductApprovalPreference;
  onSelectionChange: (selection: ActiveProviderSelection) => Promise<boolean>;
  onDefaultApprovalPolicyChange: (
    policy: ProductApprovalPreference,
  ) => Promise<void>;
}) {
  const { t } = useCopy();
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
    void onSelectionChange({ ...selection, maxSteps: parsed });
  }

  return (
    <section className="settings-panel" aria-labelledby="tools-settings-title">
      <h1 id="tools-settings-title">{t("settings.sectionTools")}</h1>
      <p className="lede">{t("tools.lede")}</p>

      <div className="settings-card">
        <h2>{t("tools.approvalTitle")}</h2>
        <div className="settings-segmented" role="group" aria-label={t("tools.approvalTitle")}>
          {(["ask", "auto", "never"] as const).map((policy) => (
            <button
              key={policy}
              type="button"
              data-active={defaultApprovalPolicy === policy}
              aria-pressed={defaultApprovalPolicy === policy}
              disabled={approvalBusy}
              onClick={() => void handleApprovalChange(policy)}
            >
              {policy === "ask"
                ? t("tools.ask")
                : policy === "auto"
                  ? t("tools.auto")
                  : t("tools.never")}
            </button>
          ))}
        </div>
        <div className="settings-policy-grid">
          <div>
            <strong>{t("tools.ask")}</strong>
            <span>{t("tools.askDesc")}</span>
          </div>
          <div>
            <strong>{t("tools.auto")}</strong>
            <span>{t("tools.autoDesc")}</span>
          </div>
          <div>
            <strong>{t("tools.never")}</strong>
            <span>{t("tools.neverDesc")}</span>
          </div>
        </div>
      </div>

      <form className="settings-card" onSubmit={handleMaxSteps}>
        <h2>{t("tools.maxStepsTitle")}</h2>
        <div className="field settings-number-field">
          <label htmlFor="settings-max-steps">
            {t("tools.maxStepsLabel")}
          </label>
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
            <CheckIcon /> {t("tools.saveDefault")}
          </button>
        </div>
      </form>

      {error ? (
        <div className="chat-error" role="alert">
          {t("chrome.settingsError")}
        </div>
      ) : null}

      <MCPSettings client={client} workspaceId={workspaceId} />
    </section>
  );
}

interface ProviderSettingsProps {
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
  onRefreshProviderProfiles: () => Promise<ProviderProfileRecord[]>;
  onSelectionChange: (selection: ActiveProviderSelection) => Promise<boolean>;
}

function ProvidersSettings(props: ProviderSettingsProps) {
  return desktopProviderCredentialPromptAvailable() ? (
    <DesktopProvidersSettings {...props} />
  ) : (
    <BrowserProvidersSettings {...props} />
  );
}

function BrowserProvidersSettings({
  profiles,
  selection,
  onCreateProfile,
  onUpdateProfile,
  onDeleteProfile,
  onSelectionChange,
}: ProviderSettingsProps) {
  const { t } = useCopy();
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
      catalogRevision: "draft",
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
      setError(describeProviderProbeFailure(testError));
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
      setError(describeProviderProbeFailure(modelsError));
    } finally {
      setModelsBusy(false);
    }
  }

  return (
    <section className="settings-panel" aria-labelledby="providers-settings-title">
      <h1 id="providers-settings-title">{t("settings.providers.title")}</h1>
      <p className="lede">
        {t("settings.providers.desc")}
      </p>

      <div className="settings-card">
        <h2>{t("settings.providers.activeSelection")}</h2>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="provider-mode">{t("settings.providers.mode")}</label>
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
              <option value="default">{t("settings.providers.modeRuntimeDefault")}</option>
              <option value="profile">{t("settings.providers.modeSavedProfile")}</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="provider-profile">{t("settings.providers.profile")}</label>
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
              <option value="">{t("settings.providers.selectProfile")}</option>
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.label}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="provider-model">{t("settings.providers.model")}</label>
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
            ? `${activeProfile.label} · ${activeProfile.apiBase}`
            : t("settings.providers.usingRuntimeDefault")}
        </p>
      </div>

      <form className="settings-card" onSubmit={handleSave}>
        <div className="settings-card__heading">
          <h2>{editingProfileId ? t("settings.providers.editProfile") : t("settings.providers.create")}</h2>
          {editingProfileId ? (
            <button type="button" className="secondary" onClick={resetDraft} disabled={saveBusy}>
              <Cross2Icon /> {t("chrome.cancelEdit")}
            </button>
          ) : null}
        </div>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="profile-label">{t("settings.providers.labelField")}</label>
            <input id="profile-label" value={label} onChange={(event) => setLabel(event.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="profile-type">{t("settings.providers.typeField")}</label>
            <select
              id="profile-type"
              value={providerType}
              onChange={(event) => handleTypeChange(event.target.value as ProviderType)}
            >
              <option value="openai">OpenAI</option>
              <option value="openai-responses">OpenAI Responses</option>
              <option value="anthropic">Anthropic</option>
              <option value="ollama">Ollama</option>
              <option value="fake">{t("settings.providers.localDemo")}</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="profile-base">{t("settings.providers.apiBase")}</label>
            <input id="profile-base" value={apiBase} onChange={(event) => setApiBase(event.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="profile-default-model">{t("settings.providers.defaultModel")}</label>
            <input
              id="profile-default-model"
              value={defaultModel}
              onChange={(event) => setDefaultModel(event.target.value)}
            />
          </div>
        </div>
        <details>
          <summary>{t("chrome.advanced")}</summary>
          <div className="field">
            <label htmlFor="profile-key-env">{t("settings.providers.apiKeyEnv")}</label>
            <input
              id="profile-key-env"
              value={apiKeyEnv}
              onChange={(event) => setApiKeyEnv(event.target.value)}
              disabled={!providerRequiresKey(providerType)}
              placeholder={providerRequiresKey(providerType) ? "OPENAI_API_KEY" : t("common.none")}
            />
            <p className="settings-inline-note">{t("settings.providers.apiKeyEnvHint")}</p>
          </div>
        </details>
        {providerType === "fake" ? <p className="settings-inline-note">{t("settings.providers.localDemoDesc")}</p> : null}
        <div className="field-actions">
          <button type="submit" disabled={saveBusy || profileDeleteBusy}>
            <CheckIcon /> {saveBusy ? t("common.saving") : t("settings.providers.save")}
          </button>
          <button type="button" className="secondary" disabled={testBusy} onClick={() => void handleTest()}>
            {testBusy ? t("common.loading") : t("settings.providers.test")}
          </button>
          <button type="button" className="secondary" disabled={modelsBusy} onClick={() => void handleListModels()}>
            {modelsBusy ? t("common.loading") : t("settings.providers.listModels")}
          </button>
        </div>
        {error ? <div className="chat-error" role="alert">{t("chrome.settingsError")}</div> : null}
        {testResult ? (
          <div className="placeholder-note testline" data-status={testResult.status}>
            {testResult.status === "ok" && (testResult.key_present || !providerRequiresKey(providerType))
              ? t("settings.providers.connected", { count: testResult.models_count })
              : testResult.status === "ok" && !testResult.key_present
                ? t("settings.providers.keyMissing")
                : t("settings.providers.unreachable")}
          </div>
        ) : null}
        {modelsResult ? (
          <div className="placeholder-note">
            {t("chrome.models", { count: modelsResult.models_count })}: {modelsResult.models.slice(0, 12).join(", ") || t("common.none")}
            {modelsResult.models.length > 12 ? "…" : ""}
          </div>
        ) : null}
      </form>

      <div className="settings-card">
        <h2>{t("settings.providers.savedProfiles")}</h2>
        {profiles.length === 0 ? (
          <p className="settings-inline-note">{t("common.noSavedProfiles")}</p>
        ) : (
          <div className="profile-list">
            {profiles.map((profile) => (
              <div className="profile-row" key={profile.id}>
                <div>
                  <strong>{profile.label}</strong>
                  <span>{providerDisplayName(profile.providerType)} · {profile.apiBase}</span>
                  {confirmingDeleteId === profile.id ? (
                    <div className="settings-inline-confirm" role="alert">
                      <span>{t("chrome.removeProvider")}</span>
                      <div className="field-actions">
                        <button type="button" className="secondary" disabled={profileDeleteBusy} onClick={() => setConfirmingDeleteId(null)}>
                          <Cross2Icon /> {t("common.cancel")}
                        </button>
                        <button type="button" className="danger" disabled={profileDeleteBusy} onClick={() => void handleDelete(profile.id)}>
                          <TrashIcon /> {deletingProfileId === profile.id ? t("common.loading") : t("settings.providers.delete")}
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
                    {t("settings.providers.use")}
                  </button>
                  <button type="button" className="secondary" disabled={profileDeleteBusy} onClick={() => startEdit(profile)}>
                    <Pencil2Icon /> {t("settings.providers.edit")}
                  </button>
                  <button type="button" className="danger" disabled={profileDeleteBusy || confirmingDeleteId === profile.id} onClick={() => setConfirmingDeleteId(profile.id)}>
                    <TrashIcon /> {t("settings.providers.delete")}
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

function providerCredentialLabel(profile: ProviderProfileRecord, t: ReturnType<typeof useCopy>["t"]): string {
  switch (profile.credentialSource?.source) {
    case "keyring":
      return t("desktopProviders.credentialKeyring");
    case "env":
      return t("desktopProviders.credentialEnv");
    case "file":
      return t("desktopProviders.credentialFile");
    default:
      return t("desktopProviders.credentialNone");
  }
}

function DesktopProvidersSettings({
  profiles,
  selection,
  onDeleteProfile,
  onRefreshProviderProfiles,
  onSelectionChange,
}: ProviderSettingsProps) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [label, setLabel] = useState(SILICONFLOW_LABEL);
  const [providerType, setProviderType] =
    useState<NativeCredentialProviderType>("openai");
  const [apiBase, setApiBase] = useState(SILICONFLOW_API_BASE);
  const [defaultModel, setDefaultModel] = useState(SILICONFLOW_MODEL);
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null);
  const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [probeResult, setProbeResult] = useState<{
    profileId: string;
    probe: DesktopProviderProbe;
  } | null>(null);
  const [modelResult, setModelResult] = useState<{
    profileId: string;
    models: string[];
  } | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = busyAction !== null;

  // A probe and a model list are both requested from a button, so no effect
  // cleanup can drop them: ask for provider A and then B, and A's slow answer
  // used to render as B's result and clear B's in-flight busy state. One gate per
  // request kind, so only the newest answer for that kind may commit
  // (PI `lib/latest-wins.ts`, plan §8.3).
  const probeGate = useMemo(() => new LatestWinsGate(), []);
  const modelsGate = useMemo(() => new LatestWinsGate(), []);

  const activeProfile = useMemo(
    () => profiles.find((profile) => profile.id === selection.profileId) ?? null,
    [profiles, selection.profileId],
  );

  function clearFeedback(): void {
    setProbeResult(null);
    setModelResult(null);
    setStatus(null);
    setError(null);
  }

  function applySiliconFlowPreset(): void {
    setEditingProfileId(null);
    setLabel(SILICONFLOW_LABEL);
    setProviderType("openai");
    setApiBase(SILICONFLOW_API_BASE);
    setDefaultModel(SILICONFLOW_MODEL);
    clearFeedback();
  }

  function startReconfigure(profile: ProviderProfileRecord): void {
    if (!isNativeCredentialProvider(profile.providerType)) {
      return;
    }
    setEditingProfileId(profile.id);
    setLabel(profile.label);
    setProviderType(profile.providerType);
    setApiBase(profile.apiBase);
    setDefaultModel(profile.defaultModel ?? selection.model);
    clearFeedback();
  }

  async function persistProductSelection(
    profileId: string,
    model: string,
  ): Promise<void> {
    const persisted = await onSelectionChange({
      ...selection,
      mode: "profile",
      profileId,
      model,
    });
    if (!persisted) {
      throw new Error("Provider was selected in the shared Catalog, but Product preferences did not persist.");
    }
  }

  async function handleSecureSave(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    setBusyAction("save");
    clearFeedback();
    try {
      const normalizedBase = apiBase.trim().replace(/\/+$/u, "");
      const normalizedModel = defaultModel.trim();
      const isSiliconFlowPreset =
        providerType === "openai" &&
        normalizedBase === SILICONFLOW_API_BASE &&
        normalizedModel === SILICONFLOW_MODEL;
      const receipt = await promptDesktopProviderCredential({
        profileId:
          editingProfileId ?? (isSiliconFlowPreset ? SILICONFLOW_PROFILE_ID : undefined),
        label: label.trim(),
        providerType,
        apiBase: normalizedBase,
        model: normalizedModel,
        makeDefault: true,
        expectedRevision:
          editingProfileId === null
            ? undefined
            : profiles.find((profile) => profile.id === editingProfileId)
                ?.catalogRevision,
      });
      await onRefreshProviderProfiles();
      await persistProductSelection(receipt.profileId, receipt.model);
      setEditingProfileId(null);
      setProbeResult({ profileId: receipt.profileId, probe: receipt.probe });
      setStatus(
        t("desktopProviders.verified", { label: receipt.label, model: receipt.model }),
      );
    } catch (saveError) {
      setError(describeProviderProbeFailure(saveError));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleProbe(profile: ProviderProfileRecord): Promise<void> {
    const token = probeGate.begin();
    setBusyAction(`probe:${profile.id}`);
    clearFeedback();
    try {
      const probe = await probeDesktopProvider({
        profileId: profile.id,
        model: profile.defaultModel,
      });
      if (!probeGate.isCurrent(token)) {
        return;
      }
      setProbeResult({ profileId: profile.id, probe });
      setStatus(t("desktopProviders.connected", { label: profile.label }));
    } catch (probeError) {
      if (!probeGate.isCurrent(token)) {
        return;
      }
      setError(describeProviderProbeFailure(probeError));
    } finally {
      // A newer probe owns the busy key now; clearing it here would hide that one.
      if (probeGate.isCurrent(token)) {
        setBusyAction(null);
      }
    }
  }

  async function handleListModels(profile: ProviderProfileRecord): Promise<void> {
    const token = modelsGate.begin();
    setBusyAction(`models:${profile.id}`);
    clearFeedback();
    try {
      const response = await client.listProviderModels(profile.id);
      if (!modelsGate.isCurrent(token)) {
        return;
      }
      setModelResult({
        profileId: profile.id,
        models: response.models.map((model) => model.id),
      });
      setStatus(t("desktopProviders.modelsLoaded", { count: response.models.length, label: profile.label }));
    } catch (modelsError) {
      if (!modelsGate.isCurrent(token)) {
        return;
      }
      setError(describeProviderProbeFailure(modelsError));
    } finally {
      if (modelsGate.isCurrent(token)) {
        setBusyAction(null);
      }
    }
  }

  async function handleUse(profile: ProviderProfileRecord): Promise<void> {
    setBusyAction(`use:${profile.id}`);
    clearFeedback();
    try {
      const receipt = await useDesktopProvider({
        profileId: profile.id,
        model: profile.defaultModel,
        expectedRevision: profile.catalogRevision,
      });
      await onRefreshProviderProfiles();
      await persistProductSelection(receipt.profileId, receipt.model);
      setStatus(t("desktopProviders.selected", { label: profile.label, model: receipt.model }));
    } catch (useError) {
      setError(describeProviderProbeFailure(useError));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleDelete(profileId: string): Promise<void> {
    setBusyAction(`delete:${profileId}`);
    clearFeedback();
    try {
      await onDeleteProfile(profileId);
      setConfirmingDeleteId(null);
      if (editingProfileId === profileId) {
        applySiliconFlowPreset();
      }
      setStatus(t("desktopProviders.removed"));
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : String(deleteError));
    } finally {
      setBusyAction(null);
    }
  }

  return (
    <section className="settings-panel" aria-labelledby="providers-settings-title">
      <h1 id="providers-settings-title">{t("settings.providers.title")}</h1>
      <p className="lede">
        {t("desktopProviders.description")}
      </p>

      <div className="settings-card">
        <h2>{t("settings.providers.activeSelection")}</h2>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="provider-mode">{t("settings.providers.mode")}</label>
            <select
              id="provider-mode"
              value={selection.mode}
              disabled={busy}
              onChange={(event) => {
                if (event.target.value === "default") {
                  void onSelectionChange({
                    ...selection,
                    mode: "default",
                    profileId: undefined,
                  });
                  return;
                }
                const profile = activeProfile ?? profiles[0];
                if (profile) {
                  void handleUse(profile);
                }
              }}
            >
              <option value="default">{t("settings.providers.modeRuntimeDefault")}</option>
              <option value="profile">{t("settings.providers.modeSavedProfile")}</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="provider-profile">{t("settings.providers.profile")}</label>
            <select
              id="provider-profile"
              value={selection.profileId ?? ""}
              disabled={busy || selection.mode !== "profile"}
              onChange={(event) => {
                const profile = profiles.find((item) => item.id === event.target.value);
                if (profile) {
                  void handleUse(profile);
                }
              }}
            >
              <option value="">{t("settings.providers.selectProfile")}</option>
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.label}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="provider-model">{t("settings.providers.model")}</label>
            <input
              id="provider-model"
              value={selection.model}
              disabled={busy}
              onChange={(event) =>
                void onSelectionChange({ ...selection, model: event.target.value })
              }
            />
          </div>
        </div>
        <p className="settings-inline-note">
          {selection.mode === "profile" && activeProfile
            ? `${activeProfile.label} · ${activeProfile.apiBase} · ${providerCredentialLabel(activeProfile, t)}`
            : t("desktopProviders.defaultSelection")}
        </p>
      </div>

      <form className="settings-card" onSubmit={(event) => void handleSecureSave(event)}>
        <div className="settings-card__heading">
          <div>
            <h2>{editingProfileId ? t("settings.providers.editSecure") : t("settings.providers.secureOnboarding")}</h2>
            <p className="settings-inline-note">
              {t("desktopProviders.onboardingHint")}
            </p>
          </div>
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={applySiliconFlowPreset}
          >
            {t("desktopProviders.preset")}
          </button>
        </div>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="profile-label">{t("settings.providers.labelField")}</label>
            <input
              id="profile-label"
              value={label}
              disabled={busy}
              onChange={(event) => setLabel(event.target.value)}
            />
          </div>
          <div className="field">
            <label htmlFor="profile-type">{t("settings.providers.typeField")}</label>
            <select
              id="profile-type"
              value={providerType}
              disabled={busy}
              onChange={(event) => {
                const next = event.target.value as NativeCredentialProviderType;
                setProviderType(next);
                setApiBase(providerDefaultApiBase(next));
                clearFeedback();
              }}
            >
              <option value="openai">{t("desktopProviders.compatible")}</option>
              <option value="openai-responses">{t("settings.providers.typeOpenaiResponses")}</option>
              <option value="anthropic">{t("settings.providers.typeAnthropic")}</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="profile-base">{t("settings.providers.apiBase")}</label>
            <input
              id="profile-base"
              value={apiBase}
              disabled={busy}
              onChange={(event) => setApiBase(event.target.value)}
            />
          </div>
          <div className="field">
            <label htmlFor="profile-default-model">{t("settings.providers.modelField")}</label>
            <input
              id="profile-default-model"
              value={defaultModel}
              disabled={busy}
              onChange={(event) => setDefaultModel(event.target.value)}
            />
          </div>
        </div>
        <div className="placeholder-note">
          {t("desktopProviders.credentialHint")}
        </div>
        <div className="field-actions">
          <button type="submit" disabled={busy}>
            <CheckIcon /> {busyAction === "save" ? t("desktopProviders.verifying") : t("desktopProviders.saveVerify")}
          </button>
          {editingProfileId ? (
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={applySiliconFlowPreset}
            >
              <Cross2Icon /> {t("common.cancel")}
            </button>
          ) : null}
        </div>
      </form>

      {error ? <div className="chat-error" role="alert">{t("chrome.settingsError")}</div> : null}
      {status ? <div className="placeholder-note" role="status">{status}</div> : null}
      {probeResult ? (
        <div className="placeholder-note">
          {t("desktopProviders.probeSummary", {
            count: probeResult.probe.inventoryCount,
            streaming: t(probeResult.probe.streamingSupported ? "desktopProviders.supported" : "desktopProviders.unsupported"),
            tools: t(probeResult.probe.nativeToolCallsSupported ? "desktopProviders.supported" : "desktopProviders.unsupported"),
            usage: t(probeResult.probe.usageSupported ? "desktopProviders.supported" : "desktopProviders.unsupported"),
          })}
        </div>
      ) : null}
      {modelResult ? (
        <div className="placeholder-note">
          {t("desktopProviders.models", { models: modelResult.models.slice(0, 12).join(", ") || t("common.none") })}
          {modelResult.models.length > 12 ? "…" : ""}
        </div>
      ) : null}

      <div className="settings-card">
        <h2>{t("settings.providers.catalogProfiles")}</h2>
        {profiles.length === 0 ? (
          <p className="settings-inline-note">
            {t("desktopProviders.empty")}
          </p>
        ) : (
          <div className="profile-list">
            {profiles.map((profile) => {
              const deleting = busyAction === `delete:${profile.id}`;
              return (
                <div className="profile-row" key={profile.id}>
                  <div>
                    <strong>
                      {profile.label}
                      {selection.profileId === profile.id ? t("desktopProviders.active") : ""}
                    </strong>
                    <span>
                      {providerDisplayName(profile.providerType)} · {profile.apiBase} · {" "}
                      {profile.defaultModel ?? t("desktopProviders.noDefaultModel")} · {providerCredentialLabel(profile, t)}
                    </span>
                    {confirmingDeleteId === profile.id ? (
                      <div className="settings-inline-confirm" role="alert">
                        <span>{t("settings.providers.removeCatalogConfirm")}</span>
                        <div className="field-actions">
                          <button
                            type="button"
                            className="secondary"
                            disabled={busy}
                            onClick={() => setConfirmingDeleteId(null)}
                          >
                            <Cross2Icon /> {t("common.cancel")}
                          </button>
                          <button
                            type="button"
                            className="danger"
                            disabled={busy}
                            onClick={() => void handleDelete(profile.id)}
                          >
                            <TrashIcon /> {deleting ? t("common.loading") : t("settings.providers.delete")}
                          </button>
                        </div>
                      </div>
                    ) : null}
                  </div>
                  <div className="field-actions">
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() => void handleProbe(profile)}
                    >
                      {busyAction === `probe:${profile.id}` ? t("common.loading") : t("settings.providers.test")}
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() => void handleListModels(profile)}
                    >
                      {busyAction === `models:${profile.id}` ? t("common.loading") : t("settings.providers.listModels")}
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() => void handleUse(profile)}
                    >
                      {busyAction === `use:${profile.id}` ? t("desktopProviders.selecting") : t("settings.providers.use")}
                    </button>
                    {isNativeCredentialProvider(profile.providerType) ? (
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() => startReconfigure(profile)}
                      >
                        <Pencil2Icon /> {t("desktopProviders.reconfigure")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="danger"
                      disabled={busy || confirmingDeleteId === profile.id}
                      onClick={() => setConfirmingDeleteId(profile.id)}
                    >
                      <TrashIcon /> {t("desktopProviders.remove")}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
