"use client";

import { CheckIcon, ChevronDownIcon, Cross2Icon, MixerHorizontalIcon } from "@radix-ui/react-icons";
import { useEffect, useId, useMemo, useRef, useState } from "react";

import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
} from "../state/product-types";

export function QuickModelControl({
  profiles,
  selection,
  saving,
  onSelectionChange,
}: {
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  saving: boolean;
  onSelectionChange: (selection: ActiveProviderSelection) => Promise<boolean>;
}) {
  const [open, setOpen] = useState(false);
  const [profileId, setProfileId] = useState(selection.profileId ?? "");
  const [model, setModel] = useState(selection.model);
  const [result, setResult] = useState<"idle" | "saved" | "error">("idle");
  const modelListId = useId();
  const popoverId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (saving) {
      return;
    }
    setProfileId(selection.mode === "profile" ? selection.profileId ?? "" : "");
    setModel(selection.model);
  }, [saving, selection.mode, selection.model, selection.profileId]);

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

  const activeProfile = profiles.find((profile) => profile.id === selection.profileId);
  const modelOptions = useMemo(
    () =>
      Array.from(
        new Set(
          [selection.model, ...profiles.map((profile) => profile.defaultModel)]
            .filter((value): value is string => Boolean(value?.trim()))
            .map((value) => value.trim()),
        ),
      ),
    [profiles, selection.model],
  );

  async function save() {
    const trimmedModel = model.trim();
    if (!trimmedModel || saving) {
      setResult("error");
      return;
    }
    setResult("idle");
    const saved = await onSelectionChange({
      ...selection,
      mode: profileId ? "profile" : "default",
      profileId: profileId || undefined,
      model: trimmedModel,
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
        aria-label="Change global next-run model default"
        onClick={() => {
          setOpen((value) => !value);
          setResult("idle");
        }}
      >
        <MixerHorizontalIcon />
        <span>
          <strong>{selection.model || "Runtime default"}</strong>
          <small>{activeProfile?.label ?? "Runtime provider"}</small>
        </span>
        <ChevronDownIcon />
      </button>
      {open ? (
        <div
          id={popoverId}
          className="quick-model__popover"
          role="dialog"
          aria-label="Global next-run model default"
        >
          <header>
            <strong>Next-run default</strong>
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
          <p>Global Preferences default for the next product run. This is not a session override.</p>
          <label>
            <span>Provider profile</span>
            <select
              aria-label="Next-run provider profile"
              value={profileId}
              disabled={saving}
              onChange={(event) => {
                const nextProfileId = event.target.value;
                const nextProfile = profiles.find((profile) => profile.id === nextProfileId);
                setProfileId(nextProfileId);
                if (nextProfile?.defaultModel) {
                  setModel(nextProfile.defaultModel);
                }
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
              aria-label="Next-run model"
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
          </label>
          {result === "error" ? (
            <span className="quick-model__result" data-tone="error" role="alert">
              The server preference was not changed. Review the product error and retry.
            </span>
          ) : null}
          <div className="field-actions">
            <button
              type="button"
              disabled={saving || !model.trim()}
              onClick={() => void save()}
            >
              <CheckIcon /> {saving ? "Saving…" : "Save global default"}
            </button>
          </div>
        </div>
      ) : null}
      {result === "saved" ? (
        <span className="quick-model__result" role="status">Server preference updated.</span>
      ) : null}
    </div>
  );
}
