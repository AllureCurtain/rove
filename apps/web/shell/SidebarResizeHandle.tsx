"use client";

import { useEffect, useRef } from "react";
import { useCopy } from "../copy/CopyProvider";
import { SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH } from "./use-sidebar-width";

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
  previewWidth,
  commitWidth,
  cancelPreview,
  onHandleKeyDown,
}: {
  width: number;
  previewWidth: (clientX: number, element: HTMLElement) => void;
  commitWidth: () => void;
  cancelPreview: (element: HTMLElement) => void;
  onHandleKeyDown: (event: {
    key: string;
    shiftKey: boolean;
    preventDefault: () => void;
  }) => void;
}) {
  const { t } = useCopy();
  const handleRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const releaseRef = useRef<(cancelled: boolean) => void>(() => undefined);

  useEffect(() => {
    const handle = handleRef.current;
    if (!handle) {
      return;
    }
    function releaseTarget(cancelled: boolean, target: HTMLDivElement) {
      if (!draggingRef.current) {
        return;
      }
      draggingRef.current = false;
      document.body.classList.remove("is-resizing-column");
      if (cancelled) {
        cancelPreview(target);
      } else {
        commitWidth();
      }
    }
    // An arrow bound to the narrowed `handle`: a hoisted function declaration
    // would lose that narrowing.
    const release = (cancelled: boolean) => releaseTarget(cancelled, handle);
    releaseRef.current = release;
    // The handle element is read through the ref so a mid-drag unmount cannot
    // leave a stale node receiving moves.
    function onMove(event: PointerEvent) {
      const target = handleRef.current;
      if (!draggingRef.current || !target) {
        return;
      }
      previewWidth(event.clientX, target);
    }
    const onUp = () => release(false);
    const onCancel = () => release(true);
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
    handle.addEventListener("pointercancel", onCancel);
    handle.addEventListener("lostpointercapture", onCancel);
    window.addEventListener("blur", onCancel);
    return () => {
      release(true);
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      handle.removeEventListener("pointercancel", onCancel);
      handle.removeEventListener("lostpointercapture", onCancel);
      window.removeEventListener("blur", onCancel);
    };
  }, [cancelPreview, commitWidth, previewWidth]);

  return (
    <div
      ref={handleRef}
      className="sidebar-resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={t("inspector.sidebarWidth")}
      aria-valuenow={width}
      aria-valuetext={t("inspector.sidebarWidthValue", { width })}
      aria-valuemin={SIDEBAR_MIN_WIDTH}
      aria-valuemax={SIDEBAR_MAX_WIDTH}
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
      onKeyDown={(event) => {
        // Escape abandons an in-flight drag, like the panel and band dividers.
        if (event.key === "Escape" && draggingRef.current) {
          event.preventDefault();
          releaseRef.current(true);
          return;
        }
        onHandleKeyDown(event);
      }}
    />
  );
}
