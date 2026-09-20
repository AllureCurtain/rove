"use client";

import { useEffect, useRef } from "react";

import { useCopy } from "../copy/CopyProvider";

/**
 * A pointer- and keyboard-resizable vertical handle for the left navigation.
 *
 * The width is committed when the drag settles, matching the existing work
 * panel: no transition on the rail while dragging, so the panel tracks the
 * pointer. Dragging adds a global col-resize cursor and suppresses text
 * selection. A capture that gets stuck (the pointer released outside the
 * window, or the element unmounting mid-drag) is released by the
 * pointercancel/lostpointercapture paths and on unmount.
 */
export function SidebarResizeHandle({
  width,
  settleWidth,
  onHandleKeyDown,
}: {
  width: number;
  settleWidth: (clientX: number, element: HTMLElement) => void;
  onHandleKeyDown: (event: {
    key: string;
    shiftKey: boolean;
    preventDefault: () => void;
  }) => void;
}) {
  const { t } = useCopy();
  const handleRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);

  useEffect(() => {
    const handle = handleRef.current;
    if (!handle) {
      return;
    }
    function release() {
      draggingRef.current = false;
      document.body.classList.remove("is-resizing-column");
    }
    // The handle element is read through the ref so a mid-drag unmount cannot
    // leave a stale node receiving moves.
    function onMove(event: PointerEvent) {
      const target = handleRef.current;
      if (!draggingRef.current || !target) {
        return;
      }
      settleWidth(event.clientX, target);
    }
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", release);
    handle.addEventListener("pointercancel", release);
    handle.addEventListener("lostpointercapture", release);
    window.addEventListener("blur", release);
    return () => {
      release();
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", release);
      handle.removeEventListener("pointercancel", release);
      handle.removeEventListener("lostpointercapture", release);
      window.removeEventListener("blur", release);
    };
  }, [settleWidth]);

  return (
    <div
      ref={handleRef}
      className="sidebar-resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={t("inspector.sidebarWidth")}
      aria-valuenow={width}
      aria-valuemin={200}
      aria-valuemax={360}
      tabIndex={0}
      onPointerDown={(event) => {
        // Pointer-only affordance; keyboard resizing is handled below.
        if (event.pointerType === "mouse" && event.button !== 0) {
          return;
        }
        draggingRef.current = true;
        document.body.classList.add("is-resizing-column");
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onKeyDown={(event) => onHandleKeyDown(event)}
    />
  );
}
