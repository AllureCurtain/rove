"use client";

import { FormEvent, useMemo, useState } from "react";

import { listProviderModels, testProvider } from "../api/run-controller";
import { BenchmarkPanel } from "../components/benchmark-panel";
import type { ProviderModelsResponse, ProviderTestResponse, ProviderType } from "../lib/rove-types";
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
} from "../state/product-types";
import { toApiProviderProfile } from "../state/product-types";
import type { SettingsSectionId } from "./sections";
import { SETTINGS_SECTIONS } from "./sections";

export function SettingsShell({
  section,
  onSectionChange,
  profiles,
  selection,
  onCreateProfile,
  onDeleteProfile,
  onSelectionChange,
  connectionLabel,
  theme,
  onThemeChange,
  error,
}: {
  section: SettingsSectionId;
  onSectionChange: (section: SettingsSectionId) => void;
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  onCreateProfile: (profile: ProviderProfileInput) => Promise<ProviderProfileRecord>;
  onDeleteProfile: (profileId: string) => Promise<void>;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
  connectionLabel: string;
  theme: "light" | "dark";
  onThemeChange: (theme: "light" | "dark") => void;
  error: string | null;
}) {
  return (
    <div className="settings-shell">
      <nav className="settings-nav" aria-label="Settings sections">
        <h2>Settings</h2>
        {SETTINGS_SECTIONS.map((item) => (
          <button
            key={item.id}
            type="button"
            data-active={item.id === section}
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
        {section === "providers" ? (
          <ProvidersSettings
            profiles={profiles}
            selection={selection}
            onCreateProfile={onCreateProfile}
            onDeleteProfile={onDeleteProfile}
            onSelectionChange={onSelectionChange}
          />
        ) : null}
        {section === "about" ? (
          <AboutSettings connectionLabel={connectionLabel} theme={theme} />
        ) : null}
        {section === "general" ? (
          <GeneralSettings theme={theme} onThemeChange={onThemeChange} />
        ) : null}
        {section === "advanced" ? <AdvancedSettings /> : null}
        {section !== "providers" &&
        section !== "about" &&
        section !== "general" &&
        section !== "advanced" ? (
          <PlaceholderSettings section={section} />
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
    <div className="settings-panel">
      <h1>General</h1>
      <p className="lede">Appearance and basic product preferences.</p>
      <div className="settings-card">
        <h2>Theme</h2>
        <div className="field-actions">
          <button
            type="button"
            className={theme === "light" ? undefined : "secondary"}
            onClick={() => onThemeChange("light")}
          >
            Light
          </button>
          <button
            type="button"
            className={theme === "dark" ? undefined : "secondary"}
            onClick={() => onThemeChange("dark")}
          >
            Dark
          </button>
        </div>
      </div>
    </div>
  );
}

function PlaceholderSettings({ section }: { section: SettingsSectionId }) {
  const label = SETTINGS_SECTIONS.find((item) => item.id === section)?.label ?? section;
  return (
    <div className="settings-panel">
      <h1>{label}</h1>
      <p className="lede">Scaffolded for M1. Deep implementation lands in later waves.</p>
      <div className="placeholder-note">
        This section is intentionally a placeholder. Providers & Models, About / Runtime, and
        Advanced / Developer (Benchmark) are the deeper M1 settings surfaces.
      </div>
    </div>
  );
}

function AdvancedSettings() {
  const [showBenchmark, setShowBenchmark] = useState(false);

  return (
    <div className="settings-panel">
      <h1>Advanced / Developer</h1>
      <p className="lede">
        Escape hatches for power users. Benchmark is intentionally not primary product navigation.
      </p>
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
            <span>
              Deterministic evaluation suites against the API. Hidden from primary chat IA.
            </span>
          </button>
          <a className="advanced-link-card" href="/dev/workbench">
            Legacy workbench scaffold
            <span>
              Temporary migration console only. Product entry remains the shell at `/`.
            </span>
          </a>
        </div>
      </div>
      {showBenchmark ? (
        <div className="advanced-benchmark" aria-label="Benchmark runner">
          <BenchmarkPanel />
        </div>
      ) : null}
    </div>
  );
}

function AboutSettings({
  connectionLabel,
  theme,
}: {
  connectionLabel: string;
  theme: "light" | "dark";
}) {
  return (
    <div className="settings-panel">
      <h1>About / Runtime</h1>
      <p className="lede">Connection and host basics for this Web management surface.</p>
      <div className="settings-card">
        <h2>Connection</h2>
        <div className="inspector-kv">
          <div>
            <span>API proxy</span>
            <strong>/api → rove-api</strong>
          </div>
          <div>
            <span>status</span>
            <strong>{connectionLabel}</strong>
          </div>
          <div>
            <span>host</span>
            <strong>web</strong>
          </div>
          <div>
            <span>theme</span>
            <strong>{theme}</strong>
          </div>
        </div>
      </div>
      <div className="settings-card">
        <h2>Product model</h2>
        <p style={{ margin: 0, color: "var(--muted)", lineHeight: 1.5 }}>
          Workspace → Session → Run. Session continue uses hard resume only. Browser never sends
          raw provider keys; only `api_key_env` names are stored and forwarded.
        </p>
      </div>
    </div>
  );
}

function ProvidersSettings({
  profiles,
  selection,
  onCreateProfile,
  onDeleteProfile,
  onSelectionChange,
}: {
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  onCreateProfile: (profile: ProviderProfileInput) => Promise<ProviderProfileRecord>;
  onDeleteProfile: (profileId: string) => Promise<void>;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
}) {
  const [label, setLabel] = useState("Local OpenAI");
  const [providerType, setProviderType] = useState<ProviderType>("openai");
  const [apiBase, setApiBase] = useState(providerDefaultApiBase("openai"));
  const [apiKeyEnv, setApiKeyEnv] = useState(providerDefaultKeyEnv("openai"));
  const [defaultModel, setDefaultModel] = useState("gpt-4.1-mini");
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

  function draftProfile(): ProviderProfileRecord {
    return {
      id: "draft",
      label: label.trim() || providerDisplayName(providerType),
      providerType,
      apiBase: apiBase.trim(),
      apiKeyEnv: providerRequiresKey(providerType)
        ? apiKeyEnv.trim() || providerDefaultKeyEnv(providerType)
        : undefined,
      defaultModel: defaultModel.trim() || undefined,
      updatedAt: new Date().toISOString(),
    };
  }

  function handleTypeChange(next: ProviderType) {
    setProviderType(next);
    setApiBase(providerDefaultApiBase(next));
    setApiKeyEnv(providerDefaultKeyEnv(next));
  }

  async function handleSave(event: FormEvent) {
    event.preventDefault();
    setSaveBusy(true);
    setError(null);
    try {
      const saved = await onCreateProfile({
        label: label.trim() || providerDisplayName(providerType),
        providerType,
        apiBase: apiBase.trim(),
        apiKeyEnv: providerRequiresKey(providerType)
          ? apiKeyEnv.trim() || providerDefaultKeyEnv(providerType)
          : undefined,
        defaultModel: defaultModel.trim() || undefined,
      });
      onSelectionChange({
        ...selection,
        mode: "profile",
        profileId: saved.id,
        model: saved.defaultModel || selection.model,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaveBusy(false);
    }
  }

  async function handleDelete(profileId: string) {
    setDeletingProfileId(profileId);
    setError(null);
    try {
      await onDeleteProfile(profileId);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeletingProfileId(null);
    }
  }

  async function handleTest() {
    setTestBusy(true);
    setError(null);
    setTestResult(null);
    try {
      const provider = toApiProviderProfile(draftProfile());
      const result = await testProvider({
        provider,
        model: defaultModel.trim() || undefined,
      });
      setTestResult(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTestBusy(false);
    }
  }

  async function handleListModels() {
    setModelsBusy(true);
    setError(null);
    setModelsResult(null);
    try {
      const provider = toApiProviderProfile(draftProfile());
      const result = await listProviderModels({ provider });
      setModelsResult(result);
      if (result.models[0] && !defaultModel.trim()) {
        setDefaultModel(result.models[0]);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setModelsBusy(false);
    }
  }

  return (
    <div className="settings-panel">
      <h1>Providers & Models</h1>
      <p className="lede">
        Profiles are stored by the local rove API. Only environment variable names are persisted,
        never raw keys.
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
              onChange={(event) =>
                onSelectionChange({
                  ...selection,
                  mode: "profile",
                  profileId: event.target.value || undefined,
                })
              }
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
          <div className="field">
            <label htmlFor="provider-approval">Approval policy</label>
            <select
              id="provider-approval"
              value={selection.approval}
              disabled={profileDeleteBusy}
              onChange={(event) =>
                onSelectionChange({
                  ...selection,
                  approval: event.target.value as ActiveProviderSelection["approval"],
                })
              }
            >
              <option value="ask">Ask</option>
              <option value="auto">Auto</option>
              <option value="never">Never</option>
            </select>
          </div>
        </div>
        {selection.mode === "profile" && activeProfile ? (
          <p style={{ margin: 0, color: "var(--muted)", fontSize: "0.9rem" }}>
            Using {activeProfile.label} · {activeProfile.apiBase}
            {activeProfile.apiKeyEnv ? ` · env ${activeProfile.apiKeyEnv}` : ""}
          </p>
        ) : (
          <p style={{ margin: 0, color: "var(--muted)", fontSize: "0.9rem" }}>
            Using the API process default provider configuration.
          </p>
        )}
      </div>

      <form className="settings-card" onSubmit={handleSave}>
        <h2>Edit / save profile</h2>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="profile-label">Label</label>
            <input id="profile-label" value={label} onChange={(e) => setLabel(e.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="profile-type">Type</label>
            <select
              id="profile-type"
              value={providerType}
              onChange={(e) => handleTypeChange(e.target.value as ProviderType)}
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
            <input id="profile-base" value={apiBase} onChange={(e) => setApiBase(e.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="profile-key-env">API key env name</label>
            <input
              id="profile-key-env"
              value={apiKeyEnv}
              onChange={(e) => setApiKeyEnv(e.target.value)}
              disabled={!providerRequiresKey(providerType)}
              placeholder={providerRequiresKey(providerType) ? "OPENAI_API_KEY" : "not required"}
            />
          </div>
          <div className="field">
            <label htmlFor="profile-default-model">Default model</label>
            <input
              id="profile-default-model"
              value={defaultModel}
              onChange={(e) => setDefaultModel(e.target.value)}
            />
          </div>
        </div>
        <div className="field-actions">
          <button type="submit" disabled={saveBusy || profileDeleteBusy}>
            {saveBusy ? "Saving…" : "Save profile"}
          </button>
          <button type="button" className="secondary" disabled={testBusy} onClick={handleTest}>
            {testBusy ? "Testing…" : "Test"}
          </button>
          <button
            type="button"
            className="secondary"
            disabled={modelsBusy}
            onClick={handleListModels}
          >
            {modelsBusy ? "Loading…" : "List models"}
          </button>
        </div>
        {error ? <div className="chat-error">{error}</div> : null}
        {testResult ? (
          <div className="placeholder-note">
            Test: {testResult.status} · key_present={String(testResult.key_present)} · models=
            {testResult.models_count}
            {testResult.wire_protocol ? ` · wire ${testResult.wire_protocol}` : ""}
          </div>
        ) : null}
        {modelsResult ? (
          <div className="placeholder-note">
            Models ({modelsResult.models_count}):{" "}
            {modelsResult.models.slice(0, 12).join(", ") || "(none)"}
            {modelsResult.models.length > 12 ? "…" : ""}
          </div>
        ) : null}
      </form>

      <div className="settings-card">
        <h2>Saved profiles</h2>
        {profiles.length === 0 ? (
          <p style={{ margin: 0, color: "var(--muted)" }}>No saved profiles yet.</p>
        ) : (
          <div className="profile-list">
            {profiles.map((profile) => (
              <div className="profile-row" key={profile.id}>
                <div>
                  <strong>{profile.label}</strong>
                  <span>
                    {providerDisplayName(profile.providerType)} · {profile.apiBase}
                  </span>
                </div>
                <div className="field-actions">
                  <button
                    type="button"
                    className="secondary"
                    disabled={profileDeleteBusy}
                    onClick={() =>
                      onSelectionChange({
                        ...selection,
                        mode: "profile",
                        profileId: profile.id,
                        model: profile.defaultModel || selection.model,
                      })
                    }
                  >
                    Use
                  </button>
                  <button
                    type="button"
                    className="danger"
                    disabled={profileDeleteBusy}
                    onClick={() => void handleDelete(profile.id)}
                  >
                    {deletingProfileId === profile.id ? "Removing…" : "Remove"}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
