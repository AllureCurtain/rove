"use client";

import { useCallback, useState, useSyncExternalStore } from "react";

import {
  createComposerDraftStore,
  type ComposerDraftBinding,
  type ComposerSendChip,
} from "./composer-draft-store";

export function useComposerDraft(binding?: ComposerDraftBinding) {
  // Standalone Composer renders retain their original component-local lifetime.
  // The product shell always supplies its layout-owned, identity-scoped store.
  const [localStore] = useState(createComposerDraftStore);
  const store = binding?.store ?? localStore;
  const workspaceId = binding?.workspaceId ?? "";
  const productSessionId = binding?.productSessionId ?? "";
  const getSnapshot = useCallback(
    () => store.getSnapshot({ workspaceId, productSessionId }),
    [store, workspaceId, productSessionId],
  );
  const snapshot = useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot);

  return {
    ...snapshot,
    setText: (text: string) => store.setText({ workspaceId, productSessionId }, text),
    submit: (
      onSend: (message: string) => Promise<boolean> | boolean,
      chips: readonly ComposerSendChip[] = [],
    ) => store.submit({ workspaceId, productSessionId }, onSend, chips),
    recall: (direction: "older" | "newer", cursor: number) =>
      store.recall({ workspaceId, productSessionId }, direction, cursor),
    restore: (text: string) => store.restore({ workspaceId, productSessionId }, text),
    restoreLastSend: () => store.restoreLastSend({ workspaceId, productSessionId }),
    clearLastSend: () => store.clearLastSend({ workspaceId, productSessionId }),
  };
}
