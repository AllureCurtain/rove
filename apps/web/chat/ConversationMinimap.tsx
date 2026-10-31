"use client";

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  alignMinimapMarkers,
  shouldRenderConversationMinimap,
  type ConversationMinimapMarker,
} from "./conversation-minimap";

/** Marker hit area height; paired with MINIMAP_MARKER_GAP for the pitch clamp. */
const MINIMAP_MARKER_HEIGHT = 14;
const MINIMAP_MARKER_GAP = 3;

/* Dock magnification (issue #32): the cosine falloff's reach and peak boost,
   and how close the cursor must sit to a dash before the preview picks it. */
const MINIMAP_MAGNIFY_RADIUS = 46;
const MINIMAP_MAGNIFY_BOOST = 1.3;
const MINIMAP_POPOVER_SNAP = 24;
const MINIMAP_POPOVER_HEIGHT = 132;

/**
 * The rail that lets a reader move between turns of a long conversation.
 *
 * Three deliberate properties: markers jump a *rendered* timeline item by id
 * (so a marker can never point at a turn that is not on screen); each marker
 * sits at its turn's own position on the rail — the fraction of the scrollable
 * content the turn occupies — so the rail reads as a minimap of the document,
 * with the translucent band marking the visible window; and the rail hides
 * itself while the reading column is too narrow to leave it a lane, instead of
 * overlapping the prose.
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
  const markerEls = useRef(new Map<string, HTMLButtonElement>());
  const magnifyCentersRef = useRef<{ id: string; center: number }[]>([]);
  const moveRaf = useRef(0);
  const [hovered, setHovered] = useState<{
    marker: ConversationMinimapMarker;
    top: number;
  } | null>(null);
  const rendered = shouldRenderConversationMinimap({
    markerCount: markers.length,
    overflows,
    hasEarlier,
  });

  /*
   * Marker positions and the viewport band are DOM measurements, not React
   * state: they change with layout and every scroll frame, and writing style
   * directly keeps scroll frames off the render path — the same policy the
   * width handles and composer-dock reserve follow. The rail re-measures on
   * content resize (streaming growth, loaded history) and on marker changes.
   */
  useEffect(() => {
    if (!rendered) {
      return;
    }
    const rail = railRef.current;
    const frame = rail?.parentElement;
    const scroller = frame?.querySelector<HTMLElement>(".chat-transcript");
    const content = scroller?.firstElementChild as HTMLElement | null;
    if (!rail || !scroller || !content) {
      return;
    }

    const positionMarkers = () => {
      const trackHeight = rail.clientHeight;
      const contentHeight = content.scrollHeight || content.offsetHeight;
      if (trackHeight <= 0 || contentHeight <= 0) {
        return;
      }
      const contentTop = content.getBoundingClientRect().top;
      const fractions = markers.map((marker) => {
        const target = scroller.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(marker.id)}"]`,
        );
        // A marker whose turn fell out of the window keeps the previous edge:
        // an unanchored marker is still clickable only while rendered, so report
        // the very top rather than inventing a position.
        if (!target) {
          return 0;
        }
        return (target.getBoundingClientRect().top - contentTop) / contentHeight;
      });
      const tops = alignMinimapMarkers(
        fractions,
        trackHeight,
        MINIMAP_MARKER_HEIGHT,
        MINIMAP_MARKER_GAP,
      );
      const nodes = rail.querySelectorAll<HTMLElement>("[data-marker-index]");
      const centers: { id: string; center: number }[] = [];
      nodes.forEach((node, index) => {
        const top = tops[index] ?? 0;
        node.style.top = `${top}px`;
        const id = markers[index]?.id;
        if (id) {
          centers.push({ id, center: top + MINIMAP_MARKER_HEIGHT / 2 });
        }
      });
      magnifyCentersRef.current = centers;
    };

    const positionViewport = () => {
      const total = scroller.scrollHeight;
      if (total <= 0) {
        return;
      }
      rail.style.setProperty(
        "--minimap-view-top",
        `${Math.min(1, Math.max(0, scroller.scrollTop / total)) * 100}%`,
      );
      rail.style.setProperty(
        "--minimap-view-height",
        `${Math.min(1, Math.max(0, scroller.clientHeight / total)) * 100}%`,
      );
    };

    const update = () => {
      positionMarkers();
      positionViewport();
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(content);
    observer.observe(rail);
    scroller.addEventListener("scroll", positionViewport, { passive: true });
    return () => {
      observer.disconnect();
      scroller.removeEventListener("scroll", positionViewport);
    };
  }, [markers, rendered]);

  /*
   * Dock magnification (issue #32): dashes scale horizontally only, so the
   * hover pass never shifts the stack's layout. Centers come from the position
   * pass above — reading offsetTop inside this loop while writing `--magnify`
   * would force one layout per dash on every mousemove frame. The nearest dash
   * inside the snap distance also owns the preview popover.
   */
  const applyMagnify = useCallback((cursorY: number | null) => {
    const rail = railRef.current;
    let nearest: { id: string; dist: number; center: number } | null = null;
    for (const { id, center } of magnifyCentersRef.current) {
      const button = markerEls.current.get(id);
      if (!button) {
        continue;
      }
      let scale = 1;
      if (cursorY !== null) {
        const dist = Math.abs(center - cursorY);
        if (dist < MINIMAP_MAGNIFY_RADIUS) {
          scale =
            1 +
            MINIMAP_MAGNIFY_BOOST *
              Math.cos((dist / MINIMAP_MAGNIFY_RADIUS) * (Math.PI / 2));
        }
        if (dist <= MINIMAP_POPOVER_SNAP && (!nearest || dist < nearest.dist)) {
          nearest = { id, dist, center };
        }
      }
      button.style.setProperty("--minimap-magnify", scale.toFixed(3));
    }
    if (nearest && rail) {
      const marker = markers.find((entry) => entry.id === nearest!.id);
      if (marker) {
        const top = Math.min(
          Math.max(nearest.center - 36, 0),
          Math.max(rail.clientHeight - MINIMAP_POPOVER_HEIGHT, 0),
        );
        setHovered((previous) =>
          previous?.marker.id === marker.id && previous.top === top
            ? previous
            : { marker, top },
        );
        return;
      }
    }
    setHovered(null);
  }, [markers]);

  const handleMouseMove = useCallback(
    (event: ReactMouseEvent) => {
      const rail = railRef.current;
      if (!rail) {
        return;
      }
      const y = event.clientY - rail.getBoundingClientRect().top;
      cancelAnimationFrame(moveRaf.current);
      moveRaf.current = requestAnimationFrame(() => applyMagnify(y));
    },
    [applyMagnify],
  );

  const handleMouseLeave = useCallback(() => {
    cancelAnimationFrame(moveRaf.current);
    applyMagnify(null);
  }, [applyMagnify]);

  useEffect(() => () => cancelAnimationFrame(moveRaf.current), []);

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

  if (!rendered) {
    return null;
  }

  return (
    <nav
      ref={railRef}
      className="conversation-minimap"
      aria-label={t("chat.minimapLabel")}
      data-count={markers.length}
      onKeyDown={handleKeyDown}
      onMouseMove={handleMouseMove}
      onMouseLeave={handleMouseLeave}
    >
      {/* The visible window of the transcript, decorative — the scroller itself
          already reports position to assistive tech. */}
      <span className="conversation-minimap__viewport" aria-hidden="true" />
      {markers.map((marker, index) => (
        <button
          key={marker.id}
          ref={(node) => {
            if (node) {
              markerEls.current.set(marker.id, node);
            } else {
              markerEls.current.delete(marker.id);
            }
          }}
          type="button"
          className="conversation-minimap__marker"
          data-role={marker.role}
          data-marker-index={index}
          aria-label={t("chat.minimapMarker", {
            n: index + 1,
            total: markers.length,
            role: marker.role === "user" ? t("chat.you") : t("chat.assistant"),
          })}
          onFocus={(event) =>
            // Focus gets the popover too — keyboard readers see what hover does.
            setHovered({
              marker,
              top: Math.max(0, event.currentTarget.offsetTop - 36),
            })
          }
          onBlur={() => setHovered(null)}
          onClick={() => onJumpTo(marker.id)}
        >
          <span aria-hidden="true" />
        </button>
      ))}
      {hovered && hovered.marker.preview ? (
        <div
          className="conversation-minimap__popover"
          role="tooltip"
          style={{ top: `${hovered.top}px` }}
        >
          <div className="conversation-minimap__popover-role">
            {hovered.marker.role === "user" ? t("chat.you") : t("chat.assistant")}
          </div>
          <div className="conversation-minimap__popover-text">
            {hovered.marker.preview}
          </div>
        </div>
      ) : null}
    </nav>
  );
}
