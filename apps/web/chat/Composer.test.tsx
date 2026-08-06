import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { Composer } from "./Composer";

describe("Composer", () => {
  it("keeps the stop action available while a busy run accepts steer controls", () => {
    const html = renderToStaticMarkup(
      <Composer
        disabled
        busy
        controlAvailable
        resumeLabel="continuity: exact product session"
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
        onSteer={vi.fn(() => true)}
        onFollowup={vi.fn(() => true)}
        onCancel={vi.fn()}
        onLoadProviderModels={vi.fn(async () => ({
          profile_id: "profile-1",
          models: [],
        }))}
        onModelConfigChange={vi.fn(async () => true)}
        controls={[]}
        controlsLoading={false}
        controlBusy={null}
        controlError={null}
        onRefreshControls={vi.fn()}
        onRevokeControl={vi.fn()}
        onConfirmFollowup={vi.fn()}
      />,
    );

    expect(html).toContain('aria-label="Stop run"');
    expect(html).toContain(">Steer</button>");
    expect(html).toContain(">Follow-up</button>");
  });
});
