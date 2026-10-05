import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { createComposerDraftStore } from "../state/composer-draft-store";
import { Composer } from "./Composer";

describe("Composer", () => {
  it("offers stop in the slot while a busy run has no draft", () => {
    const html = renderToStaticMarkup(
      <CopyProvider>
        <Composer
          disabled
          busy
          error={null}
          profiles={[]}
          modelConfig={{
            sessionId: "session-1",
            model: "fake",
            reasoning: "default",
            maxSteps: 8,
            revision: 1,
            updatedAt: "2026-07-26T00:00:00.000Z",
          }}
          modelConfigSaving={false}
          onSend={vi.fn()}
          onCancel={vi.fn()}
          onLoadProviderModels={vi.fn(async () => ({
            profile_id: "profile-1",
            models: [],
          }))}
          onModelConfigChange={vi.fn(async () => true)}
          controlError={null}
        />
      </CopyProvider>,
    );

    // §6.1: one slot, and with no draft typed it is the stop side of the mutex.
    expect(html).toContain('data-slot="stop"');
    expect(html).not.toContain('data-slot="send"');
    expect(html).toContain('aria-label="停止"');
    // §6.2: the advertised chords match the Enter-to-send default.
    expect(html).toContain('aria-keyshortcuts="Enter Alt+Enter Control+Enter Meta+Enter"');
    // The persistent hint keeps the binding readable outside the placeholder.
    expect(html).toContain("Enter 发送");
    expect(html).toContain("Shift+Enter 换行");
    expect(html).toContain("Alt+Enter 插话");
  });
  it("shows the current continuity error instead of masking it with a prior send failure", async () => {
    const store = createComposerDraftStore();
    const identity = { workspaceId: "workspace-1", productSessionId: "session-1" };
    store.setText(identity, "retain draft");
    await store.submit(identity, () => { throw new Error("private failure details"); });
    const html = renderToStaticMarkup(
      <CopyProvider>
        <Composer
          draftBinding={{ store, ...identity }}
          disabled={false} busy={false} error="Current connection unavailable"
          profiles={[]} modelConfig={null} modelConfigSaving={false}
          onSend={() => false} onCancel={() => {}}
          onLoadProviderModels={async () => ({ profile_id: "fixture", models: [] })}
          onModelConfigChange={async () => false} controlError={null}
        />
      </CopyProvider>,
    );
    expect(html).toContain("Current connection unavailable");
    expect(html).toContain("retain draft");
    expect(html).not.toContain("private failure details");
  });

  it("offers a file attachment control only when the shell can upload", () => {
    function render(onUploadAttachment?: () => Promise<never>) {
      return renderToStaticMarkup(
        <CopyProvider>
          <Composer
            disabled={false}
            busy={false}
            error={null}
            profiles={[]}
            modelConfig={null}
            modelConfigSaving={false}
            onSend={() => false}
            onUploadAttachment={onUploadAttachment}
            onCancel={() => {}}
            onLoadProviderModels={async () => ({ profile_id: "fixture", models: [] })}
            onModelConfigChange={async () => false}
            controlError={null}
          />
        </CopyProvider>,
      );
    }

    const withUpload = render(async () => {
      throw new Error("unused");
    });
    expect(withUpload).toContain('aria-label="添加附件"');
    // The allow-list is the server's, so the picker cannot offer a file the
    // server would refuse.
    expect(withUpload).toContain('accept="png,jpg,jpeg,webp,gif,pdf,txt,md"');
    expect(withUpload).toContain('type="file"');

    const withoutUpload = render();
    expect(withoutUpload).not.toContain('aria-label="添加附件"');
    expect(withoutUpload).not.toContain('type="file"');
  });
});
