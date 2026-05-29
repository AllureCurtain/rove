# rove Web Workbench

The workbench is a standalone Next.js app that talks to the rove API through
a server-side `/api/*` proxy route. It expects the Rust API server to expose the
job endpoints and SSE stream.

## Run Locally

Start the API server from the repository root:

```powershell
cargo run --bin rove-api
```

Start the workbench from this directory:

```powershell
npm run dev
```

Open `http://localhost:3000`. By default the Next.js proxy sends `/api/*` to
`http://127.0.0.1:8787/*`.

Use **Run** to start a fresh job. Use **Resume** to create a job with
`resume: "latest"` and continue the latest resumable run; the live summary shows
the new run id and the source run id when the API returns one.

To point the workbench at another API server:

```powershell
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
npm run dev
```

To use a token-protected API, set the same token in the Rust API process and the
Next.js server process:

```powershell
$env:ROVE_API_TOKEN = "local-secret"
npm run dev
```

The token is injected into upstream requests by the server-side proxy. It is not
read by browser code and should not use a `NEXT_PUBLIC_` environment name.

## Production Build

```powershell
npm run build
npm run start
```

## Verification

Before treating UI changes as ready, run:

```powershell
npm test
npm run typecheck
npm run build
```

For browser-level E2E smoke coverage:

```powershell
npm run test:e2e
```

The Playwright tests start the Next.js dev server and mock the API at the browser
boundary. They cover create job to SSE completion, pending approval submission,
and resume-latest identity display.
