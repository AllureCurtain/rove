"use client";

import { CheckIcon, Cross2Icon } from "@radix-ui/react-icons";
import { FormEvent, useState } from "react";

import type { ChatMessage, ToolCallView } from "../lib/rove-state";
import type { PendingInput } from "../lib/rove-types";

export function Transcript({
  messages,
  tools,
  pendingInputs,
  approvalBusy,
  inputBusy,
  onApproval,
  onInputSubmit,
}: {
  messages: ChatMessage[];
  tools: ToolCallView[];
  pendingInputs: PendingInput[];
  approvalBusy: string | null;
  inputBusy: string | null;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onInputSubmit: (inputId: string, answer: string) => void;
}) {
  const waitingTools = tools.filter((tool) => tool.status === "waiting" || tool.pendingApproval);
  const recentTools = tools.filter((tool) => tool.status !== "waiting" && !tool.pendingApproval).slice(0, 8);

  return (
    <div className="chat-transcript" aria-label="Conversation">
      {messages.length === 0 ? (
        <p style={{ color: "var(--muted)", margin: 0 }}>
          Send a message to start a run in this session.
        </p>
      ) : null}
      {messages.map((message) => (
        <article
          key={message.id}
          className="chat-bubble"
          data-role={message.role}
          data-status={message.status}
        >
          {message.content}
        </article>
      ))}
      {waitingTools.map((tool) => (
        <ApprovalCard
          key={`approval-${tool.id}`}
          tool={tool}
          busy={approvalBusy === tool.id}
          onApproval={onApproval}
        />
      ))}
      {pendingInputs.map((input) => (
        <InputCard
          key={input.input_id}
          inputId={input.input_id}
          prompt={input.prompt}
          busy={inputBusy === input.input_id}
          onSubmit={onInputSubmit}
        />
      ))}
      {recentTools.map((tool) => (
        <ToolCard key={`tool-${tool.id}`} tool={tool} />
      ))}
    </div>
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
  return (
    <article className="approval-card" aria-label="Pending approval">
      <div className="approval-card__head">
        <span>Approval needed · {tool.name}</span>
      </div>
      <p style={{ margin: 0 }}>{tool.reason ?? tool.details}</p>
      {tool.pendingApproval ? <pre>{formatValue(tool.pendingApproval.args)}</pre> : null}
      <div className="field-actions">
        <button type="button" disabled={busy} onClick={() => onApproval(tool, "approve")}>
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

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = answer.trim();
    if (!trimmed || busy) {
      return;
    }
    onSubmit(inputId, trimmed);
  }

  return (
    <article className="input-card">
      <div>
        <strong>Input requested</strong>
        <p style={{ margin: "6px 0 0" }}>{prompt}</p>
      </div>
      <form className="chat-composer__row" onSubmit={handleSubmit}>
        <input
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
