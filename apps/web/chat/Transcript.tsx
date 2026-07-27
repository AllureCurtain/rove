"use client";

import { CheckIcon, Cross2Icon } from "@radix-ui/react-icons";
import { FormEvent, useEffect, useRef, useState } from "react";

import type {
  ToolCallView,
  TranscriptRunGroup,
  TranscriptTimelineItem,
} from "../lib/rove-state";
import {
  describeTranscriptPartialReason,
  type TranscriptRestoreState,
} from "../state/transcript-projection";

export function Transcript({
  timeline,
  approvalBusy,
  inputBusy,
  restoreState,
  onRetryRestore,
  onStartNewSession,
  onApproval,
  onInputSubmit,
}: {
  timeline: TranscriptRunGroup[];
  approvalBusy: string | null;
  inputBusy: string | null;
  restoreState: TranscriptRestoreState;
  onRetryRestore: () => void;
  onStartNewSession: () => void;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onInputSubmit: (inputId: string, answer: string) => void;
}) {
  const itemCount = timeline.reduce((total, group) => total + group.items.length, 0);

  return (
    <div
      className="chat-transcript"
      aria-label="Conversation"
      role="log"
      aria-live="polite"
      aria-relevant="additions text"
    >
      <RestoreNotice
        state={restoreState}
        onRetry={onRetryRestore}
        onStartNewSession={onStartNewSession}
      />
      {itemCount === 0 &&
      (restoreState.status === "complete" || restoreState.status === "idle") ? (
        <p style={{ color: "var(--muted)", margin: 0 }}>
          Send a message to start a run in this session.
        </p>
      ) : null}
      {timeline.map((group) => (
        <section
          key={group.id}
          className="transcript-run"
          data-run-id={group.runId ?? undefined}
          data-run-ordinal={group.runOrdinal ?? undefined}
        >
          {group.items.map((item) => (
            <TranscriptItem
              key={item.entry.id}
              item={item}
              approvalBusy={approvalBusy}
              inputBusy={inputBusy}
              onApproval={onApproval}
              onInputSubmit={onInputSubmit}
            />
          ))}
        </section>
      ))}
    </div>
  );
}

function TranscriptItem({
  item,
  approvalBusy,
  inputBusy,
  onApproval,
  onInputSubmit,
}: {
  item: TranscriptTimelineItem;
  approvalBusy: string | null;
  inputBusy: string | null;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onInputSubmit: (inputId: string, answer: string) => void;
}) {
  switch (item.kind) {
    case "message":
      return (
        <article
          className="chat-bubble"
          data-role={item.message.role}
          data-status={item.message.status}
        >
          {item.message.content}
        </article>
      );
    case "tool":
      return item.tool.status === "waiting" || item.tool.pendingApproval ? (
        <ApprovalCard
          tool={item.tool}
          busy={approvalBusy === item.tool.id}
          onApproval={onApproval}
        />
      ) : (
        <ToolCard tool={item.tool} />
      );
    case "input":
      return item.input.status === "waiting" ? (
        <InputCard
          inputId={item.input.id}
          prompt={item.input.prompt}
          busy={inputBusy === item.input.id}
          onSubmit={onInputSubmit}
        />
      ) : (
        <article className="input-card" data-status={item.input.status} role="status">
          <div>
            <strong>
              {item.input.status === "submitted" ? "Input submitted" : "Input closed"}
            </strong>
            <p style={{ margin: "6px 0 0" }}>{item.input.prompt}</p>
          </div>
        </article>
      );
  }
}

