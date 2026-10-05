// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { createComposerDraftStore } from "../state/composer-draft-store";
import { Composer } from "./Composer";

/**
 * A transcript re-read pauses the send, not the composer.
 *
 * A stop makes the shell re-read the transcript for the session on screen. While
 * that read is in flight the composer used to be disabled outright, so a
 * `Control+Enter` pressed in the window never reached the textarea at all and the
 * reader had to press again. Mounted in jsdom because the contract is about what
 * happens after the pause lifts, which exists only across renders.
 */

const IDENTITY = { workspaceId: "workspace-1", productSessionId: "session-1" };

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

function composerElement(options: {
  store: ReturnType<typeof createComposerDraftStore>;
  sendPaused: boolean;
  disabled?: boolean;
  onSend: (message: string) => Promise<boolean> | boolean;
}) {
  return (
    <CopyProvider>
      <Composer
        draftBinding={{ store: options.store, ...IDENTITY }}
        disabled={options.disabled ?? false}
        sendPaused={options.sendPaused}
        busy={false}
        disabledReason="正在完成会话恢复…"
        error={null}
        profiles={[]}
        modelConfig={null}
        modelConfigSaving={false}
        onSend={options.onSend}
        onCancel={() => {}}
        onLoadProviderModels={async () => ({ profile_id: "fixture", models: [] })}
        onModelConfigChange={async () => false}
        controlError={null}
      />
    </CopyProvider>
  );
}

async function render(options: Parameters<typeof composerElement>[0]) {
  await act(async () => {
    root.render(composerElement(options));
  });
}

function textarea(): HTMLTextAreaElement {
  const found = container.querySelector("textarea");
  if (!found) {
    throw new Error("composer textarea missing");
  }
  return found;
}

async function pressControlEnter() {
  await act(async () => {
    textarea().dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
}

describe("Composer while a transcript re-read is in flight", () => {
  it("keeps typing alive and sends the pressed draft once the read lands", async () => {
    const store = createComposerDraftStore();
    store.setText(IDENTITY, "stop me");
    const onSend = vi.fn(async () => true);

    await render({ store, sendPaused: true, onSend });
    // The window is explained, but it does not take the input away.
    expect(container.textContent).toContain("正在完成会话恢复");
    expect(textarea().disabled).toBe(false);

    await pressControlEnter();
    expect(onSend).not.toHaveBeenCalled();
    expect(container.textContent).toContain("恢复完成后自动发送");

    await render({ store, sendPaused: false, onSend });
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("stop me", []);
    expect(container.textContent).not.toContain("恢复完成后自动发送");
  });

  it("spends the press on the draft the reader submitted, not on a later edit", async () => {
    const store = createComposerDraftStore();
    store.setText(IDENTITY, "first version");
    const onSend = vi.fn(async () => true);

    await render({ store, sendPaused: true, onSend });
    await pressControlEnter();
    expect(container.textContent).toContain("恢复完成后自动发送");

    // The reader keeps writing while the re-read runs; the deferred send belongs
    // to the version that was submitted, so it is dropped rather than sent.
    await act(async () => {
      store.setText(IDENTITY, "second version");
    });
    expect(container.textContent).not.toContain("恢复完成后自动发送");

    await render({ store, sendPaused: false, onSend });
    expect(onSend).not.toHaveBeenCalled();
    expect(textarea().value).toBe("second version");
  });

  it("still locks the composer when the shell cannot bind a send at all", async () => {
    const store = createComposerDraftStore();
    store.setText(IDENTITY, "stop me");
    const onSend = vi.fn(async () => true);

    await render({ store, sendPaused: false, disabled: true, onSend });
    expect(textarea().disabled).toBe(true);

    await pressControlEnter();
    await render({ store, sendPaused: false, disabled: true, onSend });
    expect(onSend).not.toHaveBeenCalled();
    expect(textarea().value).toBe("stop me");
  });
});
