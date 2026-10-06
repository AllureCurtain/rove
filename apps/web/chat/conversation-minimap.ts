/**
 * Conversation minimap markers.
 *
 * The rail is a *navigation* surface for a long transcript: one marker per
 * conversational turn so a reader can jump back to a turn instead of scrolling.
 * The builder is pure and takes the smallest possible message shape, so it can be
 * unit tested without the product projection.
 */

export interface ConversationMinimapMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
}

export interface ConversationMinimapMarker {
  id: string;
  role: "user" | "assistant";
  preview: string;
}

/** Preview text kept per marker; longer turns are truncated for the tooltip. */
export const CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS = 280;

/**
 * A one-marker rail is noise, so it only appears once there is something to
 * navigate: either the transcript overflows, or earlier turns are not loaded at
 * all (in which case the rail is also a reminder that history continues above).
 */
export function shouldRenderConversationMinimap({
  markerCount,
  overflows,
  hasEarlier,
}: {
  markerCount: number;
  overflows: boolean;
  hasEarlier: boolean;
}): boolean {
  return hasEarlier || (markerCount >= 2 && overflows);
}

function appendPreview(current: string, next: string): string {
  if (!next || current.length >= CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS) {
    return current;
  }
  return `${current}${current ? "\n\n" : ""}${next}`.slice(
    0,
    CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS,
  );
}

/**
 * Marker tops inside the rail's usable track, in pixels.
 *
 * Each marker hugs its turn's own position — the turn's fraction of the
 * scrollable content maps onto the track — so the rail reads as a minimap
 * rather than an index: dense runs spread to a minimum pitch instead of
 * stacking on one spot, and a matching backward pass keeps the tail from
 * overflowing the rail. Positions stay document-ordered and every marker is
 * clamped onto the track, so a transcript that out-densifies the rail degrades
 * to stacking at the ends rather than losing markers.
 */
export function alignMinimapMarkers(
  fractions: readonly number[],
  trackHeight: number,
  markerHeight: number,
  minGap: number,
): number[] {
  if (fractions.length === 0) {
    return [];
  }
  const pitch = markerHeight + minGap;
  const usable = Math.max(0, trackHeight - markerHeight);
  const tops = fractions.map(
    (fraction) => Math.min(1, Math.max(0, fraction)) * usable,
  );
  for (let index = 1; index < tops.length; index += 1) {
    tops[index] = Math.max(tops[index], tops[index - 1] + pitch);
  }
  for (let index = tops.length - 2; index >= 0; index -= 1) {
    tops[index] = Math.min(tops[index], tops[index + 1] - pitch);
  }
  return tops.map((top) => Math.min(usable, Math.max(0, top)));
}

/**
 * One user marker per user message, and one assistant marker per turn: a turn's
 * assistant prose can arrive as several messages (streamed chunks, an
 * intermediate note, the final answer), and they belong to the same marker.
 */
export function buildConversationMinimapMarkers(
  messages: ConversationMinimapMessage[],
): ConversationMinimapMarker[] {
  const markers: ConversationMinimapMarker[] = [];
  let assistantMarkerIndex: number | null = null;

  for (const message of messages) {
    if (message.role === "user") {
      markers.push({
        id: message.id,
        role: "user",
        preview: (message.content || "")
          .trim()
          .slice(0, CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS),
      });
      assistantMarkerIndex = null;
      continue;
    }
    if (message.role !== "assistant") {
      continue;
    }

    const content = (message.content || "").trim();
    if (!content) {
      continue;
    }
    if (assistantMarkerIndex === null) {
      markers.push({
        id: message.id,
        role: "assistant",
        preview: content.slice(0, CONVERSATION_MINIMAP_PREVIEW_MAX_CHARS),
      });
      assistantMarkerIndex = markers.length - 1;
      continue;
    }

    const marker = markers[assistantMarkerIndex];
    marker.preview = appendPreview(marker.preview, content);
  }

  return markers;
}
