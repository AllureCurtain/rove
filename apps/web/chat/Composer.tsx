"use client";

import { FormEvent, type Ref, useMemo, useState } from "react";
import {
  CheckIcon,
  Cross2Icon,
  PaperPlaneIcon,
  ReloadIcon,
  StopIcon,
} from "@radix-ui/react-icons";

import { QuickModelControl } from "../product-v2/QuickModelControl";
import type {
  ProviderProfileRecord,
  SessionModelConfig,
  SessionModelConfigInput,
} from "../state/product-types";
import type {
  ProductControl,
  ProductProviderModelsResponse,
} from "../product/product-api-types";

type ComposerMode = "message" | "steer" | "followup";

export function Composer({
  disabled,
  busy,
  controlAvailable,
  resumeLabel,
  disabledReason,
  error,
  profiles,
  modelConfig,
  modelConfigSaving,
  textareaRef,
  onSend,
  onSteer,
  onFollowup,
  onCancel,
  onLoadProviderModels,
  onModelConfigChange,
  controls,
  controlsLoading,
  controlBusy,
  controlError,
  onRefreshControls,
  onRevokeControl,
  onConfirmFollowup,
}: {
  disabled: boolean;
  busy: boolean;
  controlAvailable: boolean;
  resumeLabel: string;
  disabledReason?: string;
  error: string | null;
  profiles: ProviderProfileRecord[];
  modelConfig: SessionModelConfig | null;
  modelConfigSaving: boolean;
  textareaRef?: Ref<HTMLTextAreaElement>;
  onSend: (message: string) => Promise<void> | void;
  onSteer: (message: string) => Promise<boolean> | boolean;
  onFollowup: (message: string) => Promise<boolean> | boolean;
  onCancel: () => void;
  onLoadProviderModels: (profileId: string) => Promise<ProductProviderModelsResponse>;
  onModelConfigChange: (config: SessionModelConfigInput) => Promise<boolean>;
  controls: ProductControl[];
  controlsLoading: boolean;
  controlBusy: string | null;
  controlError: string | null;
  onRefreshControls: () => void;
  onRevokeControl: (controlId: string) => void;
  onConfirmFollowup: (controlId: string) => void;
}) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [mode, setMode] = useState<ComposerMode>("message");
  const activeMode: ComposerMode = controlAvailable
    ? mode === "message"
      ? "steer"
      : mode
    : "message";
  const attentionControls = useMemo(
    () =>
      controls
        .filter(
          (control) =>
            control.status === "pending" ||
            control.status === "accepted" ||
            control.status === "abandoned",
        )
        .sort((left, right) => left.seq - right.seq),
    [controls],
  );
  const pendingControls = attentionControls.slice(0, 8);
  const remainingControls = Math.max(0, attentionControls.length - pendingControls.length);
  const normalMessageAvailable = !disabled && !busy;
  const canSubmit =
    Boolean(message.trim()) &&
    !submitting &&
    (activeMode === "message" ? normalMessageAvailable : controlAvailable);

  const placeholder =
    activeMode === "steer"
      ? "Steer the active run..."
      : activeMode === "followup"
        ? "Queue the next instruction..."
        : "Message the agent...";

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = message.trim();
    if (!trimmed || !canSubmit) {
      return;
    }
    setSubmitting(true);
    try {
      if (activeMode === "message") {
        await onSend(trimmed);
        setMessage("");
        return;
      }
      const submitted = await (
        activeMode === "steer" ? onSteer(trimmed) : onFollowup(trimmed)
      );
      if (submitted) {
        setMessage("");
      }
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form className="chat-composer" onSubmit={handleSubmit} aria-label="Message composer">
      {error ? (
        <div className="chat-error" id="composer-error" role="alert">
          {error}
        </div>
      ) : null}
      <div className="chat-composer__meta">
        <span>{resumeLabel}</span>
        {busy ? <span>Streaming…</span> : null}
        {disabledReason ? <span>{disabledReason}</span> : null}
      </div>
      {controlAvailable ? (
        <div className="chat-composer__modes" role="group" aria-label="Message mode">
          <button
            type="button"
            data-active={activeMode === "steer"}
            onClick={() => setMode("steer")}
            disabled={submitting}
          >
            Steer
          </button>
          <button
            type="button"
            data-active={activeMode === "followup"}
            onClick={() => setMode("followup")}
            disabled={submitting}
          >
            Follow-up
          </button>
        </div>
      ) : null}
      <div className="chat-composer__row">
        <textarea
          ref={textareaRef}
          aria-label="Message"
          aria-keyshortcuts="/"
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          placeholder={placeholder}
          disabled={(activeMode === "message" ? disabled : !controlAvailable) || submitting}
          aria-invalid={error ? "true" : undefined}
          aria-describedby={error ? "composer-error" : undefined}
        />
        {activeMode === "message" && busy ? (
          <StopRunButton onCancel={onCancel} />
        ) : (
          <>
            <button type="submit" disabled={!canSubmit}>
              <PaperPlaneIcon />
              {activeMode === "steer"
                ? "Steer"
                : activeMode === "followup"
                  ? "Queue"
                  : "Send"}
            </button>
            {busy ? <StopRunButton onCancel={onCancel} /> : null}
          </>
        )}
      </div>
      {controlAvailable || pendingControls.length > 0 || controlError ? (
        <ControlQueue
          controls={pendingControls}
          hiddenCount={remainingControls}
          loading={controlsLoading}
          busy={controlBusy}
          error={controlError}
          onRefresh={onRefreshControls}
          onRevoke={onRevokeControl}
          onConfirm={onConfirmFollowup}
        />
      ) : null}
      <div className="chat-composer__controls">
        {modelConfig ? (
          <QuickModelControl
            profiles={profiles}
            modelConfig={modelConfig}
            saving={modelConfigSaving}
            loadProviderModels={onLoadProviderModels}
            onModelConfigChange={onModelConfigChange}
          />
        ) : (
          <span>Loading session model settings...</span>
        )}
      </div>
    </form>
  );
}

