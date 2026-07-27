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
- Settings shell with deep **Providers & Models**, **About / Runtime**, and
  **Advanced / Developer** (Benchmark only here)
- Light default + dark toggle via product design tokens
- Session continue uses the server-owned exact `product_session_id` binding.
  The product shell omits client `resume`; soft transcript stitch and
  workspace-global `latest` are not product paths.

## C0/C1 product state and continuity

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

The replay-safe M1 browser migration module exists but is not yet invoked by
the product shell; its user-facing migration/recovery flow remains C3 work.

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
- **Legacy workbench scaffold**: `/dev/workbench` (escape hatch / migration only)

Neither is primary product navigation.

## Providers

Provider profiles are loaded, created, and removed through the C0 API, and the
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

## Remaining Web Complete backlog

- Full Settings section implementations beyond Providers / About / Advanced
- Existing M1 browser-state migration invocation and recovery UX
- Live-API Playwright coverage for the default product shell (current
  continuity coverage is mock-backed; `local-full` targets `/dev/workbench`)
- Rich session export / cleanup
- Memory management UI depth
- Native Desktop host (Tauri)
- Remote Gateway / device pairing
- File-tree IDE features, diff studio, MCP hub marketplace
