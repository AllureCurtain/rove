# rove Web Workbench

The workbench is a standalone Next.js app that talks to the rove API through
a server-side `/api/*` proxy route. It expects the Rust API server to expose the
job endpoints and SSE stream.

## Run Locally

From the repository root, the simplest path is:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1
```

That starts both `rove-api` and the workbench, then prints the Web and API URLs.

Start the API server from the repository root:

```powershell
cargo run -p rove-api
```

Start the workbench from this directory (`apps/web`):

```powershell
pnpm install --frozen-lockfile
pnpm dev
```

Open `http://localhost:3000`. By default the Next.js proxy sends `/api/*` to
`http://127.0.0.1:8787/*`.

Use **Run** to start a fresh job. Use **Resume** to create a job with
`resume: "latest"` and continue the latest resumable run; the live summary shows
the new run id and the source run id when the API returns one.

The provider selector can run against the API server's default runtime provider,
or against a per-run profile for OpenAI, Anthropic, Ollama, or fake.
For official APIs or relay/gateway APIs, choose **OpenAI**, set the
API base URL, set the server environment variable name that contains the key,
and enter the model id. Anthropic uses its native API, Ollama uses the local
`/api/chat` surface, and fake stays deterministic. The browser never sends a raw
key value; the Rust API reads the named environment variable server-side. Use
**Test** to call `/providers/test` and verify model visibility before starting
the job.

To point the workbench at another API server:

```powershell
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm dev
```

To run the workbench on a non-default port:

```powershell
$env:ROVE_WEB_PORT = "3001"
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm exec next dev --port $env:ROVE_WEB_PORT
```

To use a token-protected API, set the same token in the Rust API process and the
Next.js server process:

```powershell
$env:ROVE_API_TOKEN = "local-secret"
pnpm dev
```

The token is injected into upstream requests by the server-side proxy. It is not
read by browser code and should not use a `NEXT_PUBLIC_` environment name.

## Production Build

```powershell
pnpm build
pnpm start
```

## Verification

Before treating UI changes as ready, run:

```powershell
pnpm test
pnpm typecheck
pnpm build
```

For browser-level E2E smoke coverage:

```powershell
pnpm test:e2e
```

Playwright uses `PLAYWRIGHT_BASE_URL` when set, otherwise it uses
`ROVE_WEB_PORT`, otherwise `http://127.0.0.1:13043`. By default Playwright
starts its own Next.js server; set `PLAYWRIGHT_BASE_URL` when a script has
already started the Web server and the test should reuse it.

The Playwright tests start the Next.js dev server and mock the API at the browser
boundary. They cover create job to SSE completion, pending approval submission,
and resume-latest identity display.
