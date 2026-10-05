import { expect, test } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Segmentation boundaries (design F3).
 *
 * A message is split into prose / fenced-code / prose segments, and each
 * segment is parsed by its own memoized markdown renderer. The memo must not
 * change what the reader sees: every segment still renders, and the prose on
 * both sides of a fence stays in place.
 */

const ANSWER = [
  "# Heading",
  "",
  "Intro paragraph.",
  "",
  "```rust",
  "fn main() {}",
  "```",
  "",
  "Closing paragraph.",
].join("\n");

test("a message with a fenced block renders every segment around it", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Show me the code", ANSWER),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const conversation = page.getByLabel("Conversation");

  await expect(conversation.getByRole("heading", { name: "Heading" })).toBeVisible();
  await expect(conversation.getByText("Intro paragraph.")).toBeVisible();
  await expect(conversation.locator("figure.rich-code")).toContainText("fn main() {}");
  await expect(conversation.getByText("Closing paragraph.")).toBeVisible();

  // The fence is a boundary, not a merger: the three parts keep their order.
  const intro = await conversation.getByText("Intro paragraph.").boundingBox();
  const code = await conversation.locator("figure.rich-code").boundingBox();
  const closing = await conversation.getByText("Closing paragraph.").boundingBox();
  expect(intro).not.toBeNull();
  expect(code).not.toBeNull();
  expect(closing).not.toBeNull();
  expect(intro!.y).toBeLessThan(code!.y);
  expect(code!.y).toBeLessThan(closing!.y);
});
