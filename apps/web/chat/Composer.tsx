"use client";

import { FormEvent, type Ref, useState } from "react";
import { PaperPlaneIcon, StopIcon } from "@radix-ui/react-icons";

import { QuickModelControl } from "../product-v2/QuickModelControl";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
} from "../state/product-types";

export function Composer({
  disabled,
  busy,
  resumeLabel,
  disabledReason,
  error,
  profiles,
  selection,
  selectionSaving,
  textareaRef,
  onSend,
  onCancel,
  onSelectionChange,
}: {
  disabled: boolean;
  busy: boolean;
  resumeLabel: string;
  disabledReason?: string;
  error: string | null;
  profiles: ProviderProfileRecord[];
  selection: ActiveProviderSelection;
  selectionSaving: boolean;
  textareaRef?: Ref<HTMLTextAreaElement>;
  onSend: (message: string) => Promise<void> | void;
  onCancel: () => void;
  onSelectionChange: (selection: ActiveProviderSelection) => Promise<boolean>;
}) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = message.trim();
    if (!trimmed || disabled || busy || submitting) {
      return;
    }
    setSubmitting(true);
    try {
      await onSend(trimmed);
      setMessage("");
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
          placeholder="Message the agent…"
          disabled={disabled || submitting}
          aria-invalid={error ? "true" : undefined}
          aria-describedby={error ? "composer-error" : undefined}
        />
        {busy ? (
          <button type="button" className="danger" onClick={onCancel} aria-label="Stop run">
            <StopIcon />
            Stop
          </button>
        ) : (
          <button type="submit" disabled={disabled || submitting || !message.trim()}>
            <PaperPlaneIcon />
            Send
          </button>
        )}
      </div>
      <div className="chat-composer__controls">
        <QuickModelControl
          profiles={profiles}
          selection={selection}
          saving={selectionSaving}
          onSelectionChange={onSelectionChange}
        />
        <span>Applies globally to the next product run.</span>
      </div>
    </form>
  );
}
