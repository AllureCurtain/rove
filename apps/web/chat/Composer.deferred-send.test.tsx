// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { createComposerDraftStore } from "../state/composer-draft-store";
import { Composer } from "./Composer";

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

async function harness() {
  const store = createComposerDraftStore();
  const first = { workspaceId: "workspace-a", productSessionId: "session-a" };
  store.setText(first, "send this draft");
  const onSend = vi.fn(async () => true);
  const render = async (options: {
    paused?: boolean;
    disabled?: boolean;
    identity?: typeof first;
  } = {}) => act(async () => {
    root.render(<CopyProvider><Composer
      draftBinding={{ store, ...(options.identity ?? first) }}
      disabled={options.disabled ?? false}
      sendPaused={options.paused ?? true}
      busy={false} error={null} profiles={[]} modelConfig={null}
      modelConfigSaving={false} onSend={onSend} onCancel={() => {}}
      onLoadProviderModels={async () => ({ profile_id: "fixture", models: [] })}
      onModelConfigChange={async () => true} controlError={null}
    /></CopyProvider>);
  });
  await render();
  return { store, first, onSend, render };
}

function sendButton() {
  return container.querySelector<HTMLButtonElement>('[data-slot="send"]')!;
}
async function press(mode: "pointer" | "keyboard") {
  await act(async () => {
    if (mode === "pointer") sendButton().click();
    else container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
    }));
  });
}

describe("Composer deferred send", () => {
  for (const mode of ["pointer", "keyboard"] as const) {
    it(`defers an explicit ${mode} send until restoration finishes, once`, async () => {
      const h = await harness();
      expect(sendButton().disabled).toBe(false);
      await press(mode);
      await press(mode);
      expect(container.textContent).toContain("恢复完成后自动发送");
      expect(h.onSend).not.toHaveBeenCalled();
      await h.render({ paused: false });
      expect(h.onSend).toHaveBeenCalledTimes(1);
      expect(h.onSend).toHaveBeenCalledWith("send this draft", [], undefined);
      await h.render({ paused: false });
      expect(h.onSend).toHaveBeenCalledTimes(1);
      expect(h.store.getSnapshot(h.first).text).toBe("");
    });
  }

  it("cancels the armed send when the reader edits its draft", async () => {
    const h = await harness();
    await press("pointer");
    expect(container.textContent).toContain("恢复完成后自动发送");
    await act(async () => h.store.setText(h.first, "keep this edit"));
    await h.render({ paused: false });
    expect(h.onSend).not.toHaveBeenCalled();
    expect(h.store.getSnapshot(h.first).text).toBe("keep this edit");
    expect(container.textContent).not.toContain("恢复完成后自动发送");
  });

  it("cancels across a session switch even when both drafts have identical text", async () => {
    const h = await harness();
    const second = { workspaceId: "workspace-b", productSessionId: "session-b" };
    await act(async () => h.store.setText(second, "send this draft"));
    await press("keyboard");
    expect(container.textContent).toContain("恢复完成后自动发送");
    await h.render({ identity: second, disabled: true });
    await h.render({ identity: second, paused: false });
    await h.render({ identity: h.first, paused: false });
    expect(h.onSend).not.toHaveBeenCalled();
    expect(h.store.getSnapshot(h.first).text).toBe("send this draft");
    expect(h.store.getSnapshot(second).text).toBe("send this draft");
    expect(container.textContent).not.toContain("恢复完成后自动发送");
  });

  it("does not enable an empty or structurally disabled draft", async () => {
    const h = await harness();
    await h.render({ disabled: true });
    expect(sendButton().disabled).toBe(true);
    await press("pointer");
    await h.render({ paused: false });
    expect(h.onSend).not.toHaveBeenCalled();
    await act(async () => h.store.setText(h.first, ""));
    expect(sendButton().disabled).toBe(true);
  });
});