function StopRunButton({ onCancel }: { onCancel: () => void }) {
  return (
    <button type="button" className="danger" onClick={onCancel} aria-label="Stop run">
      <StopIcon />
      Stop
    </button>
  );
}

function ControlQueue({
  controls,
  hiddenCount,
  loading,
  busy,
  error,
  onRefresh,
  onRevoke,
  onConfirm,
}: {
  controls: ProductControl[];
  hiddenCount: number;
  loading: boolean;
  busy: string | null;
  error: string | null;
  onRefresh: () => void;
  onRevoke: (controlId: string) => void;
  onConfirm: (controlId: string) => void;
}) {
  return (
    <section className="control-queue" aria-label="Server-backed control queue">
      <div className="control-queue__header">
        <strong>Controls</strong>
        <button
          type="button"
          className="icon-button"
          onClick={onRefresh}
          disabled={loading || busy !== null}
          aria-label="Refresh controls"
          title="Refresh controls"
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="control-queue__error" role="alert">{error}</p> : null}
      {controls.length === 0 ? (
        <p className="control-queue__empty">
          {loading ? "Refreshing controls..." : "No queued controls"}
        </p>
      ) : (
        <ol className="control-queue__list">
          {controls.map((control) => {
            const canRevoke =
              control.status === "pending" ||
              (control.kind === "followup" && control.status === "abandoned");
            const canConfirm =
              control.kind === "followup" && control.status === "abandoned";
            return (
              <li key={control.id} data-status={control.status}>
                <div>
                  <span>{control.kind === "steer" ? "Steer" : "Follow-up"}</span>
                  <p>{control.content}</p>
                </div>
                <div className="control-queue__actions">
                  <small>{controlStatusLabel(control.status)}</small>
                  {canConfirm ? (
                    <button
                      type="button"
                      className="icon-button"
                      onClick={() => onConfirm(control.id)}
                      disabled={busy !== null}
                      aria-label="Confirm follow-up"
                      title="Confirm follow-up"
                    >
                      <CheckIcon />
                    </button>
                  ) : null}
                  {canRevoke ? (
                    <button
                      type="button"
                      className="icon-button"
                      onClick={() => onRevoke(control.id)}
                      disabled={busy !== null}
                      aria-label="Revoke control"
                      title="Revoke control"
                    >
                      <Cross2Icon />
                    </button>
                  ) : null}
                </div>
              </li>
            );
          })}
        </ol>
      )}
      {hiddenCount > 0 ? (
        <p className="control-queue__more">{hiddenCount} older controls retained by the server</p>
      ) : null}
    </section>
  );
}

function controlStatusLabel(status: ProductControl["status"]): string {
  switch (status) {
    case "pending":
      return "Queued";
    case "accepted":
      return "Starting";
    case "abandoned":
      return "Needs confirmation";
    default:
      return status;
  }
}