function RestoreNotice({
  state,
  onRetry,
  onStartNewSession,
}: {
  state: TranscriptRestoreState;
  onRetry: () => void;
  onStartNewSession: () => void;
}) {
  if (state.status === "idle" || state.status === "complete") {
    return null;
  }
  if (state.status === "loading") {
    return (
      <section className="restore-notice" data-tone="loading" role="status">
        <strong>Restoring conversation</strong>
        <span>Reading canonical run events for this session.</span>
      </section>
    );
  }
  if (state.status === "partial") {
    return (
      <section className="restore-notice" data-tone="partial" role="status">
        <strong>Partial conversation history</strong>
        <span>Available canonical events are shown. Some durable history could not be rebuilt.</span>
        <ul>
          {state.reasons.map((reason, index) => (
            <li key={`${reason.code}-${reason.run_ordinal ?? "session"}-${index}`}>
              {describeTranscriptPartialReason(reason)}
            </li>
          ))}
        </ul>
        <div className="field-actions">
          <button type="button" className="secondary" onClick={onRetry}>
            Retry restore
          </button>
          <button type="button" className="secondary" onClick={onStartNewSession}>
            New session
          </button>
        </div>
      </section>
    );
  }
  return (
    <section className="restore-notice" data-tone="error" role="alert">
      <strong>Conversation restore failed</strong>
      <span>{state.error}</span>
      <span>No empty history has been substituted for the failed read.</span>
      <div className="field-actions">
        <button type="button" onClick={onRetry}>
          Retry restore
        </button>
        <button type="button" className="secondary" onClick={onStartNewSession}>
          New session
        </button>
      </div>
    </section>
  );
}

function ToolCard({ tool }: { tool: ToolCallView }) {
  const [open, setOpen] = useState(tool.status === "running" || tool.status === "error");
  return (
    <article className="tool-card" data-status={tool.status}>
      <div className="tool-card__head">
        <span>
          {tool.name} · {tool.status}
        </span>
        <button type="button" className="ghost" onClick={() => setOpen((value) => !value)}>
          {open ? "Hide" : "Show"}
        </button>
      </div>
      {open ? <pre>{tool.details}</pre> : null}
    </article>
  );
}

function ApprovalCard({
  tool,
  busy,
  onApproval,
}: {
  tool: ToolCallView;
  busy: boolean;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
}) {
  const approvalCardRef = useRef<HTMLElement>(null);

  useEffect(() => {
    approvalCardRef.current?.focus();
  }, []);

  return (
    <article
      ref={approvalCardRef}
      className="approval-card"
      aria-label="Pending approval"
      role="alert"
      tabIndex={-1}
    >
      <div className="approval-card__head">
        <span>Approval needed · {tool.name}</span>
      </div>
      <p style={{ margin: 0 }}>{tool.reason ?? tool.details}</p>
      {tool.pendingApproval ? <pre>{formatValue(tool.pendingApproval.args)}</pre> : null}
      <div className="field-actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => onApproval(tool, "approve")}
        >
          <CheckIcon />
          Approve
        </button>
        <button
          type="button"
          className="danger"
          disabled={busy}
          onClick={() => onApproval(tool, "reject")}
        >
          <Cross2Icon />
          Reject
        </button>
      </div>
    </article>
  );
}

function InputCard({
  inputId,
  prompt,
  busy,
  onSubmit,
}: {
  inputId: string;
  prompt: string;
  busy: boolean;
  onSubmit: (inputId: string, answer: string) => void;
}) {
  const [answer, setAnswer] = useState("");
  const answerRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    answerRef.current?.focus();
  }, []);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = answer.trim();
    if (!trimmed || busy) {
      return;
    }
    onSubmit(inputId, trimmed);
  }

  return (
    <article className="input-card" role="status" aria-live="polite">
      <div>
        <strong>Input requested</strong>
        <p style={{ margin: "6px 0 0" }}>{prompt}</p>
      </div>
      <form className="chat-composer__row" onSubmit={handleSubmit}>
        <input
          ref={answerRef}
          type="text"
          value={answer}
          onChange={(event) => setAnswer(event.target.value)}
          placeholder="Type your answer"
          disabled={busy}
          aria-label={prompt}
        />
        <button type="submit" disabled={busy || !answer.trim()}>
          Send
        </button>
      </form>
    </article>
  );
}

function formatValue(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
