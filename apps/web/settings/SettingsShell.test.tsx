import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTranslator, type Locale } from "../copy";
import type { ProviderProfileRecord } from "../state/product-types";
import { SettingsShell, type SettingsShellProps } from "./SettingsShell";
import { createSettingsPlatformClient } from "./settings-platform-client";

const state = vi.hoisted(() => ({ locale: "zh-CN" as Locale, values: [] as unknown[] }));
vi.mock("../copy/CopyProvider", () => ({
  useCopy: () => ({ t: createTranslator(state.locale) }),
}));
vi.mock("../platform/desktop-commands", () => ({
  desktopProviderCredentialPromptAvailable: () => true,
}));
vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useState: (initial: unknown) => actual.useState(state.values.length ? state.values.shift() : initial),
  };
});

const profile: ProviderProfileRecord = {
  id: "internal-profile-id",
  label: "Example service",
  providerType: "openai",
  apiBase: "https://example.com/v1",
  defaultModel: "example-model",
  updatedAt: "2026-09-13T00:00:00Z",
  catalogRevision: "internal-revision",
  credentialSource: { source: "keyring", service: "internal-service", account: "internal-account" },
};
const props: SettingsShellProps = {
  section: "providers", onSectionChange: vi.fn(),
  settingsClient: createSettingsPlatformClient(),
  profiles: [profile], selection: { mode: "profile", profileId: profile.id, model: "example-model", approval: "ask", maxSteps: 8 },
  defaultApprovalPolicy: "ask", onCreateProfile: vi.fn(), onUpdateProfile: vi.fn(),
  onDeleteProfile: vi.fn(), onRefreshProviderProfiles: vi.fn(), onSelectionChange: vi.fn(),
  onDefaultApprovalPolicyChange: vi.fn(), workspaces: [], sessions: [],
  activeWorkspaceId: null, activeSessionId: null, onSelectWorkspace: vi.fn(),
  onSelectSession: vi.fn(), onTogglePin: vi.fn(), onRemoveWorkspace: vi.fn(),
  onRenameSession: vi.fn(), onDeleteSession: vi.fn(), connectionLabel: "", theme: "light",
  onThemeChange: vi.fn(), error: null,
};

beforeEach(() => { state.values = []; });

describe.each(["zh-CN", "en-US"] as const)("Desktop provider copy (%s)", (locale) => {
  beforeEach(() => { state.locale = locale; });

  it("localizes controls and credential sources without exposing storage details", () => {
    const t = createTranslator(locale);
    const html = renderToStaticMarkup(<SettingsShell {...props} />);
    expect(html).toContain(t("desktopProviders.preset"));
    expect(html).toContain(t("desktopProviders.credentialKeyring"));
    expect(html).toContain(t("desktopProviders.reconfigure"));
    expect(html).toContain(t("settings.providers.apiBase"));
    expect(html).toContain('value="openai-responses"');
    expect(html).toContain('value="anthropic"');
    expect(html).not.toMatch(/OS keyring|keyring reference|React state|localStorage|internal-service|internal-account|internal-revision/);
    if (locale === "zh-CN") expect(html).not.toMatch(/Providers &amp; Models|Save &amp; verify|Reconfigure|API base/);
  });

  it("renders connection capabilities in human terms, not profile IDs or raw flags", () => {
    state.values = ["Example", "openai", profile.apiBase, "example-model", null, null, null,
      { profileId: profile.id, probe: { inventoryCount: 3, streamingSupported: true, nativeToolCallsSupported: false, usageSupported: true } },
      { profileId: profile.id, models: ["example-model"] }, null, null];
    const html = renderToStaticMarkup(<SettingsShell {...props} />);
    expect(html).toContain(locale === "zh-CN" ? "连接测试：3 个模型可用" : "Connection test: 3 models available");
    expect(html).toContain(locale === "zh-CN" ? "使用工具：不支持" : "Tool use: Not supported");
    expect(html).toContain(locale === "zh-CN" ? "用量统计：支持" : "Usage tracking: Supported");
    expect(html).toContain(locale === "zh-CN" ? "可用模型：example-model" : "Available models: example-model");
    const text = html.replace(/<[^>]+>/g, "");
    expect(text).not.toMatch(/internal-profile-id|Probe for|native tools|streaming yes|true|false/);
  });

  it("localizes default and empty states", () => {
    const html = renderToStaticMarkup(<SettingsShell {...props} profiles={[]} selection={{ ...props.selection, mode: "default" }} />);
    const t = createTranslator(locale);
    expect(html).toContain(t("desktopProviders.defaultSelection"));
    expect(html).toContain(t("desktopProviders.empty"));
  });
});
