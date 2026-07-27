# rove Web

Primary surface: the **product shell** (Workspace → Session → Run) against live
`rove-api`. The old developer workbench is not a second primary entry.

## Product shell (default `/`)

- Empty state → open an absolute Folder/Repo path
- API-backed workspace tree with durable sessions, recents, and pin state
- Parallel-session **running** badges in the sidebar
- Refresh-restored chat transcript with explicit partial/error/retry states,
  tool cards, inline approvals, and stop/cancel
- Collapsible run inspector with empty / loading / error / ready states
- Durable `/w/:workspaceId/s/:sessionId` and `/settings/:section` routes
- A fail-closed M1 browser migration gate that runs before product catalog reads,
  preserves the exact retry payload, and rewrites legacy deep routes only after
  a validated server acknowledgement
- Complete nine-section Settings shell with provider CRUD, approval defaults,
  workspace/session and Memory management, runtime health, keyboard shortcuts,
  and **Advanced / Developer** (Benchmark only here)
- Light default + dark toggle via product design tokens
- Responsive 390px/320px chat and Settings layouts, bounded mobile Inspector,
  visible focus, modal focus containment/restoration, live status semantics,
  reduced-motion handling, and representative light/dark screenshot artifacts
- Session continue uses the server-owned exact `product_session_id` binding.
  The product shell omits client `resume`; soft transcript stitch and
  workspace-global `latest` are not product paths.

## C0-C3 product state, continuity, migration, and polish

`apps/web/product/` now contains strict product API response validation, a thin
client for workspace/session/profile/preferences/transcript endpoints, and a
versioned replay-safe M1 browser migration state machine. `CreateJobRequest`
also accepts the server-owned `product_session_id` path implemented by the Rust
API.

The default `ProductApp` now loads workspaces, sessions, safe preferences, and
provider profiles from that client. Entering a session route projects its
canonical event transcript; switching sessions closes the old observation and
reattaches only the focused live job. Background running/attention badges are
refreshed from the durable catalog. If `POST /jobs` may have committed before a
network failure, the shell performs bounded binding checks and reattaches or
restores the transcript without automatically submitting a duplicate turn.

`ProductApp` now invokes the replay-safe M1 browser migration before mounting
the server-backed product state hook. Pending, rejected, blocked, and
superseded outcomes fail closed without substituting an empty catalog; retry
reuses the exact durable idempotency key and body. A newly completed migration
shows one dismissible summary, while refresh of an existing receipt does not
repeat the notice. Legacy workspace/session deep routes are rewritten through
validated server mappings while preserving query and fragment state.

C2 adds revision-CAS preference writes, the default approval policy used by
product jobs, bounded durable-memory and runtime-health clients, and complete
Settings routes. Workspace pin/remove, session rename/delete/safe catalog
export, provider edit/update, Memory read/delete, and four keyboard shortcuts
operate without browser-local catalog authority.

C3 completes narrow-layout reflow, mobile Inspector containment, accessible
dialogs and dynamic run status, higher-contrast focus treatment, reduced-motion
behavior, server-confirmed theme bootstrap, deep Settings-tab visibility, and
visual evidence for representative mobile/light/dark recovery states.

## Run locally

From the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
```

Or start pieces separately:

```powershell
# API
cargo run -p rove-api

# Web (from apps/web)
pnpm install --frozen-lockfile
pnpm dev
```

Open `http://localhost:3000`. The Next.js proxy forwards `/api/*` to
`http://127.0.0.1:8787/*` by default.

```powershell
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm dev
```

Token-protected API (server-side proxy only — never `NEXT_PUBLIC_`):

```powershell
$env:ROVE_API_TOKEN = "local-secret"
pnpm dev
```

## Advanced / Developer only

- **Benchmark**: Settings → Advanced / Developer → Benchmark runner
- **Legacy workbench scaffold**: `/dev/workbench` (bounded advanced escape hatch)

Neither is primary product navigation.

## Providers

Provider profiles are loaded, created, updated, and removed through the C0 API, and the
active profile/model selection is stored in safe API preferences. Clearing
browser storage does not remove an API-persisted profile. The browser stores
and sends **environment variable names** (`api_key_env`) only — never raw keys.
Settings → Providers can **Test** and **List models** via the API.

## Verification

```powershell
pnpm test
pnpm typecheck
pnpm build
pnpm test:e2e
```

Focused product smoke (mock API):

- empty → open workspace → run → complete
- inline approval
- refresh restore, session switching, partial/error recovery, and deep routes
- second turn through the exact product-session binding
- ambiguous job-start response reconciliation without duplicate submission
- provider save → clear browser storage → refresh restore → delete
- providers test/list models without raw keys
- theme toggle
- inspector empty → ready after run
- Advanced-only benchmark
- all nine Settings sections, revision-conflict rollback, provider update,
  approval/step job requests, catalog and Memory mutations, shortcuts, and
  mobile bounds
- M1 import/retry/malformed-state/deep-route recovery before catalog boot
- 390px and 320px reflow, reduced motion, migration-summary placement, dialog
  focus containment/restoration, and server-confirmed dark-theme reload

The `local-full` gate also runs `real-api.spec.ts` against a live local
`rove-api`. The latest C3 worktree run passed all three scenarios: live M1
migration, exact cross-session continuation/refresh/tools/cancellation/Settings,
and the bounded `/dev/workbench` smoke. The external-provider gate remains
opt-in and was not run for this evidence.

## Remaining delivery work

- Coordinator review and ordered integration of the stacked C1–C3 PR chain into
  `main`; the implementation and local verification described above are not a
  claim that the stack has already landed
- Optional external-provider gate execution when credentials and external
  service availability are intentionally in scope
- Optional transcript-rich export, bulk cleanup, and deeper Memory organization
- Native Desktop host (Tauri)
- Remote Gateway / device pairing
- File-tree IDE features, diff studio, MCP hub marketplace
