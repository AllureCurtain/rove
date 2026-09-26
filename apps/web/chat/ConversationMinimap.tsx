"use client";

import { useCallback, useRef, type KeyboardEvent } from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  shouldRenderConversationMinimap,
  type ConversationMinimapMarker,
} from "./conversation-minimap";

/**
 * The rail that lets a reader move between turns of a long conversation, ported
 * from PI-Desktop's conversation minimap.
 *
 * Divergences from the reference, both deliberate: markers jump a *rendered*
 * timeline item by id (so a marker can never point at a turn that is not on
 * screen), and the rail hides itself while the reading column is too narrow to
 * leave it a lane, instead of overlapping the prose.
 */
export function ConversationMinimap({
  markers,
  overflows,
  hasEarlier,
  onJumpTo,
}: {
  markers: ConversationMinimapMarker[];
  overflows: boolean;
  hasEarlier: boolean;
  onJumpTo: (messageId: string) => void;
}) {
  const { t } = useCopy();
  const railRef = useRef<HTMLElement | null>(null);

  const focusMarker = useCallback((index: number) => {
    const buttons = railRef.current?.querySelectorAll<HTMLButtonElement>(
      "[data-marker-index]",
    );
    buttons?.[index]?.focus();
  }, []);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLElement>) => {
      const buttons = railRef.current?.querySelectorAll<HTMLButtonElement>(
        "[data-marker-index]",
      );
      if (!buttons || buttons.length === 0) {
        return;
      }
      const last = buttons.length - 1;
      const focused = Array.prototype.indexOf.call(buttons, document.activeElement);
      if (event.key === "ArrowUp") {
        focusMarker(focused < 0 ? last : Math.max(0, focused - 1));
      } else if (event.key === "ArrowDown") {
        focusMarker(focused < 0 ? 0 : Math.min(last, focused + 1));
      } else if (event.key === "Home") {
        focusMarker(0);
      } else if (event.key === "End") {
        focusMarker(last);
      } else {
        return;
      }
      event.preventDefault();
    },
    [focusMarker],
  );

  if (!shouldRenderConversationMinimap({
    markerCount: markers.length,
    overflows,
    hasEarlier,
  })) {
    return null;
  }

  return (
    <nav
      ref={railRef}
      className="conversation-minimap"
      aria-label={t("chat.minimapLabel")}
      data-count={markers.length}
      onKeyDown={handleKeyDown}
    >
      {markers.map((marker, index) => (
        <button
          key={marker.id}
          type="button"
          className="conversation-minimap__marker"
          data-role={marker.role}
          data-marker-index={index}
          aria-label={t("chat.minimapMarker", {
            n: index + 1,
            total: markers.length,
            role: marker.role === "user" ? t("chat.you") : t("chat.assistant"),
          })}
          title={marker.preview}
          onClick={() => onJumpTo(marker.id)}
        >
          <span aria-hidden="true" />
        </button>
      ))}
    </nav>
  );
}
