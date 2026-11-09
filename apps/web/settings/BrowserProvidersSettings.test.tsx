// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createTranslator } from "../copy";
import type { ProviderProfileRecord } from "../state/product-types";
import { SettingsShell, type SettingsShellProps } from "./SettingsShell";

const mocks = vi.hoisted(() => ({
  onboard: vi.fn(), listSaved: vi.fn(), test: vi.fn(), listDraft: vi.fn(), notify: vi.fn(),
}));
vi.mock("../platform/desktop-commands", () => ({ desktopProviderCredentialPromptAvailable: () => false }));
vi.mock("../product/product-client", () => ({ createProductApiClient: () => ({ onboardProvider: mocks.onboard, listProviderModels: mocks.listSaved }) }));
vi.mock("../api/run-controller", () => ({ testProvider: mocks.test, listProviderModels: mocks.listDraft }));
vi.mock("../shell/toast/ToastProvider", () => ({ useToast: () => ({ notify: mocks.notify }) }));
vi.mock("../copy/CopyProvider", () => ({ useCopy: () => ({ t: createTranslator("en-US") }) }));

const profile: ProviderProfileRecord = {
  id: "saved-profile", label: "Example", providerType: "openai", apiBase: "https://example.test/v1",
  defaultModel: "model-a", updatedAt: "2026-11-09T00:00:00Z", catalogRevision: "revision-1",
  credentialSource: { source: "keyring", service: "service", account: "account" },
};
let props: SettingsShellProps;
let container: HTMLDivElement;
let root: Root;
beforeEach(async () => {
  vi.clearAllMocks();
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Element.prototype.scrollIntoView = vi.fn();
  props = {
    section: "providers", onSectionChange: vi.fn(), settingsClient: {} as SettingsShellProps["settingsClient"],
    profiles: [profile], selection: { mode: "default", model: "fake", approval: "ask", maxSteps: 8 },
    defaultApprovalPolicy: "ask", onCreateProfile: vi.fn(async () => profile), onUpdateProfile: vi.fn(async () => profile),
    onDeleteProfile: vi.fn(), onRefreshProviderProfiles: vi.fn(async () => [profile]), onSelectionChange: vi.fn(async () => true),
    onDefaultApprovalPolicyChange: vi.fn(), workspaces: [], sessions: [], activeWorkspaceId: null,
    activeSessionId: null, onSelectWorkspace: vi.fn(), onSelectSession: vi.fn(), onTogglePin: vi.fn(),
    onRemoveWorkspace: vi.fn(), onRenameSession: vi.fn(), onDeleteSession: vi.fn(), connectionLabel: "", theme: "light",
    onThemeChange: vi.fn(), error: null,
  };
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => root.render(<SettingsShell {...props} />));
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove();
});
function button(text: string) {
  const found = Array.from(container.querySelectorAll<HTMLButtonElement>("button"))
    .find((element) => element.textContent?.trim() === text);
  expect(found, text).toBeDefined(); return found!;
}
async function click(text: string) { await act(async () => button(text).click()); }
async function fill(id: string, value: string) {
  const input = container.querySelector<HTMLInputElement>(`#${id}`)!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function selectType(value: string) {
  await act(async () => {
    const select = container.querySelector<HTMLSelectElement>("#profile-type")!;
    select.value = value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

describe("browser provider setup", () => {
  it("validates the address beside its field before making a request", async () => {
    await fill("profile-base", "not-a-url"); await click("Save changes");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("complete http:// or https://");
    expect(container.querySelector('#profile-base')?.getAttribute('aria-invalid')).toBe("true");
    expect(document.activeElement?.id).toBe("profile-base");
    expect(props.onCreateProfile).not.toHaveBeenCalled(); expect(mocks.onboard).not.toHaveBeenCalled();
  });

  it("preserves keyring credentials when renaming a saved profile", async () => {
    await click("Edit"); await fill("profile-label", "Renamed"); await click("Save changes");
    expect(props.onUpdateProfile).toHaveBeenCalledWith(profile.id, expect.objectContaining({ label: "Renamed", apiKeyEnv: undefined }));
    expect(container.textContent).toContain("Profile saved");
  });

  it("tests saved credentials through the catalog without falling back to an environment variable", async () => {
    mocks.listSaved.mockResolvedValue({ profile_id: profile.id, models: [{ id: "model-a" }] });
    await click("Edit"); await click("Test connection");
    expect(mocks.listSaved).toHaveBeenCalledWith(profile.id); expect(mocks.test).not.toHaveBeenCalled();
    expect(container.querySelector('.testline')?.textContent).toContain("Connected");
    await fill("profile-default-model", "missing-model"); await click("Test connection");
    expect(container.querySelector('.testline')?.textContent).toContain("no usable models");
  });

  it("preserves stored credentials across compatible protocol changes", async () => {
    await click("Edit"); await selectType("openai-responses");
    await fill("profile-default-model", "model-a"); await click("Save changes");
    expect(props.onUpdateProfile).toHaveBeenCalledWith(profile.id, expect.objectContaining({ providerType: "openai-responses", apiKeyEnv: undefined }));
  });

  it("requires an explicit credential source when changing authentication style", async () => {
    await click("Edit"); await selectType("anthropic"); await fill("profile-default-model", "model-b");
    await click("Save changes");
    expect(props.onUpdateProfile).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("different authentication");
    await fill("profile-key-env", "ANTHROPIC_API_KEY"); await click("Save changes");
    expect(props.onUpdateProfile).toHaveBeenCalledWith(profile.id, expect.objectContaining({ apiKeyEnv: "ANTHROPIC_API_KEY" }));
  });

  it("does not report an unavailable env-backed model as connected", async () => {
    mocks.test.mockResolvedValue({ status: "pass", key_present: true, model_present: false, models_count: 1 });
    await click("Test connection");
    expect(container.querySelector('.testline')?.textContent).toContain("no usable models");
    await fill("profile-default-model", "another-model");
    expect(container.querySelector('.testline')).toBeNull();
  });

  it("does not test a changed endpoint using an old saved identity", async () => {
    await click("Edit"); await fill("profile-base", "https://other.test/v1"); await click("Test connection");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Save them before testing");
    expect(mocks.listSaved).not.toHaveBeenCalled(); expect(mocks.test).not.toHaveBeenCalled();
  });

  it("directs a draft key only to onboarding and clears it even on failure", async () => {
    const input = container.querySelector<HTMLInputElement>('#profile-api-key')!;
    expect(input.disabled).toBe(false);
    await fill("profile-api-key", "fixture-credential");
    expect(button("Test connection").disabled).toBe(true); expect(button("List models").disabled).toBe(true);
    mocks.onboard.mockRejectedValue({ code: "provider_authentication", message: "fixture-credential" });
    await click("Verify and save");
    expect(mocks.onboard).toHaveBeenCalledWith(expect.objectContaining({ credential: "fixture-credential" }));
    expect(mocks.test).not.toHaveBeenCalled(); expect(mocks.listDraft).not.toHaveBeenCalled();
    expect(input.value).toBe(""); expect(container.textContent).not.toContain("fixture-credential");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("authentication failed");
  });

  it("clears the uncontrolled credential immediately while verification is pending", async () => {
    let reject!: (error: unknown) => void;
    mocks.onboard.mockReturnValue(new Promise((_resolve, fail) => { reject = fail; }));
    await fill("profile-api-key", "fixture-pending-key");
    await click("Verify and save");
    const input = container.querySelector<HTMLInputElement>("#profile-api-key")!;
    expect(input.value).toBe("");
    expect(input.disabled).toBe(true);
    expect(mocks.onboard).toHaveBeenCalledOnce();
    await act(async () => reject({ code: "provider_authentication" }));
    expect(input.value).toBe("");
    expect(input.disabled).toBe(false);
  });

  it("explains a saved profile when updating the default selection fails", async () => {
    vi.mocked(props.onSelectionChange).mockResolvedValue(false);
    await click("Save changes");
    expect(props.onCreateProfile).toHaveBeenCalledOnce(); expect(props.onSelectionChange).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("Profile saved, but the new-session default was not updated");
  });
});
