"use client";

import {
  CheckIcon,
  ChevronDownIcon,
  Cross2Icon,
  MixerHorizontalIcon,
  ReloadIcon,
} from "@radix-ui/react-icons";
import { useEffect, useId, useMemo, useRef, useState } from "react";

import type {
  ProviderProfileRecord,
  SessionModelConfig,
  SessionModelConfigInput,
} from "../state/product-types";
import {
  PRODUCT_REASONING_PREFERENCES,
  type ProductModelDescriptor,
  type ProductProviderModelsResponse,
  type ProductReasoningPreference,
} from "../product/product-api-types";

type ModelInventoryState =
  | { profileId: string; status: "loading" }
  | { profileId: string; status: "ready"; response: ProductProviderModelsResponse }
  | { profileId: string; status: "error"; message: string };

export function quickModelOptions(
  currentModel: string,
  profileDefaults: Array<string | undefined>,
  inventory: ProductModelDescriptor[] | undefined,
): string[] {
  return Array.from(
    new Set(
      [
        currentModel,
        ...(inventory?.map(({ id }) => id) ?? profileDefaults),
      ]
        .filter((value): value is string => Boolean(value?.trim()))
        .map((value) => value.trim()),
    ),
  );
}

export function quickModelReasoning(
  model: string,
  inventory: ProductModelDescriptor[] | undefined,
): { available: boolean; reason: string } {
  if (!inventory) {
    return {
      available: false,
      reason: "Load the provider model catalog before selecting reasoning.",
    };
  }
  const descriptor = inventory.find(({ id }) => id === model.trim());
  if (!descriptor) {
    return {
      available: false,
      reason: "The selected model was not reported by the provider.",
    };
  }
  return descriptor.supports_reasoning
    ? { available: true, reason: "Reasoning controls are available for this model." }
    : {
        available: false,
        reason:
          descriptor.reasoning_unavailable_reason ??
          "The provider reports no reasoning controls for this model.",
      };
}

