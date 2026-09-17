"use client";

import {
  CheckIcon,
  ArrowUpIcon,
  ChevronDownIcon,
  CodeIcon,
  Cross2Icon,
  FileIcon,
  LockClosedIcon,
} from "@radix-ui/react-icons";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";

import type {
  ToolCallView,
  TranscriptRunGroup,
  TranscriptTimelineItem,
} from "../lib/rove-state";
import { useCopy } from "../copy/CopyProvider";
import { DiffView } from "../product-v2/DiffView";
import { RichText } from "../product-v2/RichText";
import type { ProductMessage } from "../product/product-api-types";
import {
  describeTranscriptPartialReason,
  type TranscriptRestoreState,
} from "../state/transcript-projection";

const INITIAL_VISIBLE_RUNS = 24;
const RUN_PAGE_SIZE = 16;

export function Transcript({
  timeline,
  messages = [],
  messageBusy = null,
  canPromote = false,
  approvalBusy,
  inputBusy,
  restoreState,
  onRetryRestore,
  onStartNewSession,
  onApproval,
  onInputSubmit,
  onPromoteMessage = () => {},
  onRevokeMessage = () => {},
}: {
  timeline: TranscriptRunGroup[];
  messages?: ProductMessage[];
  messageBusy?: string | null;
  canPromote?: boolean;
  approvalBusy: string | null;
  inputBusy: string | null;
  restoreState: TranscriptRestoreState;
  onRetryRestore: () => void;
  onStartNewSession: () => void;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onInputSubmit: (inputId: string, answer: string) => void;
  onPromoteMessage?: (messageId: string) => void;
  onRevokeMessage?: (messageId: string) => void;
}) {
  const { t } = useCopy();
  const [visibleRunCount, setVisibleRunCount] = useState(INITIAL_VISIBLE_RUNS);
  const visibleTimeline = useMemo(
    () => timeline.slice(Math.max(0, timeline.length - visibleRunCount)),
    [timeline, visibleRunCount],
  );
  const actionableMessages = useMemo(
    () => messages.filter((message) =>
      message.status === "queued" ||
      message.status === "intervention_requested" ||
      message.status === "needs_attention"
    ),
    [messages],
  );
  const hiddenRunCount = Math.max(0, timeline.length - visibleTimeline.length);
  const itemCount = visibleTimeline.reduce((total, group) => total + group.items.length, 0)
    + actionableMessages.length;
  const transcriptRef = useRef<HTMLDivElement>(null);
  const prependHeightRef = useRef<number | null>(null);
  const [atLatest, setAtLatest] = useState(true);

  useEffect(() => {
    const transcript = transcriptRef.current;
    if (!transcript) {
      return;
    }
    const observer = new ResizeObserver(() => {
      if (prependHeightRef.current !== null) {
        transcript.scrollTop += transcript.scrollHeight - prependHeightRef.current;
        prependHeightRef.current = null;
      } else if (atLatest) {
        transcript.scrollTop = transcript.scrollHeight;
      }
    });
    const content = transcript.firstElementChild ?? transcript;
    observer.observe(content);
    if (atLatest) {
      transcript.scrollTop = transcript.scrollHeight;
    }
    return () => observer.disconnect();
  }, [atLatest]);

  useEffect(() => {
    setVisibleRunCount(INITIAL_VISIBLE_RUNS);
  }, [restoreState.status === "idle" ? "idle" : restoreState.sessionId]);

  function loadOlderRuns() {
    const transcript = transcriptRef.current;
    if (transcript) {
      prependHeightRef.current = transcript.scrollHeight;
    }
    setVisibleRunCount((count) => Math.min(timeline.length, count + RUN_PAGE_SIZE));
  }

  function syncScrollPosition() {
    const transcript = transcriptRef.current;
    if (!transcript) {
      return;
    }
    setAtLatest(
      transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight < 48,
    );
  }

  function returnToLatest() {
    const transcript = transcriptRef.current;
    if (!transcript) {
      return;
    }
    transcript.scrollTo({ top: transcript.scrollHeight, behavior: "smooth" });
    setAtLatest(true);
  }

  return (
    <div className="chat-transcript-frame">
      <div
        ref={transcriptRef}
        className="chat-transcript"
        aria-label="Conversation"
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
        onScroll={syncScrollPosition}
      >
        <div className="chat-transcript__content">
          <RestoreNotice
            state={restoreState}
            onRetry={onRetryRestore}
            onStartNewSession={onStartNewSession}
          />
          {hiddenRunCount > 0 ? (
            <button type="button" className="load-older-turns" onClick={loadOlderRuns}>
              Load {Math.min(RUN_PAGE_SIZE, hiddenRunCount)} older turns
            </button>
          ) : null}
          {itemCount === 0 &&
          (restoreState.status === "complete" || restoreState.status === "idle") ? (
            <p className="transcript-empty">{t("chat.emptyTranscript")}</p>
          ) : null}
          {visibleTimeline.map((group) => (
          <section
            key={group.id}
            className="transcript-run"
            data-run-id={group.runId ?? undefined}
            data-run-ordinal={group.runOrdinal ?? undefined}
            data-inherited={group.inherited ? "true" : undefined}
          >
            <header className="transcript-run__header">
              <span className="transcript-run__label">
                <span>
                  {group.runOrdinal
                    ? t("chat.turn", { n: group.runOrdinal })
                    : t("chat.currentTurn")}
                </span>
                {group.inherited ? (
                  <small>{t("workspace.forked")}</small>
                ) : null}
              </span>
            </header>
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
          {actionableMessages.map((message) => (
            <QueuedMessage
              key={message.id}
              message={message}
              busy={messageBusy !== null}
              canPromote={canPromote}
              onPromote={onPromoteMessage}
              onRevoke={onRevokeMessage}
            />
          ))}
        </div>
      </div>
      {!atLatest && itemCount > 0 ? (
        <button type="button" className="return-to-latest" onClick={returnToLatest}>
          Return to latest
        </button>
      ) : null}
    </div>
  );
}

function QueuedMessage({
  message,
  busy,
  canPromote,
  onPromote,
  onRevoke,
}: {
  message: ProductMessage;
  busy: boolean;
  canPromote: boolean;
  onPromote: (messageId: string) => void;
  onRevoke: (messageId: string) => void;
}) {
  const { t } = useCopy();
  const promotable = message.status === "queued" && canPromote;
  const revocable = message.status === "queued" || message.status === "needs_attention";
  return (
    <article className="chat-bubble queued-message" data-role="user" data-status={message.status}>
      <div className="message-byline">
        <strong>{t("chat.you")}</strong>
        <span>{messageStatusLabel(message.status, t)}</span>
      </div>
      <RichText content={message.content} />
      {message.reason ? <p className="queued-message__reason">{message.reason}</p> : null}
      {promotable || revocable ? (
        <div className="queued-message__actions">
          {promotable ? (
            <button type="button" className="secondary" disabled={busy} onClick={() => onPromote(message.id)}>
              <ArrowUpIcon />
              {t("chat.promote")}
            </button>
          ) : null}
          {revocable ? (
            <button type="button" className="icon-button" disabled={busy} onClick={() => onRevoke(message.id)} aria-label={t("chat.revokeTitle")} title={t("chat.revokeTitle")}>
              <Cross2Icon />
            </button>
          ) : null}
        </div>
      ) : null}
    </article>
  );
}

function messageStatusLabel(
  status: ProductMessage["status"],
  t: (path: string) => string,
): string {
  switch (status) {
    case "queued":
      return t("inspector.statusQueued");
    case "intervention_requested":
      return t("chat.promote");
    case "applied_current_run":
      return t("chat.promote");
    case "claimed_successor":
      return t("chat.queue");
    case "needs_attention":
      return t("workspace.needsAttention");
    case "revoked":
      return t("chat.revoke");
  }
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
  const { t } = useCopy();
  const content = (() => {
    switch (item.kind) {
      case "message":
        return (
          <article
            className="chat-bubble"
            data-role={item.message.role}
            data-status={item.message.status}
          >
            <div className="message-byline">
              <strong>
                {item.message.role === "user" ? t("chat.you") : t("chat.assistant")}
              </strong>
              <span>
                {item.message.status === "streaming"
                  ? t("chat.responding")
                  : ""}
              </span>
            </div>
            <RichText content={item.message.content} />
            {item.message.role === "assistant" ? (
              <MessageEvidence message={item.message} />
            ) : null}
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
                {item.input.status === "submitted"
                  ? t("chat.inputSubmitted")
                  : t("chat.inputClosed")}
              </strong>
              <p>{item.input.prompt}</p>
            </div>
          </article>
        );
    }
  })();

  return (
    <div className="transcript-item" data-kind={item.kind}>
      <div className="transcript-item__meta" aria-hidden="true">
        <span data-state={timelineItemState(item)} />
      </div>
      <div className="transcript-item__content">{content}</div>
    </div>
  );
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
  const { t } = useCopy();
  if (state.status === "idle" || state.status === "complete") {
    return null;
  }
  if (state.status === "loading") {
    return (
      <section className="restore-notice" data-tone="loading" role="status">
        <strong>{t("chat.restoring")}</strong>
      </section>
    );
  }
  if (state.status === "partial") {
    return (
      <section className="restore-notice" data-tone="partial" role="status">
        <strong>{t("chat.restorePartialTitle")}</strong>
        <span>{t("chat.restorePartial")}</span>
        <ul>
          {state.reasons.map((reason, index) => (
            <li key={`${reason.code}-${reason.run_ordinal ?? "session"}-${index}`}>
              {describeTranscriptPartialReason(reason)}
            </li>
          ))}
        </ul>
        <div className="field-actions">
          <button type="button" className="secondary" onClick={onRetry}>
            {t("chat.restoreRetry")}
          </button>
          <button type="button" className="secondary" onClick={onStartNewSession}>
            {t("nav.newSession")}
          </button>
        </div>
      </section>
    );
  }
  return (
    <section className="restore-notice" data-tone="error" role="alert">
      <strong>{t("chat.restoreFailed")}</strong>
      <span>{state.error}</span>
      <span>{t("chat.restoreError")}</span>
      <div className="field-actions">
        <button type="button" onClick={onRetry}>
          {t("chat.restoreRetry")}
        </button>
        <button type="button" className="secondary" onClick={onStartNewSession}>
          {t("nav.newSession")}
        </button>
      </div>
    </section>
  );
}

function ToolCard({ tool }: { tool: ToolCallView }) {
  const [open, setOpen] = useState(
    tool.status === "running" || tool.status === "error" || Boolean(tool.mutations?.length),
  );
  const detailId = `tool-detail-${tool.timelineId ?? tool.id}`.replace(/[^A-Za-z0-9_-]/gu, "-");
  const structured =
    tool.args !== undefined ||
    tool.output !== undefined ||
    tool.error !== undefined ||
    tool.metadata !== undefined ||
    Boolean(tool.mutations?.length);
  return (
    <article className="tool-card" data-status={tool.status}>
      <button
        type="button"
        className="tool-card__head"
        aria-controls={detailId}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <CodeIcon />
        <span>
          <strong>{tool.name}</strong>
          <small>{tool.metadata?.read_only ? "read only" : tool.metadata?.workspace_changed ? "workspace changed" : "tool call"}</small>
        </span>
        <span className="tool-card__outcome" data-status={tool.status}>
          {tool.status}
        </span>
        <ChevronDownIcon data-open={open} />
      </button>
      {open ? (
        <div className="tool-card__details" id={detailId}>
          {tool.args !== undefined ? (
            <section>
              <h4>Invocation</h4>
              <pre tabIndex={0}>{formatValue(tool.args)}</pre>
            </section>
          ) : null}
          {tool.output !== undefined ? (
            <section>
              <h4>Result</h4>
              <RichText content={tool.output} />
            </section>
          ) : null}
          {tool.error !== undefined ? (
            <section>
              <h4>Failure</h4>
              <pre tabIndex={0}>{formatValue(tool.error)}</pre>
            </section>
          ) : null}
          {tool.metadata ? <ToolFacts tool={tool} /> : null}
          {tool.mutations?.length ? (
            <section className="tool-mutations">
              <h4>Workspace mutations</h4>
              {tool.mutations.map((mutation, index) => (
                <article className="tool-mutation" key={`${mutation.path}-${mutation.operation}-${index}`}>
                  <header>
                    <FileIcon />
                    <strong>{mutation.path}</strong>
                    <span>{mutation.operation}</span>
                  </header>
                  {mutation.diff ? (
                    <DiffView
                      diff={mutation.diff}
                      label={`${mutation.path} diff`}
                      sourcePath={mutation.path}
                    />
                  ) : null}
                </article>
              ))}
            </section>
          ) : null}
          {!structured ? <pre tabIndex={0}>{tool.details}</pre> : null}
        </div>
      ) : null}
    </article>
  );
}

function ToolFacts({ tool }: { tool: ToolCallView }) {
  const metadata = tool.metadata!;
  const affectedPaths = metadata.affected_paths ?? [];
  const diffSummary = metadata.diff_summary ?? [];
  return (
    <section>
      <h4>Execution facts</h4>
      <dl className="tool-facts">
        <div><dt>Call</dt><dd><code>{tool.id}</code></dd></div>
        <div><dt>Status</dt><dd>{metadata.status}</dd></div>
        <div><dt>Risk</dt><dd>{metadata.risk_level}</dd></div>
        <div><dt>Access</dt><dd>{metadata.read_only ? "Read only" : "Mutation capable"}</dd></div>
        <div><dt>Workspace</dt><dd>{metadata.workspace_changed ? "Changed" : "Unchanged"}</dd></div>
        {metadata.error_code ? <div><dt>Error code</dt><dd>{metadata.error_code}</dd></div> : null}
        {metadata.security_event_type ? <div><dt>Security event</dt><dd>{metadata.security_event_type}</dd></div> : null}
        {affectedPaths.length ? (
          <div><dt>Affected paths</dt><dd>{affectedPaths.join(", ")}</dd></div>
        ) : null}
      </dl>
      {diffSummary.length ? (
        <ul className="tool-diff-summary">
          {diffSummary.map((summary, index) => <li key={`${summary}-${index}`}>{summary}</li>)}
        </ul>
      ) : null}
    </section>
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
  const { t } = useCopy();
  const approvalCardRef = useRef<HTMLElement>(null);

  useEffect(() => {
    approvalCardRef.current?.focus();
  }, []);

  return (
    <article
      ref={approvalCardRef}
      className="approval-card"
      aria-label={t("inspector.approvalsTitle")}
      role="alert"
      tabIndex={-1}
    >
      <div className="approval-card__head">
        <LockClosedIcon />
        <span><strong>{t("inspector.statusWaitingApproval")}</strong><small>{tool.name}</small></span>
      </div>
      <p>{tool.reason ?? tool.details}</p>
      {tool.args !== undefined || tool.pendingApproval ? (
        <pre tabIndex={0}>{formatValue(tool.args ?? tool.pendingApproval?.args)}</pre>
      ) : null}
      <div className="field-actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => onApproval(tool, "approve")}
        >
          <CheckIcon />
          {t("chat.approve")}
        </button>
        <button
          type="button"
          className="danger"
          disabled={busy}
          onClick={() => onApproval(tool, "reject")}
        >
          <Cross2Icon />
          {t("chat.reject")}
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
  const { t } = useCopy();
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
        <strong>{t("chat.responding")}</strong>
        <p>{prompt}</p>
      </div>
      <form className="chat-composer__row" onSubmit={handleSubmit}>
        <input
          ref={answerRef}
          type="text"
          value={answer}
          onChange={(event) => setAnswer(event.target.value)}
          placeholder={t("chat.placeholder")}
          disabled={busy}
          aria-label={prompt}
        />
        <button type="submit" disabled={busy || !answer.trim()}>
          {t("chat.send")}
        </button>
      </form>
    </article>
  );
}

function MessageEvidence({
  message,
}: {
  message: Extract<TranscriptTimelineItem, { kind: "message" }>["message"];
}) {
  const { t } = useCopy();
  if (!message.usage && !message.promptBuild && !message.promptCompaction) {
    return null;
  }
  return (
    <dl className="message-evidence" aria-label={t("inspector.usageEvidence")}>
      {message.usage ? (
        <>
          <div><dt>{t("inspector.total")}</dt><dd>{formatNumber(message.usage.total_tokens)}</dd></div>
          <div><dt>{t("inspector.prompt")}</dt><dd>{formatNumber(message.usage.prompt_tokens)}</dd></div>
          <div><dt>{t("inspector.completion")}</dt><dd>{formatNumber(message.usage.completion_tokens)}</dd></div>
          {message.usage.cached_tokens !== undefined ? (
            <div><dt>{t("inspector.cached")}</dt><dd>{formatNumber(message.usage.cached_tokens)}</dd></div>
          ) : null}
        </>
      ) : null}
      {message.promptBuild ? (
        <div>
          <dt>{t("inspector.contextEstimate")}</dt>
          <dd>{formatNumber(message.promptBuild.token_estimate)}</dd>
        </div>
      ) : null}
      {message.promptCompaction ? (
        <div><dt>{t("inspector.compacted")}</dt><dd>{message.promptCompaction.mode.replaceAll("_", " ")}</dd></div>
      ) : null}
    </dl>
  );
}

function timelineItemState(item: TranscriptTimelineItem): string {
  if (item.kind === "message") {
    return item.message.status;
  }
  if (item.kind === "tool") {
    return item.tool.status;
  }
  return item.input.status;
}

function shortId(value: string): string {
  return value.length <= 12 ? value : value.slice(0, 12);
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
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
