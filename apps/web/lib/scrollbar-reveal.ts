/**
 * Reveal-while-scrolling for the custom scrollbars in `styles/product.css`,
 * ported from PI-Desktop's `lib/scrollbar-reveal.ts`.
 *
 * `::-webkit-scrollbar` pseudo-elements have no "is scrolling" state, so the
 * stylesheet can only key off `:hover` and an attribute. This module owns the
 * attribute: it listens for `scroll` in the capture phase (scroll does not
 * bubble), marks the element that scrolled, and clears the mark once that element
 * has been quiet for `holdMs`. A document-level scroll cannot carry attributes, so
 * it is attributed to the root element. Programmatic scrolls (pinned follow while
 * a turn streams) count too, which matches native overlay-scrollbar behaviour.
 */

export const SCROLLING_ATTRIBUTE = "data-scrolling";

/** Quiet time after the last scroll event before the thumb settles back. */
export const SCROLLBAR_REVEAL_HOLD_MS = 300;

interface MarkableElement {
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
}

interface RevealRoot {
  documentElement: MarkableElement | null;
  addEventListener(
    type: "scroll",
    listener: (event: { target: unknown }) => void,
    options: AddEventListenerOptions,
  ): void;
  removeEventListener(
    type: "scroll",
    listener: (event: { target: unknown }) => void,
    options: EventListenerOptions,
  ): void;
}

export interface ScrollbarRevealOptions {
  holdMs?: number;
}

function isMarkable(target: unknown): target is MarkableElement {
  return (
    typeof target === "object" &&
    target !== null &&
    typeof (target as MarkableElement).setAttribute === "function" &&
    typeof (target as MarkableElement).removeAttribute === "function"
  );
}

/**
 * Start marking scrolling elements under `root`. Returns a disposer that removes
 * the listener, cancels pending hides, and clears every mark it set.
 */
export function installScrollbarReveal(
  root: RevealRoot,
  options: ScrollbarRevealOptions = {},
): () => void {
  const holdMs = options.holdMs ?? SCROLLBAR_REVEAL_HOLD_MS;
  const timers = new Map<MarkableElement, ReturnType<typeof setTimeout>>();

  const onScroll = (event: { target: unknown }) => {
    const element = isMarkable(event.target) ? event.target : root.documentElement;
    if (!element) {
      return;
    }
    element.setAttribute(SCROLLING_ATTRIBUTE, "");
    const pending = timers.get(element);
    if (pending !== undefined) {
      clearTimeout(pending);
    }
    timers.set(
      element,
      setTimeout(() => {
        timers.delete(element);
        element.removeAttribute(SCROLLING_ATTRIBUTE);
      }, holdMs),
    );
  };

  root.addEventListener("scroll", onScroll, { capture: true, passive: true });

  return () => {
    root.removeEventListener("scroll", onScroll, { capture: true });
    for (const [element, timer] of timers) {
      clearTimeout(timer);
      element.removeAttribute(SCROLLING_ATTRIBUTE);
    }
    timers.clear();
  };
}
