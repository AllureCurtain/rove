# rove Web

Primary surface: the **product shell** (Workspace → Session → Run) against live
`rove-api`. The old developer workbench is no longer the default entry.

## Product shell (default `/`)

- Empty state → open an absolute Folder/Repo path
- Workspace tree with sessions (recents + pin, local persistence)
- Chat transcript, tool cards, inline approvals, stop/cancel
- Collapsible run inspector (plan / tools / approvals)
- Settings shell with deep **Providers & Models** and **About / Runtime**
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

## Temporary workbench scaffold

The previous runtime console remains available only at:

`http://localhost:3000/dev/workbench`

It is migration scaffolding, not a second primary product entry.

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
