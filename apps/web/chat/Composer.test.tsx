import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { createComposerDraftStore } from "../state/composer-draft-store";
import { Composer } from "./Composer";

describe("Composer", () => {
  it("keeps the stop action available while a busy run accepts steer controls", () => {
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

    expect(html).toContain('aria-label="停止"');
    expect(html).toContain(">发送</button>");
    expect(html).toContain('aria-keyshortcuts="/ Control+Enter Meta+Enter"');
    expect(html).toContain("Ctrl/Cmd+Enter");
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
});