export function QuickModelControl({
  profiles,
  modelConfig,
  saving,
  loadProviderModels,
  onModelConfigChange,
}: {
  profiles: ProviderProfileRecord[];
  modelConfig: SessionModelConfig;
  saving: boolean;
  loadProviderModels: (profileId: string) => Promise<ProductProviderModelsResponse>;
  onModelConfigChange: (config: SessionModelConfigInput) => Promise<boolean>;
}) {
  const [open, setOpen] = useState(false);
  const [profileId, setProfileId] = useState(modelConfig.profileId ?? "");
  const [model, setModel] = useState(modelConfig.model);
  const [reasoning, setReasoning] = useState<ProductReasoningPreference>(
    modelConfig.reasoning,
  );
  const [maxSteps, setMaxSteps] = useState(String(modelConfig.maxSteps));
  const [result, setResult] = useState<"idle" | "saved" | "error">("idle");
  const [inventory, setInventory] = useState<ModelInventoryState | null>(null);
  const [inventoryReload, setInventoryReload] = useState(0);
  const modelListId = useId();
  const popoverId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const inventoryRequestRef = useRef(0);

  useEffect(() => {
    if (saving) {
      return;
    }
    setProfileId(modelConfig.profileId ?? "");
    setModel(modelConfig.model);
    setReasoning(modelConfig.reasoning);
    setMaxSteps(String(modelConfig.maxSteps));
  }, [modelConfig, saving]);

  useEffect(() => {
    if (!open) {
      return;
    }
    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
        window.requestAnimationFrame(() => triggerRef.current?.focus());
      }
    }
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  useEffect(() => {
    const request = ++inventoryRequestRef.current;
    if (!open || !profileId) {
      setInventory(null);
      return;
    }
    setInventory({ profileId, status: "loading" });
    void loadProviderModels(profileId).then(
      (response) => {
        if (inventoryRequestRef.current === request) {
          setInventory({ profileId, status: "ready", response });
        }
      },
      (error: unknown) => {
        if (inventoryRequestRef.current === request) {
          setInventory({
            profileId,
            status: "error",
            message: error instanceof Error ? error.message : "Provider model catalog failed.",
          });
        }
      },
    );
  }, [inventoryReload, loadProviderModels, open, profileId]);

  const activeProfile = profiles.find((profile) => profile.id === modelConfig.profileId);
  const selectedProfile = profiles.find((profile) => profile.id === profileId);
  const loadedInventory =
    inventory?.profileId === profileId && inventory.status === "ready"
      ? inventory.response.models
      : undefined;
  const reasoningCapability = profileId
    ? quickModelReasoning(model, loadedInventory)
    : {
        available: false,
        reason: "Select a provider profile to use explicit reasoning controls.",
      };
  const reasoningAvailable = reasoningCapability.available;
  const modelOptions = useMemo(
    () => quickModelOptions(
      modelConfig.model,
      profiles.map((profile) => profile.defaultModel),
      loadedInventory,
    ),
    [loadedInventory, modelConfig.model, profiles],
  );

  async function save() {
    const trimmedModel = model.trim();
    const parsedMaxSteps = Number(maxSteps);
    if (
      !trimmedModel ||
      !Number.isInteger(parsedMaxSteps) ||
      parsedMaxSteps < 1 ||
      parsedMaxSteps > 256 ||
      saving
    ) {
      setResult("error");
      return;
    }
    setResult("idle");
    const saved = await onModelConfigChange({
      profileId: profileId || undefined,
      model: trimmedModel,
      reasoning: reasoningAvailable ? reasoning : "default",
      maxSteps: parsedMaxSteps,
    });
    if (saved) {
      setResult("saved");
      setOpen(false);
      window.requestAnimationFrame(() => triggerRef.current?.focus());
    } else {
      setResult("error");
    }
  }

  return (
    <div ref={rootRef} className="quick-model" data-open={open}>
      <button
        ref={triggerRef}
        type="button"
        className="quick-model__trigger secondary"
        aria-expanded={open}
        aria-controls={popoverId}
        aria-haspopup="dialog"
        aria-label="Change session model settings"
        onClick={() => {
          setOpen((value) => !value);
          setResult("idle");
        }}
      >
        <MixerHorizontalIcon />
        <span>
          <strong>{modelConfig.model || "Runtime default"}</strong>
          <small>{activeProfile?.label ?? "Runtime provider"}</small>
        </span>
        <ChevronDownIcon />
      </button>
      {open ? (
        <div
          id={popoverId}
          className="quick-model__popover"
          role="dialog"
          aria-label="Session model settings"
        >
          <header>
            <strong>Session model</strong>
            <button
              type="button"
              className="ghost icon-button"
              aria-label="Close model control"
              onClick={() => {
                setOpen(false);
                window.requestAnimationFrame(() => triggerRef.current?.focus());
              }}
            >
              <Cross2Icon />
            </button>
          </header>
          <p>Changes apply from the next run in this session.</p>
          <label>
            <span>Provider profile</span>
            <select
              aria-label="Session provider profile"
              value={profileId}
              disabled={saving}
              onChange={(event) => {
                const nextProfileId = event.target.value;
                const nextProfile = profiles.find((profile) => profile.id === nextProfileId);
                setProfileId(nextProfileId);
                if (nextProfile?.defaultModel) {
                  setModel(nextProfile.defaultModel);
                }
                setReasoning(
                  nextProfile?.providerType === "openai-responses" ? reasoning : "default",
                );
                setResult("idle");
              }}
            >
              <option value="">Runtime default</option>
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>{profile.label}</option>
              ))}
            </select>
          </label>
          <label>
            <span>Model</span>
            <input
              aria-label="Session model"
              value={model}
              list={modelListId}
              disabled={saving}
              onChange={(event) => {
                setModel(event.target.value);
                setResult("idle");
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void save();
                }
              }}
            />
            <datalist id={modelListId}>
              {modelOptions.map((option) => <option value={option} key={option} />)}
            </datalist>
            {profileId ? (
              <span className="quick-model__inventory">
                <small
                  role={inventory?.status === "error" ? "alert" : "status"}
                  data-tone={inventory?.status === "error" ? "error" : undefined}
                >
                  {inventory?.status === "loading"
                    ? "Loading provider models..."
                    : inventory?.status === "error"
                      ? inventory.message
                      : inventory?.status === "ready" && inventory.response.models.length === 0
                        ? "Provider returned no models."
                        : inventory?.status === "ready"
                          ? `${inventory.response.models.length} provider models`
                          : "Provider models not loaded"}
                </small>
                <button
                  type="button"
                  className="ghost icon-button"
                  aria-label="Reload provider models"
                  title="Reload provider models"
                  disabled={inventory?.status === "loading"}
                  onClick={() => setInventoryReload((value) => value + 1)}
                >
                  <ReloadIcon />
                </button>
              </span>
            ) : null}
          </label>
          <label>
            <span>Inference level</span>
            <select
              aria-label="Session reasoning"
              value={reasoning}
              disabled={saving || !reasoningAvailable}
              onChange={(event) => {
                setReasoning(event.target.value as ProductReasoningPreference);
                setResult("idle");
              }}
            >
              {PRODUCT_REASONING_PREFERENCES.map((option) => (
                <option
                  key={option}
                  value={option}
                  disabled={option !== "default" && !reasoningAvailable}
                >
                  {option === "default" ? "Provider default" : option}
                </option>
              ))}
            </select>
            <small>
              {reasoningCapability.reason}
            </small>
          </label>
          <label>
            <span>Max steps</span>
            <input
              aria-label="Session max steps"
              type="number"
              min={1}
              max={256}
              step={1}
              value={maxSteps}
              disabled={saving}
              onChange={(event) => {
                setMaxSteps(event.target.value);
                setResult("idle");
              }}
            />
          </label>
          {result === "error" ? (
            <span className="quick-model__result" data-tone="error" role="alert">
              The session model settings were not changed. Review the product error and retry.
            </span>
          ) : null}
          <div className="field-actions">
            <button
              type="button"
              disabled={saving || !model.trim() || !maxSteps.trim()}
              onClick={() => void save()}
            >
              <CheckIcon /> {saving ? "Saving..." : "Save session model"}
            </button>
          </div>
        </div>
      ) : null}
      {result === "saved" ? (
        <span className="quick-model__result" role="status">Session model updated.</span>
      ) : null}
    </div>
  );
}
