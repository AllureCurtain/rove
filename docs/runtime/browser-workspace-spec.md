# Browser Workspace Future Spec

This is a future workspace design note, not an implementation commitment in the
current runtime. `WorkspaceKind` intentionally does not include `Browser`.

## Scope

A Browser workspace would own an isolated browser context for web tasks. The
workspace boundary would include navigation history, cookies/session storage,
downloads, screenshots, and any files exported from the browser context.

## Required Capabilities

- create and dispose isolated browser contexts;
- navigate, reload, and inspect the current page URL/title;
- capture screenshots and structured page snapshots;
- interact with controls through explicit, auditable actions;
- manage downloads under the workspace state directory;
- expose browser events without leaking hidden credentials.

## Safety Boundaries

Browser automation must be permission-gated for credentialed sessions,
cross-origin downloads, file uploads, payment or account actions, and destructive
site operations. Screenshots and snapshots should be stored as artifacts with
clear retention and cleanup behavior.

## Implementation Gate

Before implementation, write a dedicated plan that covers browser lifecycle,
tool schemas, approval policy, artifact storage, replay/debuggability, and Web
UI rendering. Do not add runtime `Browser` stubs before that plan exists.
