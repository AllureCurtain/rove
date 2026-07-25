"use client";

import { FormEvent, useState } from "react";
import { PaperPlaneIcon, StopIcon } from "@radix-ui/react-icons";

export function Composer({
  disabled,
  busy,
  modelLabel,
  resumeLabel,
  error,
  onSend,
  onCancel,
}: {
  disabled: boolean;
  busy: boolean;
  modelLabel: string;
  resumeLabel: string;
  error: string | null;
  onSend: (message: string) => Promise<void> | void;
  onCancel: () => void;
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
      {error ? <div className="chat-error">{error}</div> : null}
      <div className="chat-composer__meta">
        <span>{modelLabel}</span>
        <span>{resumeLabel}</span>
        {busy ? <span>Streaming…</span> : null}
      </div>
      <div className="chat-composer__row">
        <textarea
          aria-label="Message"
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          placeholder="Message the agent…"
          disabled={disabled || submitting}
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
    </form>
  );
}
