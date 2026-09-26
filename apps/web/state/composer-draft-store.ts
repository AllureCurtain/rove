export type ComposerDraftIdentity = Readonly<{
  workspaceId: string;
  productSessionId: string;
}>;

/**
 * A paste chip as the draft store knows it: the content, nothing else. The
 * composer owns ids and rendering.
 */
export type ComposerSendChip = Readonly<{ text: string }>;

/**
 * What the last accepted send looked like before it was folded into one string
 * (design F6). A smart stop restores from this instead of un-folding the fence
 * markers in the sent text, which cannot be done faithfully.
 */
export type ComposerSendSnapshot = Readonly<{
  /** The draft text as typed, without the folded attachments. */
  text: string;
  chips: readonly ComposerSendChip[];
  at: number;
}>;

/** How the composer draft was last put back, so the UI can say where from. */
export type ComposerDraftRestore = Readonly<{
  /** Monotonic: a second restore of the same kind still has to re-fire. */
  seq: number;
  source: "snapshot" | "text";
  chips: readonly ComposerSendChip[];
}>;

export type ComposerDraftSnapshot = Readonly<{
  text: string;
  version: number;
  submitting: boolean;
  sendFailed: boolean;
  lastSend: ComposerSendSnapshot | null;
  lastRestore: ComposerDraftRestore | null;
}>;

const EMPTY_DRAFT: ComposerDraftSnapshot = Object.freeze({
  text: "",
  version: 0,
  submitting: false,
  sendFailed: false,
  lastSend: null,
  lastRestore: null,
});

function draftKey(identity: ComposerDraftIdentity): string {
  return JSON.stringify([identity.workspaceId, identity.productSessionId]);
}

/** Owned by the product layout, not a module singleton or durable cache.
 * No eviction: unsent text survives client navigation until that layout unloads.
 */
export function createComposerDraftStore() {
  const drafts = new Map<string, ComposerDraftSnapshot>();
  const sentHistory = new Map<string, string[]>();
  const listeners = new Set<() => void>();
  const read = (key: string) => drafts.get(key) ?? EMPTY_DRAFT;
  function publish(key: string, draft: ComposerDraftSnapshot) {
    drafts.set(key, Object.freeze(draft));
    listeners.forEach((listener) => listener());
  }

   const SENT_HISTORY_LIMIT = 50;
  return {
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    getSnapshot(identity: ComposerDraftIdentity) {
      return read(draftKey(identity));
    },
    setText(identity: ComposerDraftIdentity, text: string) {
      const key = draftKey(identity);
      const current = read(key);
      publish(key, {
        ...current,
        text,
        version: current.version + 1,
        sendFailed: false,
      });
    },
    async submit(
      identity: ComposerDraftIdentity,
      onSend: (message: string) => Promise<boolean> | boolean,
      chips: readonly ComposerSendChip[] = [],
    ): Promise<boolean> {
      // Capture both identity and immutable text/version before invoking onSend.
      const key = draftKey(identity);
      const snapshot = read(key);
      const message = snapshot.text.trim();
      if (snapshot.submitting || !message) return false;
      // Synchronous publication guards duplicate events and remounted consumers.
      publish(key, { ...snapshot, submitting: true, sendFailed: false });
      try {
        const accepted = await onSend(message);
        if (accepted) {
          const history = sentHistory.get(key) ?? [];
          sentHistory.set(key, [...history, message].slice(-SENT_HISTORY_LIMIT));
        }
        const current = read(key);
        if (accepted && current.version === snapshot.version) {
          publish(key, {
            ...current,
            text: "",
            version: current.version + 1,
            // The snapshot is written from the text as typed, not from the
            // folded message: that is the whole point of keeping it.
            lastSend: {
              text: snapshot.text,
              chips: [...chips],
              at: Date.now(),
            },
          });
        }
        return accepted;
      } catch {
        // Never expose thrown provider/network details or retry automatically.
        const current = read(key);
        if (current.version === snapshot.version) {
          publish(key, { ...current, sendFailed: true });
        }
        return false;
      } finally {
        publish(key, { ...read(key), submitting: false });
      }
    },
    recall(identity: ComposerDraftIdentity, direction: "older" | "newer", cursor: number): { text: string; cursor: number } | null {
      const history = sentHistory.get(draftKey(identity)) ?? [];
      if (history.length === 0) return null;
      const next = direction === "older" ? Math.min(history.length - 1, cursor + 1) : Math.max(-1, cursor - 1);
      if (next < 0) return { text: "", cursor: -1 };
      return { text: history[history.length - 1 - next] ?? "", cursor: next };
    },
    restore(identity: ComposerDraftIdentity, text: string) {
      const key = draftKey(identity);
      const current = read(key);
      publish(key, {
        ...current,
        text,
        version: current.version + 1,
        sendFailed: false,
        lastRestore: {
          seq: (current.lastRestore?.seq ?? 0) + 1,
          source: "text",
          chips: [],
        },
      });
    },
    /**
     * Smart-stop restore (design F6): put the last accepted send back exactly as
     * it was, chips included. Returns the source so the caller can say where the
     * draft came from; the snapshot is consumed, because a restore is one-shot.
     */
    restoreLastSend(
      identity: ComposerDraftIdentity,
    ): "snapshot" | "none" {
      const key = draftKey(identity);
      const current = read(key);
      if (current.lastSend === null) {
        return "none";
      }
      const { text, chips } = current.lastSend;
      publish(key, {
        ...current,
        text,
        version: current.version + 1,
        sendFailed: false,
        lastSend: null,
        lastRestore: {
          seq: (current.lastRestore?.seq ?? 0) + 1,
          source: "snapshot",
          chips,
        },
      });
      return "snapshot";
    },
    /**
     * The turn reached a terminal state, so its send is no longer something a
     * stop could legitimately put back. A stop that restored already consumed
     * the snapshot, so this only ever drops a stale one.
     */
    clearLastSend(identity: ComposerDraftIdentity) {
      const key = draftKey(identity);
      const current = read(key);
      if (current.lastSend === null) {
        return;
      }
      publish(key, { ...current, lastSend: null });
    },
  };
}

export type ComposerDraftStore = ReturnType<typeof createComposerDraftStore>;

export type ComposerDraftBinding = ComposerDraftIdentity & {
  store: ComposerDraftStore;
};
