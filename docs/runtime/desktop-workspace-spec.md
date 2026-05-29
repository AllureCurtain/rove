# Desktop Workspace Future Spec

This is a future workspace design note, not an implementation commitment in the
current runtime. `WorkspaceKind` intentionally does not include `Desktop`.

## Scope

A Desktop workspace would own an explicit OS automation session for local UI
tasks. The workspace boundary would include screenshots, window metadata,
automation traces, downloaded/exported files, and any local state created during
the run.

## Required Capabilities

- enumerate and target windows by stable metadata;
- capture screenshots and optional accessibility trees;
- perform keyboard, pointer, and text-entry actions;
- manage focus changes explicitly;
- store visual artifacts under the workspace state directory;
- report enough state for debugging and replay.

## Safety Boundaries

Desktop automation must be opt-in and permission-gated for actions that can
modify local files, send messages, make purchases, reveal secrets, or interact
with privileged OS prompts. Local security policy should default to deny for
unknown windows and sensitive applications.

## Implementation Gate

Before implementation, write a dedicated plan that covers OS support, tool
schemas, permissions, screenshots, window targeting, local secret handling, and
failure recovery. Do not add runtime `Desktop` stubs before that plan exists.
