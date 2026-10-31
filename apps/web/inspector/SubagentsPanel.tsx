"use client";

import { Component2Icon } from "@radix-ui/react-icons";
import { useEffect, useMemo, useState } from "react";

import type {
  SubagentView,
  ToolCallView,
  WorkbenchState,
} from "../lib/rove-state";
import { useCopy } from "../copy/CopyProvider";
import { RichText } from "../product-v2/RichText";
import { SubagentStopButton, subagentStatusLabel } from "../chat/subagent-topology";
import { ThinkingRegion } from "../chat/thinking-region";
import { ToolCard } from "../chat/tool-card";
import type { FileReference } from "../chat/file-ref";
import { InspectorEmpty } from "./InspectorEmpty";
import { buildSubagentTranscript } from "./subagent-transcript";

/**
 * The work panel's delegation surface (issue #31): the run's `task` calls as a
 * list, and the selected child's own transcript — its prompt as the user row,
 * its tools and answers in the transcript's own language. The foot is a
 * disabled composer: a delegate is driven by the main agent, never by typing.
 */
export function SubagentsPanel({
  runState,
  onStopSubagent,
  onOpenFile,
}: {
  runState: WorkbenchState;
  onStopSubagent?: (parentCallId: string) => void | Promise<unknown>;
  onOpenFile?: (reference: FileReference) => void;
}) {
  const { t } = useCopy();
  const subagents = runState.subagents;
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Keep the selection on a delegation that still exists; a settled child stays
  // listed, but a reset run drops them all.
  const selected =
    subagents.find((subagent) => subagent.parentCallId === selectedId) ??
    subagents[0];
  useEffect(() => {
    if (selected && selected.parentCallId !== selectedId) {
      setSelectedId(selected.parentCallId);
    }
  }, [selected, selectedId]);

  if (subagents.length === 0) {
    return (
      <InspectorEmpty
        icon={<Component2Icon />}
        title={t("inspector.subagentsNone")}
        body={t("inspector.subagentsNoneBody")}
      />
    );
  }

  const parentTool = selected
    ? runState.tools.find((tool) => tool.id === selected.parentCallId)
    : undefined;

  return (
    <div className="subagent-panel">
      <ul
        className="subagent-panel__list"
        role="list"
        aria-label={t("inspector.subagentsListLabel")}
      >
        {subagents.map((subagent) => {
          const active = selected?.parentCallId === subagent.parentCallId;
          return (
            <li key={subagent.parentCallId}>
              <button
                type="button"
                className="subagent-panel__item"
                data-active={active ? "true" : undefined}
                data-status={subagent.status}
                onClick={() => setSelectedId(subagent.parentCallId)}
                aria-pressed={active}
              >
                <span
                  className="subagent-panel__dot"
                  data-status={subagent.status}
                  aria-hidden="true"
                />
                <span className="subagent-panel__name">
                  {subagentRowTitle(subagent, runState.tools)}
                </span>
                <span
                  className="subagent-panel__state"
                  data-status={subagent.status}
                >
                  {subagentStatusLabel(subagent.status, t)}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      {selected ? (
        <SubagentTranscriptView
          key={selected.parentCallId}
          subagent={selected}
          parentTool={parentTool}
          onStopSubagent={onStopSubagent}
          onOpenFile={onOpenFile}
        />
      ) : null}
    </div>
  );
}

function SubagentTranscriptView({
  subagent,
  parentTool,
  onStopSubagent,
  onOpenFile,
}: {
  subagent: SubagentView;
  parentTool: ToolCallView | undefined;
  onStopSubagent?: (parentCallId: string) => void | Promise<unknown>;
  onOpenFile?: (reference: FileReference) => void;
}) {
  const { t } = useCopy();
  const transcript = useMemo(
    () => buildSubagentTranscript(subagent, parentTool),
    [subagent, parentTool],
  );
  if (!transcript) {
    return (
      <InspectorEmpty
        icon={<Component2Icon />}
        title={t("inspector.subagentsNone")}
        body={t("inspector.subagentsNoneBody")}
      />
    );
  }
  const running = subagent.status === "running";
  return (
    <div className="subagent-panel__transcript">
      <div
        className="subagent-panel__log"
        role="log"
        aria-live="polite"
        aria-label={t("inspector.subagentTranscriptLabel")}
      >
        {transcript.turns.map((turn, turnIndex) => (
          <div className="subagent-panel__turn" key={turnIndex}>
            <div className="transcript-item" data-role="user">
              <div className="transcript-item__col" data-role="user">
                <article className="chat-bubble" data-role="user">
                  {turn.prompt || t("inspector.subagentPromptEmpty")}
                </article>
              </div>
            </div>
            {turn.rows.map((row, rowIndex) =>
              row.kind === "tool" ? (
                <ToolCard
                  key={row.tool.id || rowIndex}
                  tool={row.tool}
                  onOpenFile={onOpenFile}
                />
              ) : (
                <div
                  className="transcript-item"
                  data-kind="message"
                  data-role="assistant"
                  key={row.message.id || rowIndex}
                >
                  <div className="transcript-item__col" data-role="assistant">
                    <article
                      className="chat-bubble"
                      data-role="assistant"
                      data-status={row.message.status}
                    >
                      {row.message.thinking ? (
                        <ThinkingRegion
                          thinking={row.message.thinking}
                          streaming={
                            running &&
                            row.message.status === "streaming" &&
                            !row.message.content
                          }
                        />
                      ) : null}
                      {row.message.content ? (
                        <RichText content={row.message.content} />
                      ) : null}
                    </article>
                  </div>
                </div>
              ),
            )}
          </div>
        ))}
      </div>
      <div className="subagent-panel__composer">
        <textarea
          disabled
          rows={2}
          aria-label={t("inspector.subagentReadOnly")}
          placeholder={t("inspector.subagentReadOnly")}
        />
        <div className="subagent-panel__composer-actions">
          <SubagentStopButton
            parentCallId={subagent.parentCallId}
            running={running}
            onStop={onStopSubagent}
          />
        </div>
      </div>
    </div>
  );
}

/** The list row's label: the requested agent, else the prompt's first line. */
function subagentRowTitle(
  subagent: SubagentView,
  tools: ToolCallView[],
): string {
  const parent = tools.find((tool) => tool.id === subagent.parentCallId);
  const args = parent?.args;
  if (args && typeof args === "object" && !Array.isArray(args)) {
    const agent = (args as { agent?: unknown }).agent;
    if (typeof agent === "string" && agent.trim()) {
      return agent.trim();
    }
    const prompt = (args as { prompt?: unknown }).prompt;
    if (typeof prompt === "string" && prompt.trim()) {
      const firstLine = prompt.trim().split("\n", 1)[0]!;
      return firstLine.length > 80 ? `${firstLine.slice(0, 80)}…` : firstLine;
    }
  }
  return subagent.parentCallId;
}
