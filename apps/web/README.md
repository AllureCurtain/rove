# rove Web

Primary surface: the **product shell** (Workspace → Session → Run) against live
`rove-api`. The old developer workbench is not a second primary entry.

## Product shell (default `/`)

- Empty state → open an absolute Folder/Repo path
- Workspace tree with sessions (recents + pin, local persistence)
- Parallel-session **running** badges in the sidebar
- Chat transcript, tool cards, inline approvals, stop/cancel
- Collapsible run inspector with empty / loading / error / ready states
- Settings shell with deep **Providers & Models**, **About / Runtime**, and
  **Advanced / Developer** (Benchmark only here)
- Light default + dark toggle via product design tokens
- Session continue uses **hard resume only** (`resume: "latest"` under the
  opened workspace root). Soft transcript stitch is not a product path.

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

Provider profiles may be saved in browser local storage for M1. The browser
stores and sends **environment variable names** (`api_key_env`) only — never
raw keys. Settings → Providers can **Test** and **List models** via the API.

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
- second turn hard resume with workspace root
- providers test/list models without raw keys
- theme toggle
- inspector empty → ready after run
- Advanced-only benchmark

## Known M2 backlog (not M1)

- Transcript restore after refresh (catalog persists; messages still in-memory)
- Multi-session SSE follow when switching between parallel sessions
- Full Settings section implementations beyond Providers / About / Advanced
- Server-side provider profile persistence
- Rich session export / cleanup
- Memory management UI depth
- Native Desktop host (Tauri)
- Remote Gateway / device pairing
- File-tree IDE features, diff studio, MCP hub marketplace
