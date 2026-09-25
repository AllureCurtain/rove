export type ComposerDraftIdentity = Readonly<{
  workspaceId: string;
  productSessionId: string;
}>;

export type ComposerDraftSnapshot = Readonly<{
  text: string;
  version: number;
  submitting: boolean;
  sendFailed: boolean;
}>;

const EMPTY_DRAFT: ComposerDraftSnapshot = Object.freeze({
  text: "",
  version: 0,
  submitting: false,
  sendFailed: false,
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
          publish(key, { ...current, text: "", version: current.version + 1 });
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
      publish(key, { ...current, text, version: current.version + 1, sendFailed: false });
    },
  };
}

export type ComposerDraftStore = ReturnType<typeof createComposerDraftStore>;

export type ComposerDraftBinding = ComposerDraftIdentity & {
  store: ComposerDraftStore;
};
