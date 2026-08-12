"use client";

import { FormEvent, type Ref, useState } from "react";
import {
  PaperPlaneIcon,
  StopIcon,
} from "@radix-ui/react-icons";

import { QuickModelControl } from "../product-v2/QuickModelControl";
import type {
  ProviderProfileRecord,
  SessionModelConfig,
  SessionModelConfigInput,
} from "../state/product-types";
import type { ProductProviderModelsResponse } from "../product/product-api-types";

export function Composer({
  disabled,
  busy,
  resumeLabel,
  disabledReason,
  error,
  profiles,
  modelConfig,
  modelConfigSaving,
  textareaRef,
  onSend,
  onCancel,
  onLoadProviderModels,
  onModelConfigChange,
  controlError,
}: {
  disabled: boolean;
  busy: boolean;
  resumeLabel: string;
  disabledReason?: string;
  error: string | null;
  profiles: ProviderProfileRecord[];
  modelConfig: SessionModelConfig | null;
  modelConfigSaving: boolean;
  textareaRef?: Ref<HTMLTextAreaElement>;
  onSend: (message: string) => Promise<boolean> | boolean;
  onCancel: () => void;
  onLoadProviderModels: (profileId: string) => Promise<ProductProviderModelsResponse>;
  onModelConfigChange: (config: SessionModelConfigInput) => Promise<boolean>;
  controlError: string | null;
}) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const canSubmit =
    Boolean(message.trim()) &&
    !submitting &&
    !disabled;

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = message.trim();
    if (!trimmed || !canSubmit) {
      return;
    }
    setSubmitting(true);
    try {
      if (await onSend(trimmed)) {
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
      <div className="chat-composer__row">
        <textarea
          ref={textareaRef}
          aria-label="Message"
          aria-keyshortcuts="/"
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          placeholder="Message the agent..."
          disabled={disabled || submitting}
          aria-invalid={error ? "true" : undefined}
          aria-describedby={error ? "composer-error" : undefined}
        />
        <button type="submit" disabled={!canSubmit} aria-label="Send message">
          <PaperPlaneIcon />
          Send
        </button>
        {busy ? <StopRunButton onCancel={onCancel} /> : null}
      </div>
      {controlError ? <p className="control-queue__error" role="alert">{controlError}</p> : null}
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
