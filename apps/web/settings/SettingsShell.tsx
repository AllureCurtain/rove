"use client";

import { FormEvent, useMemo, useState } from "react";

import { listProviderModels, testProvider } from "../api/run-controller";
import type { ProviderModelsResponse, ProviderTestResponse, ProviderType } from "../lib/rove-types";
import {
  providerDefaultApiBase,
  providerDefaultKeyEnv,
  providerDisplayName,
  providerRequiresKey,
  removeProviderProfile,
  upsertProviderProfile,
} from "../state/provider-store";
import type {
  ActiveProviderSelection,
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
  onProfilesChange,
  onSelectionChange,
  connectionLabel,
  theme,
  onThemeChange,
}: {
  section: SettingsSectionId;
  onSectionChange: (section: SettingsSectionId) => void;
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  onProfilesChange: (profiles: ProviderProfileRecord[]) => void;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
  connectionLabel: string;
  theme: "light" | "dark";
  onThemeChange: (theme: "light" | "dark") => void;
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
        {section === "providers" ? (
          <ProvidersSettings
            profiles={profiles}
            selection={selection}
            onProfilesChange={onProfilesChange}
            onSelectionChange={onSelectionChange}
          />
        ) : null}
        {section === "about" ? (
          <AboutSettings connectionLabel={connectionLabel} theme={theme} />
        ) : null}
        {section === "general" ? (
          <GeneralSettings theme={theme} onThemeChange={onThemeChange} />
        ) : null}
        {section !== "providers" && section !== "about" && section !== "general" ? (
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
        This section is intentionally a placeholder. Providers & Models and About / Runtime are
        the deep M1 settings surfaces.
      </div>
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
  onProfilesChange,
  onSelectionChange,
}: {
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  onProfilesChange: (profiles: ProviderProfileRecord[]) => void;
  onSelectionChange: (selection: ActiveProviderSelection) => void;
}) {
  const [label, setLabel] = useState("Local OpenAI");
  const [providerType, setProviderType] = useState<ProviderType>("openai");
  const [apiBase, setApiBase] = useState(providerDefaultApiBase("openai"));
  const [apiKeyEnv, setApiKeyEnv] = useState(providerDefaultKeyEnv("openai"));
  const [defaultModel, setDefaultModel] = useState("gpt-4.1-mini");
  const [testBusy, setTestBusy] = useState(false);
  const [modelsBusy, setModelsBusy] = useState(false);
  const [testResult, setTestResult] = useState<ProviderTestResponse | null>(null);
  const [modelsResult, setModelsResult] = useState<ProviderModelsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  function handleSave(event: FormEvent) {
    event.preventDefault();
    const next = upsertProviderProfile(profiles, {
      label: label.trim() || providerDisplayName(providerType),
      providerType,
      apiBase: apiBase.trim(),
      apiKeyEnv: providerRequiresKey(providerType)
        ? apiKeyEnv.trim() || providerDefaultKeyEnv(providerType)
        : undefined,
      defaultModel: defaultModel.trim() || undefined,
    });
    onProfilesChange(next);
    const saved = next[0];
    if (saved) {
      onSelectionChange({
        ...selection,
        mode: "profile",
        profileId: saved.id,
        model: saved.defaultModel || selection.model,
      });
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
        Profiles live in local browser storage for M1. Only environment variable names are stored
        — never raw keys.
      </p>

      <div className="settings-card">
        <h2>Active selection</h2>
        <div className="field-grid">
          <div className="field">
            <label htmlFor="provider-mode">Mode</label>
            <select
              id="provider-mode"
              value={selection.mode}
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
              disabled={selection.mode !== "profile"}
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
          <button type="submit">Save profile</button>
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
                    onClick={() => onProfilesChange(removeProviderProfile(profiles, profile.id))}
                  >
                    Remove
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
