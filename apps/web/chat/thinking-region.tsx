"use client";

import { ChevronDownIcon } from "@radix-ui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { RichText } from "../product-v2/RichText";

/**
 * The provider's chain-of-thought beside an answer, in one quiet fold.
 *
 * Reasoning is deliberately de-emphasized — a secondary stream the model
 * produced before its reply, not part of the reply — so it collapses once the
 * answer exists and expands only while the model is still thinking aloud.
 * The body is the raw thinking text; the bubble's own copy surface still
 * copies `content` only.
 *
 * Shared by the main transcript and the work-panel subagent mini transcript —
 * a delegate's reasoning reads exactly like the parent's.
 */
export function ThinkingRegion({
  thinking,
  streaming,
}: {
  thinking: string;
  streaming: boolean;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(streaming);
  // The fold opens while the model reasons and settles shut once the answer
  // lands — unless the reader already took it over.
  const userToggledRef = useRef(false);
  const bodyId = useMemo(() => `thinking-${crypto.randomUUID()}`, []);
  useEffect(() => {
    if (!streaming && !userToggledRef.current) {
      setOpen(false);
    }
  }, [streaming]);
  return (
    <section
      className="thinking-region"
      data-open={open ? "true" : undefined}
      data-streaming={streaming ? "true" : undefined}
    >
      <button
        type="button"
        className="thinking-region__head"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={() => {
          userToggledRef.current = true;
          setOpen((value) => !value);
        }}
      >
        <ChevronDownIcon data-open={open} />
        <span>
          {streaming ? t("chat.thinkingStreaming") : t("chat.thinking")}
        </span>
      </button>
      {open ? (
        <div className="thinking-region__body" id={bodyId}>
          <RichText content={thinking} />
        </div>
      ) : null}
    </section>
  );
}
