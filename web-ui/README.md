# rove Web Workbench

The M6 workbench is a standalone Next.js app that talks to the rove API through
the local `/api` rewrite. It expects the Rust API server to expose the M5 job
endpoints and SSE stream.

## Run Locally

Start the API server from the repository root:

```powershell
cargo run --bin rove-api
```

Start the workbench from this directory:

```powershell
npm run dev
```

Open `http://localhost:3000`. By default the Next.js rewrite sends `/api/*` to
`http://127.0.0.1:8787/*`.

To point the workbench at another API server:

```powershell
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
npm run dev
```

## Production Build

```powershell
npm run build
npm run start
```

## Verification

Before treating M6 UI changes as ready, run:

```powershell
npm test
npm run typecheck
npm run build
```

For end-to-end smoke testing:

1. Start `rove-api`.
2. Start the workbench.
3. Submit a task with model `fake`.
4. Confirm the conversation, plan, tool rail, trace rail, cancel action,
   approval prompts, and pending input prompts update without a page refresh.
