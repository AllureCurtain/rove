# rove Implementation Guide

This guide is for maintainers who need to understand, debug, or extend the current implementation. It describes what exists in the codebase today. Product intent and historical design rationale live in the top-level docs; the current runtime source of truth remains this `docs/runtime/` directory.

> Web status note (2026-07-26): Web Complete C0 is implemented. The default Web
> shell has not yet been wired to the C0 client modules; C1–C3 remain future
> work.

The root manifest is a modular resolver-3 Cargo Workspace whose default
member is `apps/cli`, with independent packages `rove-models`, `rove-core`,
`rove-runtime`, `rove-app-bootstrap`, `rove-cli`, `rove-api`, `rove-bench`, and
`rove-integration-tests`. Use Workspace-wide commands for full gates.

## 1. Runtime Shape

`rove` is a local-first agent runtime with three user-facing shells. The CLI
offers REPL, exec, and optional full-screen TUI modes:

```text
CLI (REPL / exec / TUI) / API / Web
    -> apps/bootstrap build_engine / tool_registry
        -> ContextManager
        -> rove-runtime Engine / identity / task / execution / workspace contracts
        -> rove-core model turn / ToolRegistry
            -> rove-models ModelClient / RoutingModelClient
        -> runtime Executor / approval / input
        -> Memory loaders and hooks
        -> StateStore

External embedding
    -> rove-core::Agent
        -> rove-models::ModelClient
        -> custom ToolRegistry / ToolPolicy
        -> in-memory AgentEvent

StateStore
    -> .rove/runs/<run_id>/*
    -> .rove/state.sqlite
```

Product shells use `runtime::Engine` via `build_engine`. `core::Agent` is
embed-only. The interface layers construct the runtime and consume `StreamEvent`
values. Core code does not depend on CLI, TUI, API, or Web modules.

Important entry points:

| Area | Files |
|---|---|
| CLI binary | `apps/cli/src/main.rs`, `apps/cli/src/cli/*` |
| Full-screen TUI mode | `apps/cli/src/tui/*`, `apps/cli/src/terminal/*` |
| API binary | `apps/api` |
| Web product shell | `apps/web/` |
| In-memory Agent and tool contracts | `core/src/*` |
| Persistent runtime services | `runtime/src/*` |
| Persistent Engine and coordination | `runtime/src/engine/*`, `runtime/src/planning/*` |
| State artifacts and SQLite index | `runtime/src/state/*` |
| Model protocol and providers | `models/src/*` |
| Product provider assembly | `apps/bootstrap/src/factory.rs` |
| Local built-in tools and invocation adapters | `runtime/src/tools/*` |
| Product registry assembly | `apps/bootstrap` |
| MCP transport/proxy | `runtime/src/tools/mcp_proxy.rs` |
| Memory/context/compaction services | `runtime/src/memory/*`, `runtime/src/context/manager.rs`, `runtime/src/context/compaction.rs` |
| Tool executor and hooks | `runtime/src/tools/executor.rs`, `runtime/src/tools/hooks/` |

## 2. Workspaces

`Workspace::detect` is the first runtime boundary. It canonicalizes the starting directory, walks upward for `.git`, and returns either:

- `WorkspaceKind::Repo` with the nearest git root as `workspace.root`;
- `WorkspaceKind::Folder` with the starting directory as `workspace.root`.

The default state directory is `workspace.root/.rove`. Config can override
`state.state_dir`, `state.sqlite_path`, `memory.session_dir`, and
`memory.durable_dir`. Core state and layered memory use the resolved config
paths; there is no built-in vector-RAG artifact path.

`WorkspaceKind::Task` is an explicit standalone workspace. It is created under
a task base directory and does not require the shell or API server to start
inside an existing project. The task name must be a single path component; path
traversal and absolute names are rejected. After creation, config is rebased to
the task root so defaults resolve to:

```text
<task-base>/<task-name>/
  .rove/
    state.sqlite
    runs/
    memory/
```

CLI runs create or reuse a task workspace with:

```powershell
cargo run -p rove-cli -- --task-workspace invoice-check --task-base .rove/tasks --model fake "review the files in this task"
```

If `--task-base` is omitted, the CLI uses `<configured state_dir>/tasks` from
the initially detected config context.

API jobs can request a task workspace per job:

```json
{
  "message": "review the uploaded task files",
  "model": "fake",
  "workspace": {
    "kind": "task",
    "name": "invoice-check",
    "base": "D:/rove-tasks"
  }
}
```

If `base` is omitted, the API uses `<configured state_dir>/tasks` from the
server workspace config. Each task workspace gets its own resolved state store,
run artifacts, filesystem tool boundary, shell working directory, session
memory, and durable memory paths.

API jobs can also bind an explicit absolute Folder or Repo root (product
workspace open). The path becomes the real tool/state execution root for that
job; the API process cwd is not used as a silent fallback:

```json
{
  "message": "summarize this folder",
  "model": "fake",
  "workspace": {
    "kind": "folder",
    "root": "D:/projects/notes"
  }
}
```

```json
{
  "message": "inspect this repo",
  "model": "fake",
  "workspace": {
    "kind": "repo",
    "root": "D:/projects/my-repo"
  }
}
```

Rules for `folder` / `repo`:

- `root` is required and must be an absolute existing directory.
- `folder` pins the provided path even if a parent `.git` exists (no walk-up).
- `repo` requires a `.git` entry at `root` itself.
- `name` / `base` are task-only; mixing them with `folder`/`repo` is rejected.
- Resume (`resume: "latest"` or a `run_id`) is store-scoped to the requested
  workspace (explicit `folder`/`repo`/`task` root, otherwise the API process
  workspace). Hard resume is fail-closed: if the resume key does not resolve
  durable `task_state` in that store, create-job returns **400**
  (`nothing to resume in this workspace`) instead of opening a silent one-shot
  session. Clients must re-send the same workspace binding on continue turns.

Task workspace lifecycle:

1. Create or reuse the named workspace through CLI `--task-workspace` or API
   `workspace.kind = "task"`.
2. Put task inputs under the task root or let tools create files there.
3. Resume, inspect, repair, or clean state from the same task workspace context.
4. When the task is no longer needed, delete the task workspace directory. This
   removes the task files, `.rove` state, run artifacts, and default memory for
   that isolated task.

Browser and Desktop workspaces are documented future designs only:
`docs/runtime/browser-workspace-spec.md` and
`docs/runtime/desktop-workspace-spec.md`. The runtime intentionally has no
`Browser` or `Desktop` workspace enum stubs yet.

Relevant code:

- `runtime/src/workspace/root.rs`
- `runtime/src/workspace/boundary.rs`
- `apps/bootstrap/src/config.rs`

## 3. Configuration

`AppConfig::load` merges configuration in this order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

The config is grouped by runtime, provider, tool, memory, state, API, web, and
routing. Provider configuration supports named profiles with explicit wire
protocol IDs, active/fallback references, environment or bounded-file secret
references, custom headers, provider options, and protocol options. Legacy
flat provider fields remain compatible. `dump-config` prints the effective
config, source summary, resolved paths, legacy secret-presence flags, and
profile secret/header source summaries without resolved values.

Common paths and defaults:

| Config | Default |
|---|---|
| `runtime.system_prompt_path` | `prompts/system.md` |
| `runtime.planner_prompt_path` | `prompts/planner.md` |
| `runtime.model_compaction_enabled` | `false` |
| `runtime.compaction_failure_threshold` | `3` |
| `state.state_dir` | `.rove` |
| `state.sqlite_path` | `.rove/state.sqlite` |
| `tool.mcp_config_path` | `.rove/mcp_servers.json` |
| `tool.shell.timeout_ms` | `30000` |
| `tool.shell.max_output_bytes` | `65536` |
| `tool.shell.inherit_environment` | `true` |
| `tool.shell.denylist` | `[]` |
| `memory.session_dir` | `.rove/memory/sessions` |
| `memory.durable_dir` | `.rove/memory` |
| `routing.failure_threshold` | `3` |
| `routing.open_cooldown_ms` | `30000` |
| `routing.retry_max_attempts` | `1` |
| `routing.retry_backoff_base_ms` | `250` |
| `routing.retry_backoff_max_ms` | `5000` |
| `api.bind_addr` | `127.0.0.1:8787` |

Remote API binding is rejected unless token auth is configured or `api.unsafe_remote_without_auth = true` is set.

Useful commands:

```powershell
cargo run -p rove-cli -- dump-config
```

Relevant code:

- `apps/bootstrap/src/config.rs`
- `apps/bootstrap/src/provider.rs`
- `apps/cli/src/cli/config.rs`

## 4. CLI Startup Path

The CLI binary starts in a synchronous `main()` and only creates a Tokio runtime
for commands that need async work. True sync fast paths are handled before
runtime construction.

High-level flow in `src/main.rs`:

1. Parse `Args` from `apps/cli/src/cli/args.rs`.
2. If `Args::is_sync_fast_path()` is true, run it without creating a Tokio runtime:
   - `dump-config`
3. Create a Tokio runtime for async commands and normal runs.
4. Run async maintenance subcommands:
   - `index`
   - `sessions`
   - `state repair`
   - `state cleanup`
5. Detect the starting Folder/Repo workspace.
6. Load `AppConfig`.
7. If `--task-workspace` is set, create or reuse that Task workspace and rebase
   config paths to the task root.
8. Construct the model client.
9. Register the shared runtime tool registry, including configured MCP tools.
10. Build `ContextManager`.
11. Build `Engine`.
12. Create `StateStore`.
13. Resolve optional CLI resume state when an exec or TUI run starts.
14. If `tui` is present, split a bounded interaction broker into providers for
    the shared Engine and one receiver for the alternate-screen application
    loop.
15. If `exec <message>` is present, run the non-interactive exec backend.
16. If a bare message argument is present, enter the rich terminal REPL and
    submit that message as the first prompt.
17. If no message and no subcommand are present, enter the rich terminal REPL
    and wait for input.

Interactive REPL smoke command:

```powershell
cargo run -p rove-cli -- --model fake
```

Interactive REPL with an initial prompt:

```powershell
cargo run -p rove-cli -- --model fake "echo hello from rove"
```

Non-interactive exec smoke command:

```powershell
cargo run -p rove-cli -- exec --model fake "echo hello from rove"
```

The CLI accepts unquoted multi-word initial prompts and exec prompts by joining
the trailing message words:

```powershell
cargo run -p rove-cli -- --model fake inspect this workspace
cargo run -p rove-cli -- exec --model fake inspect this workspace
```

`Cargo.toml` sets `default-run = "rove"`, so plain `cargo run -p rove-cli -- ...` uses the CLI binary.

Running `rove` with no task enters the rich scrollback terminal REPL in the
current terminal. Startup prints the active workspace, model, provider, state
directory, session status, and common commands:

```text
rove
local-first agent runtime
workspace  repo  <workspace-root>
model      <model-id>
provider   <provider>
state      .rove  ·  session new

/help  /sessions  /resume latest  /status  /clear  /exit
rove>
```

The REPL remains a normal terminal prompt, not a full-screen TUI. During runs it
prints compact `You`, `Plan`, `Tool`, `Error`, and `Done` sections. The default
Web product shell provides the richer product interaction surface, while the
advanced `/dev/workbench` retains direct run history/report inspection.

The compact REPL is backed by a terminal view/action layer. `StreamEvent` values
are first projected into terminal view updates and accumulated into view state;
the current REPL renders those updates as line-oriented output. The optional TUI
uses the same projection and run-artifact path without adding a second engine
loop or persistence format.

### Full-screen TUI

The current full-screen TUI is available with:

```powershell
cargo run -p rove-cli -- tui --model fake
```

`rove tui` enters raw mode and the alternate screen through the RAII
`TerminalSession`, then runs a fair asynchronous loop over Crossterm input,
canonical run updates, process-local interaction requests, cancellation
signals, and redraw ticks. A prompt follows this path:

```text
key event -> TuiAction -> reducer -> shared Engine
Engine StreamEvent -> RunViewState -> bounded update channel -> Ratatui
approval/input provider -> bounded request channel -> matching modal -> oneshot response
```

Approval and input become actionable only after two independently delivered
halves agree on both interaction kind and `CallId`: the canonical
`RunViewUpdate` supplies display state, while the process-local request owns the
live responder. The responder stays outside cloneable `TuiState`. A second
request cannot replace it, and cancellation, completion, exit, terminal EOF,
draw failure, and restoration failure all drop it fail closed. Queued requests
are discarded between runs so stale capabilities cannot cross a run boundary.

Terminal setup enables bracketed paste and requests enhanced keyboard event
types where the terminal supports them. Non-Windows terminals with that
capability use direct `Y` approval and `Enter` input submission. Windows uses
native key events but does not expose a trustworthy paste-vs-key distinction,
so approval requires `Y` to select and a fresh non-text `F8` to confirm, while
input uses `F8` to submit. Without a usable key-event mode the basic TUI
remains available, but an approval request resolves as Reject and an input
request returns a typed unavailable error without opening a modal. A matched
modal is initially unarmed: the event loop drains already-ready input, tracks
keys held before the modal, draws the modal successfully, and waits one
additional frame with no keys held before accepting a response. This prevents a
queued composer `y`, held-key repeat, old Enter, or pasted text from resolving
an interaction that was not yet visible.

The run driver uses an awaited bounded sink. It continues polling the Engine
after `RunCompleted` so post-run hooks execute, finalizes the shared trace/task
state/report/index artifacts, and only then publishes the completion update to
the screen. Ctrl+C cancels the existing run token and waits for the canonical
cancelled completion. Ctrl+Q while a run is active requires a second
confirmation, then cancels and waits before leaving the terminal.

The TUI currently supports prompt editing (bounded to 32 KiB), focus switching,
wrapped transcript scrolling with bottom anchoring, archived runs in the current
session, plan/tool/activity rendering, resize and narrow-terminal layouts, and
startup `--resume`. `Ctrl+R` opens a bounded session picker (newest-first,
non-running task states only); selecting a candidate performs a second identity
and liveness check before loading resume state. Stale, malformed, wrong-ID, or
busy candidates fail closed. Submitting the next prompt atomically claims the
indexed terminal job for the expected run ID; a concurrent or stale claim is
rejected before the engine starts and the draft remains available. `Ctrl+T`
opens a bounded tool-detail overlay for
completed or failed calls, and `F1` opens help generated from the actual keymap.
Tool detail and session goals are display-sanitized and size-bounded; this is a
defensive presentation filter, not a formal secret detector or a permission
boundary.

The live transcript consumes `RunViewState::timeline_entries()`, a
renderer-neutral in-memory ledger capped at 512 entries with bounded text.
Entries retain canonical delivery order for visible user, assistant, model-status,
plan, tool, approval/input, compaction, memory, and completion facts. Duplicate
lifecycle notifications are deduplicated without changing the high-watermark;
`trace.jsonl`, `task_state.json`, `report.json`, and SQLite remain the durable
sources of truth. Hidden reasoning is filtered across streamed chunk boundaries,
and raw tool payloads, memory notes, and provider error text are not copied into
the visible timeline. An empty legacy `RunViewState` timeline falls back to a
sanitized aggregate rendering path for compatibility; newly delivered live
updates use the ordered timeline. The resulting filtering is conservative and
heuristic; callers must continue to bound and sanitize every display field.

On direct-capability terminals the TUI also supports destructive-tool approval
and `request_input`: approval accepts `Y` only from a real key press and rejects
on a real `N` or `Esc` press; Enter, repeat, release, and paste cannot authorize.
On Windows, `Y` only stages approval and a fresh `F8` press confirms it; input is
submitted with `F8` rather than Enter. Direct input accepts typed and
bracketed-paste UTF-8 up to 32 KiB and submits the exact draft, including an
empty or whitespace-only answer. Existing REPL and `rove exec` behavior is
unchanged. Tracing is routed to a sink while the alternate screen is active so
runtime logs cannot corrupt the display.

An opt-in PTY smoke harness covers the Unix standard-library PTY path, including
nonblank frames, bounded resize/redraw, clean exit, termios restoration, and
alternate-screen/bracketed-paste/cursor restore sequences:

```powershell
python scripts/tui-pty-smoke.py --run
```

The harness emits a typed `skipped` result with exit code `77` on Windows because
this repository does not yet include a native ConPTY runner. That skip is not
cross-platform interoperability evidence. Mouse interaction, multi-session
tabs, background tasks, and a native Windows real-terminal automation gate
remain future scope. The renderer explicitly excludes known hidden-reasoning
formats and keeps defense-in-depth sanitization at its display boundary.

### REPL Commands

Supported slash commands are:

| Command | Purpose |
|---|---|
| `/help` | Print REPL commands. |
| `/status` | Print workspace, model, provider, state directory, session id, active run/job, and session memory path. |
| `/exit`, `/quit` | Exit the REPL. |
| `/clear` | Clear the terminal screen. |
| `/sessions` | List resumable task states from the active workspace. |
| `/resume latest` | Load the newest task snapshot as the active resume state. |
| `/resume <run_id>` | Load a specific task snapshot as the active resume state. |

Normal text input runs a new engine run. After a successful non-cancelled run,
the REPL loads the latest task snapshot for follow-up turns, preserving the
session id and reusing the previous job id while creating a new run id. Ctrl+C
while the prompt is idle returns to the prompt; Ctrl+C while a run is active
cancels that run and keeps the REPL process alive.

Relevant code:

- `src/main.rs`
- `apps/cli/src/cli/args.rs`
- `apps/cli/src/cli/oneshot.rs`
- `apps/cli/src/cli/repl.rs`
- `apps/cli/src/cli/sessions.rs`
- `apps/cli/src/cli/state.rs`
- `apps/cli/src/cli/index.rs`
- `apps/cli/src/tui/action.rs`
- `apps/cli/src/tui/app.rs`
- `apps/cli/src/tui/effect.rs`
- `apps/cli/src/tui/keymap.rs`
- `apps/cli/src/tui/providers.rs`
- `apps/cli/src/tui/state.rs`
- `apps/cli/src/tui/reducer.rs`
- `apps/cli/src/tui/render.rs`
- `apps/cli/src/tui/sanitize.rs`
- `apps/cli/src/tui/terminal.rs`
- `apps/cli/src/tui/widgets/*`
- `apps/cli/src/terminal/interaction.rs`
- `apps/cli/src/terminal/run.rs`
- `apps/cli/src/terminal/view.rs`
- `runtime/src/state/index.rs`
- `runtime/src/state/resume.rs`
- `runtime/src/state/store.rs`
- `scripts/tui-pty-smoke.py`

## 5. API Startup Path

The API binary is thin. `src/bin/rove-api.rs` parses an optional bind address and working directory, then calls `serve`.

`serve_with_shutdown`:

1. Detects the workspace.
2. Loads config with API bind override.
3. Applies configured `state_dir`.
4. Creates `ApiState`.
5. Initializes the SQLite index.
6. Marks stale `init` or `running` jobs as `interrupted`.
7. Opens the API-global `<state_dir>/product.sqlite`, applies its schema, and
   conservatively recovers stale product-turn claims.
8. Binds the TCP listener and serves the router.

Routes:

| Route | Purpose |
|---|---|
| `POST /jobs` | Create and start a job |
| `GET /jobs/{job_id}/state` | Read live or persisted job state |
| `GET /jobs/{job_id}/events` | Stream SSE events, with replay support |
| `POST /jobs/{job_id}/cancel` | Cancel a live job |
| `POST /jobs/{job_id}/approvals/{call_id}` | Resolve a pending destructive tool approval |
| `POST /jobs/{job_id}/inputs/{input_id}` | Resolve a pending `request_input` prompt |
| `GET /runs` | List recent persisted runs |
| `GET /runs/{run_id}/report` | Read the indexed `report.json` artifact for a run |
| `POST /providers/models` | List models for a validated per-request provider profile |
| `POST /providers/test` | Validate provider connectivity/model presence without returning secrets |
| `GET/POST /product/workspaces`, `DELETE /product/workspaces/{workspace_id}` | List, create, or remove product workspace catalog entries without deleting workspace files |
| `GET/POST /product/sessions`, `PATCH/DELETE /product/sessions/{session_id}` | List, create, rename/archive, or remove server-owned product sessions |
| `GET /product/sessions/{session_id}/transcript` | Project ordered canonical run events with complete/partial status and typed reasons |
| `GET/POST /product/provider-profiles`, `PUT/DELETE /product/provider-profiles/{profile_id}` | Manage secret-reference-only provider profiles |
| `GET/PUT /product/preferences` | Read or update the bounded safe product preference set |
| `POST /product/migrations/m1-browser` | Validate and atomically apply or replay an idempotent M1 browser import |

API jobs have two state layers:

- live handles in memory: task handle, cancellation token, broadcast sender, approval/input channels;
- durable state in SQLite and `.rove/runs/<run_id>/`.

`POST /jobs` accepts `message`, optional `model`, `max_steps`, `approval`,
optional provider profile, optional `resume`, optional `workspace`, and optional
`product_session_id`.
Per-job workspaces support Task (`name`/optional `base`) and explicit Folder or
Repo (`root`) shapes described in §2. Folder/Repo roots become the real
execution, state, tool, shell, and memory boundary; they do not fall back to the
API process workspace.
`resume` follows the CLI semantics: omit it for a fresh session/job, use `"latest"` for the newest task snapshot, or pass a run id to load that exact snapshot. A resumed API job keeps the loaded `session_id` and `job_id`, creates a new `run_id`, and passes the loaded `TaskState` into `RunRequest` and artifact recording.

When `product_session_id` is present, the server owns continuity. It validates
the product session's workspace, rejects a simultaneous client `resume`, claims
the session's single active turn, and starts fresh only when no runtime binding
exists. Later turns load the exact bound `latest_run_id`; missing, corrupt, or
mismatched state fails closed instead of falling back to workspace-global
`latest`. A successful launch records an immutable ordered run binding, and the
supervisor persists the terminal product-session status before releasing the
claim. Different product sessions may run concurrently.

After restart, historical job state and SSE events can be read from SQLite.
Pending approvals and pending inputs follow Policy A: requests are persisted
for audit while live, but their channels are not reconstructed after process
restart. API startup marks stale running jobs and pending approval/input rows as
`interrupted`; `/jobs/{job_id}/state` then shows no answerable pending lists for
that historical job. Resuming latest starts a new run with a new `run_id`. If
the snapshot contains a planned-step attempt with no terminal record, the new
run emits an `interrupted` `StepRecord`, produces no new model/tool start for
that attempt, and terminates with `RunStatus::Error` rather than risking a
duplicate external side effect.

Historical run discovery is read-only. `/runs` returns recent run identity, status, last indexed event sequence, and report availability from SQLite. `/runs/{run_id}/report` uses the indexed report row to load `report.json`; it does not expose arbitrary filesystem paths.

Product state is separate from runtime run state. ProductStore owns safe
catalog/settings/mapping data in API-global `product.sqlite`; canonical events,
task snapshots, and reports remain in the selected execution workspace. The
transcript endpoint walks the product session's ordered run bindings and reads
canonical indexed events from those workspace stores. Missing or inconsistent
facts produce typed partial reasons rather than a silently complete response.

The M1 migration route uses strict unknown-field rejection, bounded bodies,
server-side workspace/runtime validation, idempotency receipts, and one SQLite
transaction for the accepted import. A 30-second deadline covers only
pre-commit preparation. Accepted apply work runs under an API-owned supervisor,
so HTTP disconnect does not cancel a commit. The ProductStore persists the
first preflight preference revision baseline per idempotency key, reuses it on
retry, applies preferences with revision CAS, and atomically replaces the
preparation with a success receipt. A source-mapped product session with an
active turn returns typed `product_session_active`; the Web retains the exact
pending key and body for retry.

Before a verified runtime binding is committed, all runtime SQLite paths are
canonicalized, sorted, and reserved with `BEGIN IMMEDIATE`. External commit
guards require canonical runtime database and artifact paths to remain inside
the canonical workspace when external paths are disabled, and open SQLite with
`SQLITE_OPEN_NOFOLLOW`; symlinked parent paths fail closed. Read-only runtime
verification uses the same workspace boundary before opening a database.

Relevant code:

- `src/bin/rove-api.rs`
- `apps/api/src/lib.rs`
- `apps/api/src/security.rs`

## 6. Web Product Shell Path

`apps/web/` is a standalone Next.js app. Browser code talks to relative `/api/*`
URLs. A Next.js app route proxies those requests to the Rust API:

```text
/api/* -> ROVE_API_BASE or http://127.0.0.1:8787
```

If `ROVE_API_TOKEN` is set in the Next.js server environment, the proxy injects
`Authorization: Bearer <token>` into upstream requests. The token is not exposed
through `NEXT_PUBLIC_*` variables or browser JavaScript. SSE job streams are
proxied by returning the upstream response body directly, so `EventSource` keeps
using `/api/jobs/{job_id}/events` without custom browser headers.

The default `/` route is the M1 product shell:

```text
Workspace/session rail | Chat transcript/composer | collapsible Run Inspector
```

Settings is a separate full-page shell. Providers/About are implemented deeply,
General owns theme, Advanced owns Benchmark, and several other sections remain
explicit placeholders. `/dev/workbench` retains the old developer surface as an
advanced escape hatch only.

The current M1 product shell:

1. Opens an absolute Folder or Repo path and stores the M1 catalog in browser
   storage.
2. Creates a product session and sends the first turn with `POST /api/jobs`
   bound to that real workspace root.
3. Opens `EventSource` on `/api/jobs/{job_id}/events`, applies canonical events
   through the shared reducer/controller, and renders chat/tool/approval/input
   projections.
4. Calls approval, input, and cancel endpoints when required; fetches job state
   on stream errors to resync.
5. Continues a durable M1 session using workspace-scoped `resume: "latest"` and
   surfaces hard failures rather than silently opening a disconnected run.
6. Sends only provider type/base/model/key-environment references; raw provider
   keys never enter browser requests.

Web Complete C0 adds `apps/web/product/` with strict response
validation, a thin client for all product CRUD/transcript/migration routes, and
a same-origin-locking, replay-safe M1 migration state machine. It also adds
`product_session_id` to the shared create-job request type. These modules are
not yet invoked by `ProductApp`.

Current UI limits are therefore still product-significant: transcript messages
disappear on refresh; the visible workspace/session catalog and provider
Settings remain browser-authoritative; routes are effectively single-page;
background session SSE reattachment is incomplete; and several Settings
sections are placeholders. C1 must switch turns and restore to the C0 exact
product-session/transcript path. C2 must move the corresponding Settings
authority to the C0 APIs. C3 owns final migration UX and acceptance evidence.

Relevant code:

- `apps/web/shell/ProductApp.tsx`
- `apps/web/api/run-controller.ts`
- `apps/web/state/*`
- `apps/web/chat/*`
- `apps/web/inspector/*`
- `apps/web/settings/*`
- `apps/web/app/api/[...path]/route.ts`
- `apps/web/lib/rove-api-proxy.ts`
- `apps/web/lib/rove-client.ts`
- `apps/web/lib/rove-state.ts`
- `apps/web/lib/rove-types.ts`

Current Web checks:

```powershell
cd apps/web
pnpm test
pnpm typecheck
pnpm build
```

Browser-level Playwright tests live under `apps/web/tests/e2e` and run through
`pnpm test:e2e`. They are separate from the default fast Web checks so the
unit/type/build loop stays lightweight. `shell.spec.ts` covers the default `/`
product shell with browser-boundary mocks. `workbench.spec.ts` does the same for
the advanced surface, and the gated `real-api.spec.ts` opens `/dev/workbench`
against a live Rust API. Those three real-API cases do not constitute
live-API acceptance of the default product shell; that remains Web Complete C3
work.

Real provider smoke tests are opt-in and documented in `docs/runtime/provider-smoke.md`. They are intentionally excluded from default CI because they require credentials, network access, local Ollama availability, or provider quota.

## 7. Core Runtime Types

The core type model is centered on explicit IDs and serializable runtime state:

| Type | Purpose |
|---|---|
| `SessionId` | User-level continuity across jobs |
| `JobId` | One submitted task |
| `RunId` | One engine execution |
| `CallId` | One tool call; owned by `rove-core` |
| `RunRequest` | Identity + user message + optional resume state |
| `TaskState` | Serializable resume snapshot |
| `PromptCheckpoint` | Compact reconstruction point for resume |
| `TaskPlan` | Planner output and current step pointer |
| `PlanIdentity` | Stable logical plan and compatibility revision identity |
| `PlanRevision` | Immutable initial or replacement snapshot of remaining work |
| `PlanDecisionRecord` | Correlation from one terminal step fact to its rule-first decision |
| `PlanLifecycleState` | Materialized revision and decision projections |
| `StepAttempt` | Persisted identity for one in-flight planned attempt |
| `StepRecord` | Append-only terminal fact for one planned attempt |
| `StepLedgerState` | Materialized ledger and active-attempt projection |
| `Message` | Provider-facing conversation message; owned by `rove-models` |
| `rove_models::ModelToolSchema` | Model-visible name, description, and input schema |
| `rove_core::ToolDescriptor` | Operational schema plus destructive/parallel/capability metadata |
| `RunStatus` | API/job status |
| `TerminationReason` | Engine completion reason |

Relevant code:

- `models/src/protocol.rs`
- `core/src/types.rs`
- `runtime/src/foundation/types.rs`
- `runtime/src/planning/execution.rs`

## 8. Stream Events

`Engine::run_with_cancel` returns a stream of `StreamEvent` values. Consumers should treat the event stream as the public runtime protocol.

Current event variants:

- `run_started`
- `llm_chunk`
- `model_status`
- `llm_message`
- `tool_call_started`
- `tool_call_approval_needed`
- `tool_call_completed`
- `tool_call_failed`
- `input_needed`
- `plan_created`
- `plan_step_started`
- `step_result`
- `plan_decision`
- `plan_revised`
- `prompt_compacted`
- `memory_flushed`
- `prompt_built`
- `run_completed`

The API serializes these events as SSE using `StreamEvent::event_name()`. The
trace writer serializes the same events to `trace.jsonl` and indexes them in
SQLite with sequence numbers. `PlanCreated` and `PlanStepStarted` carry stable
plan/revision/attempt identity. `step_result` is the canonical terminal fact,
`plan_decision` records the deterministic transition selected for it, and
`plan_revised` carries an immutable child revision when remaining work is
replaced. The older completed/failed events remain compatibility notifications.
For a live job, SSE withholds a terminal event that is durable in SQLite until
the live finalization barrier publishes it; replay and `Last-Event-ID` handoff
then emit it at most once and close even when the client already acknowledged
its sequence.

`model_status` is the safe progress surface for model-side work. It can say
that the model is thinking, has selected a tool, or that the run is waiting for
approval. It must not expose raw provider `ThinkingDelta` or hidden reasoning
text.

Adding a new event requires checking:

- CLI rendering in `apps/cli/src/cli/oneshot.rs`
- API SSE/event persistence in `apps/api/src/lib.rs` and `runtime/src/state/index.rs`
- Web types and reducer in `apps/web/lib/rove-types.ts` and `apps/web/lib/rove-state.ts`
- artifact recording in `runtime/src/state/artifacts.rs` if it affects resume/report state

Relevant code:

- `runtime/src/foundation/events.rs`
- `runtime/src/state/trace.rs`

## 9. Engine Execution Flow

`Engine` is the persistent orchestration facade and now lives in
`rove-runtime`. It owns the model client, tool registry, context manager,
workspace, approval policy, hooks, resolved memory paths, planner prompt, and
optional interface providers for approval/input. IDs, task/execution data,
Workspace/path safety, runtime identity, approval/input contracts,
context/compaction, memory, events, state services, the tool `Executor`
pipeline, hooks, runtime-specific tool turns, planning/run coordination, and
durable event translation live in `rove-runtime`; the normalized model turn and
action parser live in `rove-core`. Product registry assembly and first-party
`AppConfig` live in product bootstrap and app shells.

The high-level run flow:

1. Emit `RunStarted`.
2. Build history from resume checkpoint or full resume state.
3. Load durable/session memory into working prompt memory.
4. If planning is enabled:
   - draft a new `TaskPlan` with the configured planner prompt, or resume a saved plan;
   - emit `PlanCreated` with an immutable revision for an initial plan, or wrap
     a legacy persisted mutable plan once as revision zero;
   - loop over plan steps;
   - run each step through `step_runner.rs` with a four-model-turn compatibility
     ceiling;
   - build step-specific context while preserving current-step tool history;
   - call the model and execute tools through the shared turn helpers;
   - append tool results and return to the model in the same step;
   - complete the step only on a model step conclusion;
   - collect model-turn, tool-call, mutation, and token metrics from emitted
     events;
   - emit a terminal `step_result`, evaluate it deterministically, and emit one
     correlated `plan_decision`;
   - continue, finish with a typed reason, or emit `plan_revised` after an
     explicitly recoverable terminal failure;
   - repair malformed/recoverable tool output within the step before creating
     a terminal failure.
5. If planning is disabled:
   - run the simpler ReAct loop over the original user message.
6. Emit `RunCompleted`.
7. Run post-run hooks before the stream closes.

Termination can happen because of:

- final answer;
- step-attempt limit or planned step model-turn limit;
- token hard limit;
- model error;
- planner error;
- cancellation.

The planned and unplanned paths share model-turn and tool-turn helpers. If you are changing model streaming, native tool-use conversion, approval, batch execution, or history mutation, start in `model_turn.rs` or `tool_turn.rs`. `step_runner.rs` owns bounded within-step iteration, scoped history, and event-derived attempt metrics; `plan_evaluator.rs` owns replay-safe rule-first decisions; `plan_loop.rs` owns plan/revision identity, attempt closure, the append-only terminal record, decision ordering, plan cursor, and replacement revisions. Current planned execution emits only the canonical lifecycle events; compatibility plan-step dual-fire was removed.

Plan mutation semantics:

- Step IDs are stable within one `TaskPlan`.
- Initial planning emits `PlanCreated` with revision zero. Replanning replaces
  only the active remaining `TaskPlan` and emits `PlanRevised` with the same
  logical `plan_id`, a new `revision_id`, an incremented revision, parent and
  trigger correlations, retained/superseded step IDs, and a budget snapshot.
- Each terminal `StepRecord` is followed by one rule-first decision. Only an
  explicitly recoverable failure can select `replace_remaining`; approval
  denial and other blocked outcomes finish without asking the planner to find
  a way around the boundary.
- Completed and failed attempt records remain append-only across replacement
  plans; replanning does not overwrite their evidence, tool IDs, or mutations.
- Resume prefers the checkpoint plan when present, then the task-state plan. A
  terminal successful record advances a stale materialized cursor without
  replay, and a terminal record missing its decision is evaluated exactly once.
  A complete active attempt without a terminal record becomes `interrupted`
  and the resumed run stops with an error. Resume does not yet scan trace
  events newer than the task-state projection.

Relevant code:

- `runtime/src/engine/facade.rs`
- `core/src/agent.rs`
- `core/src/model_turn.rs`
- `core/src/parser.rs`
- `runtime/src/engine/model_turn.rs` (durable event translation)
- `runtime/src/engine/tool_turn.rs`
- `runtime/src/engine/run_loop.rs`
- `runtime/src/engine/step_runner.rs`
- `runtime/src/engine/plan_loop.rs`
- `runtime/src/planning/plan_evaluator.rs`
- `runtime/src/planning/planner.rs`
- `runtime/src/context/manager.rs`
- `runtime/src/context/compaction.rs`

## 10. Context And Compaction

`ContextManager` builds provider messages from:

```text
system -> durable memory -> session memory -> compact summary -> recent history tail -> current user message
```

There are two modes:

- message-count history limit;
- token-budget history limit.

Both modes select provider-native assistant tool calls and all matching tool results as one atomic history unit. Incomplete or orphan native rounds are excluded rather than replayed as invalid provider protocol. Token estimates are approximate: four characters per token plus message/tool-call overhead. The context builder reports whether it crossed soft/hard budgets and whether automatic compaction is needed.

Checkpoint compaction has two paths:

- default deterministic compaction, used when model compaction is disabled or as fallback;
- optional model-generated compaction when `runtime.model_compaction_enabled = true`.

When automatic compaction is needed and old history has been dropped from the active prompt, the engine first flushes durable-worthy notes from the soon-to-be-compacted messages into session memory and emits `memory_flushed` when notes were written. It then attempts a structured model summary behind prompt version `rove.compaction.v3`. The dropped segment is serialized as JSON inside one ordinary user data message, so assistant/tool protocol roles are not replayed and embedded text is treated as untrusted historical data. A successful model summary emits `prompt_compacted` and records `mode = "model_generated"` with model and source-message metadata. If summary generation fails, the run continues with a deterministic structured fallback summary, degraded/circuit metadata, and the last error. After `runtime.compaction_failure_threshold` consecutive failures, model compaction is circuit-opened for that runtime and deterministic behavior remains available through normal checkpointing.

`RunArtifactRecorder` writes `PromptCheckpoint` with:

- optional summary;
- preserved tail messages;
- current plan;
- memory pointers;
- last step;
- last event sequence matching the SQLite event high-water mark;
- token estimate;
- compacted message count;
- compaction metadata, including mode, degraded state, model, prompt version,
  source message count, and last error when present;
- bounded lifecycle metadata: active plan/revision identity, terminal record,
  revision, and decision counts, plus an optional active attempt. Full records,
  decisions, and revisions remain in `TaskState` and `trace.jsonl`.

Resume prefers checkpoint tail/summary over replaying the full saved history.
During a planned step, current-step assistant/tool messages are injected as a
bounded prefix so the next model turn cannot lose the just-produced tool result
to global history trimming. They are merged back into ordinary history when the
step reaches a terminal outcome.

Relevant code:

- `runtime/src/context/manager.rs`
- `runtime/src/context/compaction.rs`
- `runtime/src/state/artifacts.rs`

## 11. Model Layer

All providers implement:

```rust
trait ModelClient {
    fn stream(&self, messages: &[Message], tools: &[rove_models::ModelToolSchema])
        -> BoxStream<'_, Result<ModelEvent, ModelError>>;

    fn model_id(&self) -> &str;

    fn client_id(&self) -> ModelClientId;
}
```

Provider adapters normalize provider-specific streaming responses into `ModelEvent`:

- `TextDelta`
- `ThinkingDelta`
- `ToolUseStart`
- `ToolUseDelta`
- `ToolUseDone`
- `Usage`
- `Done`

The engine does not forward raw `ThinkingDelta` text to interfaces. It emits
safe `model_status` progress events around model turns and selected tool-use
blocks instead.

Native providers:

| Provider | File |
|---|---|
| OpenAI Chat | `models/src/provider/protocols/openai_completions.rs` |
| OpenAI Responses | `models/src/provider/protocols/openai_responses.rs` |
| Anthropic Messages | `models/src/provider/protocols/anthropic.rs` |
| Ollama Chat | `models/src/provider/protocols/ollama.rs` |
| External adapter v1 | `models/src/provider/external_adapter.rs` |
| Fake | `models/src/fake.rs` |

`models/src/provider/` supplies validated protocol IDs, strategy and decoder
contracts, a duplicate-safe registry, bounded SSE/JSONL framing, redacted
resolved authentication, shared bounded HTTP transport, `ProviderClient`, and
native OpenAI Chat, OpenAI Responses, Anthropic Messages, and Ollama Chat
protocol implementations with legacy parity tests. An opt-in
`external-adapter-v1` process client can run a direct argv sidecar for
unsupported wire formats with bounded timeouts, env allowlisting, secret
injection, and kill-on-drop cleanup. `ModelClientFactory` in `apps/bootstrap`
assembles native primary and fallback targets through `ProviderClient`,
resolves named profiles and bounded secret references, accepts an injected
protocol registry, rejects removed flat provider configuration, and routes
`external-adapter-v1` profiles to the process
client. Fake remains local. API and Web per-run profiles prefer a product
**type** (OpenAI / OpenAI Responses / Anthropic / Ollama / Fake) that maps to
an internal `wire_protocol`. Official and relay Base URLs use the same type.
Display names are optional and default from the endpoint host.
Environment-variable secret names remain the only browser-visible credential
surface.

The modules under `models/src/openai.rs`, `openai_responses.rs`, `anthropic.rs`,
and `ollama.rs` remain as parity-test references only. Production bootstrap
assembly does not construct those native legacy HTTP clients, and the
unreleased product has no public compatibility window for removed config.

Provider-native tool use is the preferred path for real providers. Provider adapters emit `ToolUseStart` and `ToolUseDone`, `core/src/model_turn.rs` converts those into `ToolCallAction` and `AgentEvent` values, and the root adapter maps the latter to durable `StreamEvent` values. `LlmMessage.tool_calls` plus `tool_call_id` preserve structured history for provider replay. OpenAI, Anthropic, and Ollama formatters replay that history in their native request shapes.

The JSON text action path remains for compatibility and fake-model tests. It is used only when no native tool calls were emitted, flows through `parse_action`, and produces no provider-native `tool_use_id`. Planned and unplanned loops both call the same `run_model_turn` helper, whose `build_action_from_model_output` boundary chooses native tool calls before text fallback.

`RoutingModelClient` wraps a primary model plus fallback models/providers. It can fall back only before committed visible output or committed tool-use. Provider target identity is provider plus endpoint plus model, exposed as `ModelClientId`, so two providers using the same model name do not share a health bucket.

Each routed provider candidate is attempted up to `routing.retry_max_attempts` before moving to fallback. Retryable request failures, stream interruptions, and rate limits before commit use exponential backoff from `routing.retry_backoff_base_ms` capped by `routing.retry_backoff_max_ms`; rate-limit `retry-after` values override the computed delay. Authentication, context-length, and invalid-provider-configuration errors are never retried, though another fallback candidate can still be tried if no output or tool-use has committed. After committed text or committed native tool-use begins, later stream errors are returned directly with no retry and no fallback.

`models/src/health.rs` owns `ModelHealthStore`, `HealthConfig`, and circuit state. CLI-created routed clients keep private health state configured from `routing.failure_threshold` and `routing.open_cooldown_ms`. API state creates one process-shared `ModelHealthStore` and injects it into routed model clients so API jobs share circuit breaker decisions across runs in the same process.

First-packet routing decisions are emitted through `tracing`: candidate start, skipped open circuit, committed first event, no content, timeout, error-before-commit, retry scheduling, and candidate exhaustion. These are observability records only; they do not add user-facing `StreamEvent` variants.

Relevant code:

- `models/src/protocol.rs`
- `models/src/traits.rs`
- `models/src/routing.rs`
- `models/src/error.rs`
- `models/src/provider/`
- `apps/bootstrap/src/provider.rs`
- `apps/bootstrap/src/factory.rs`

## 12. Tool System

Tools implement the `rove-core` `Tool` contract and are registered in its
`ToolRegistry`. The registry projects operational `ToolDescriptor` values into
model-visible schemas and dispatches validated execution by name. Local
built-in implementations and invocation adapters live in `runtime/src/tools/`.
Product shells assemble the default registry through
`apps/bootstrap::tool_registry` / `tool_registry_with_mcp`.

Current built-in tools:

| Tool | Purpose |
|---|---|
| `read_file` | Read UTF-8 workspace file |
| `write_file` | Write UTF-8 workspace file |
| `search_code` | Structured workspace text/regex search (workspace-bounded; not vector RAG) |
| `run_shell` | Run shell command in workspace |
| `save_memory` | Save durable memory topic |
| `reindex_memory` | Rebuild durable memory index |
| `read_memory` | Read durable memory topic |
| `request_input` | Ask user/interface for mid-run input |

Division of labor: prefer `search_code` for repo/text search; use `run_shell` for arbitrary commands.


| `mcp__<server>__<tool>` | MCP-proxied remote tools |

CLI and API construct runtime tools through the shared async
`tool_registry_with_mcp(&Workspace, ShellPolicy, mcp_config_path)` builder. That
builder registers built-ins through `tool_registry_with_shell_policy`
and then loads configured MCP tools. Root-bound tools receive the workspace root
at construction. Runtime-specific Workspace, Memory paths, approval policy, and
input provider are attached to the invocation through `RuntimeToolServices`;
they are not fields on the minimal `rove_core::ToolContext`.

Operational Tool descriptors include:

- `destructive`: requires approval unless policy allows it;
- `parallel_safe`: allows concurrent batch execution if every call is non-destructive and safe.
- optional `capability`: lets interfaces distinguish enabled tools from feature-gated disabled stubs.

The executor pipeline is currently:

```text
schema lookup -> argument validation -> pre-tool hooks -> permission -> execute -> result wrapping with mutations -> post-tool hooks
```

Argument validation supports the JSON Schema subset used by built-in tools: object, array, string, number, integer, boolean, and null type checks; required fields; enum values; nested properties; array `items`, `minItems`, and `maxItems`; numeric `minimum` and `maximum`; string `minLength` and `maxLength`; and `additionalProperties: false`. Validation failures preserve `ToolError::InvalidArgs` and happen before tool execution.

Filesystem tools resolve paths through `runtime/src/workspace/boundary.rs`. Reads canonicalize the final target; writes canonicalize existing targets or the nearest existing ancestor for new files. Both paths reject absolute paths, lexical workspace escapes, and symlink/reparse-point escapes that resolve outside the workspace.

`write_file` returns structured mutation metadata for deterministic file writes. The metadata includes path, operation type, and a textual diff; it is exposed on `ToolCallCompleted.result.mutations` and persisted to `report.json` as `tool_mutations`. Shell commands are bounded by policy and return structured stdout/stderr/exit metadata, but shell write-sets are intentionally not inferred or snapshotted.

Shell policy comes from `tool.shell`: timeout, max output bytes per stream, environment inheritance, and a denylist. The shell working directory is fixed to the workspace root. Empty commands, NUL bytes, denied substrings, timeouts, and output truncation are handled before unbounded history growth.

Tool-call parallelism is conservative and batch-scoped. When a model turn
returns multiple tool calls at once, the runtime runs them concurrently only if
every call is non-destructive and its schema is marked `parallel_safe`. Results
are still written back in model call order. Calls that depend on earlier tool
results naturally happen in later model turns and therefore run serially. The
runtime does not currently infer a general dependency DAG between arbitrary tool
calls.

Relevant code:

- `core/src/tools.rs`
- `core/src/policy.rs`
- `core/src/validation.rs`
- `runtime/src/tools/`
- `runtime/src/tools/executor.rs`
- `runtime/src/workspace/boundary.rs`
- `runtime/src/tools/hooks/`

## 13. Approval And Input

Approval is controlled by `ApprovalPolicy`:

- `ask`: destructive tools emit approval-needed events and wait for a decision;
- `auto`: destructive tools run without prompting;
- `never`: destructive tools are blocked.

Interfaces provide decisions through `ToolApprovalProvider`.

For API jobs, pending approvals live in the live `JobRecord` and are exposed through:

```text
POST /jobs/{job_id}/approvals/{call_id}
```

`request_input` uses `UserInputProvider`. For API jobs, pending inputs are exposed through:

```text
POST /jobs/{job_id}/inputs/{input_id}
```

Pending approval/input answer channels are live-only. API rows are persisted while live for audit and state display, but answerable channels are not reconstructed after process restart.

Relevant code:

- `runtime/src/foundation/types.rs`
- `runtime/src/tools/tool_input.rs`
- `runtime/src/engine/tool_turn.rs`
- `apps/cli/src/cli/approval.rs`
- `apps/cli/src/cli/input.rs`
- `apps/api/src/lib.rs`
- `apps/cli/src/terminal/interaction.rs`
- `apps/cli/src/tui/`
- `runtime/src/tools/request_input.rs`

## 14. State Artifacts

Each run writes readable files under:

```text
.rove/runs/<run_id>/
  trace.jsonl
  task_state.json
  report.json
```

`trace.jsonl` is append-only event history and the source used to rebuild event
rows during repair. Every line is one serialized `StreamEvent`; terminal
planned attempts use `step_result` as their canonical ledger transition.

`task_state.json` is the resume snapshot. It includes:

- identity;
- goal;
- step count;
- conversation history;
- summary;
- prompt checkpoint;
- plan state;
- materialized terminal step records and any active step attempt.

`report.json` is the final aggregate report. It includes:

- identity;
- workspace metadata;
- model id;
- final status;
- termination reason;
- step count;
- total usage;
- tool counts;
- terminal step records with per-attempt usage, evidence/tool references, and
  mutations;
- final output.

Relevant code:

- `runtime/src/state/store.rs`
- `runtime/src/state/trace.rs`
- `runtime/src/state/artifacts.rs`
- `runtime/src/state/report.rs`

## 15. SQLite State Index

SQLite is the query/replay index, not the only source of truth. Files remain the readable artifacts.

The index stores:

- sessions;
- jobs;
- runs;
- events and event offsets;
- reports;
- task state metadata;
- pending approval/input schema slots;
- migrations.

Startup and maintenance behavior:

- `StateIndex::initialize` creates/migrates the database.
- API startup marks stale running jobs as `interrupted`.
- task states can be lazily imported from artifacts.
- `rove state repair` imports task state artifacts, report artifacts, and trace
  events, including `step_result`; corrupted trace lines are counted and
  skipped. SQLite has no separate mutable ledger table in this phase.
- `rove state cleanup` removes expired rows and safe run artifacts.

Useful commands:

```powershell
cargo run -p rove-cli -- sessions
cargo run -p rove-cli -- state repair
cargo run -p rove-cli -- state cleanup
```

Relevant code:

- `runtime/src/state/index.rs`
- `runtime/src/state/store.rs`
- `apps/cli/src/cli/state.rs`

### ProductStore

Web Complete C0 adds a separate API-global SQLite database at
`<configured state_dir>/product.sqlite`. It owns:

- known Folder/Repo product workspaces;
- server-owned product sessions and one active-turn claim per session;
- immutable ordered runtime session/job/run bindings;
- secret-reference-only provider profiles;
- safe product preferences;
- schema versions, durable M1 migration preparations, and migration
  receipts/mappings/issues.

It intentionally does not copy canonical runtime event payloads, task state, or
reports. Those facts remain in each execution workspace's `StateStore` and are
read on demand by the transcript projector.

## 16. Memory

Memory has three layers:

| Layer | Storage | How it is used |
|---|---|---|
| Working memory | in-memory prompt messages | Included in the current context |
| Session memory | `memory.session_dir/<session_id>.md` | Loaded on resume / same session |
| Durable memory | `memory.durable_dir/MEMORY.md` + `topics/*.md` | Recalled by lexical relevance |

Session memory is written by a post-run hook when a run completes with `TerminationReason::Final`. The summary is deterministic markdown containing the goal, final status, output excerpt, completed plan steps, tools used, and file write-set metadata when tools report mutations. Run and plan loops also append `## Flush at <timestamp>` blocks before compaction so useful notes are not lost when detailed history is summarized; final summaries preserve those flush blocks.

Durable memory is managed by tools:

- `save_memory`
- `reindex_memory`
- `read_memory`

`save_memory` rejects unsafe topic names, likely secrets, and transient one-off content before writing. Topic frontmatter records `type` (`user`, `feedback`, `project`, or `reference`), `scope`, `source`, `confidence`, and timestamps.

Durable recall is bounded by `memory.recall_limit` and uses CJK-aware tokenization, smoothed IDF scoring, field weights, confidence scaling, and a small recency boost. The prompt path recalls all memory types; lower-level recall calls can provide a hard `type_filter`.

CLI and API engine assembly pass `AppConfig::memory_paths()` into the runtime, so prompt memory loading, the session-memory post-run hook, and memory tools all use the same resolved `memory.session_dir`, `memory.durable_dir`, and `memory.recall_limit` values. Defaults still resolve to `.rove/memory/sessions` and `.rove/memory`.

Relevant code:

- `runtime/src/memory/layered.rs`
- `runtime/src/memory/paths.rs`
- `runtime/src/memory/session.rs`
- `runtime/src/memory/durable.rs`
- `runtime/src/tools/hooks/session_memory.rs`
- `runtime/src/tools/memory.rs`

## 17. MCP

MCP integration registers remote server tools into the local `ToolRegistry`.
Both CLI and API jobs use the same runtime registry builder, so configured MCP
tools are available through CLI runs, API jobs, and both Web surfaces via the
API proxy.

Config path:

```text
.rove/mcp_servers.json
```

Example config:

- `docs/examples/mcp_servers.json`

Supported transports:

- `stdio`;
- `sse`.

For stdio, rove spawns the configured command, sends JSON-RPC messages over stdin, reads stdout lines, initializes the MCP session, calls `tools/list`, and registers each returned tool as:

```text
mcp__<sanitized_server_name>__<remote_tool_name>
```

The proxy maps MCP annotations into local tool metadata:

- `destructiveHint` -> `destructive`;
- `readOnlyHint` -> `parallel_safe` when not destructive.

Each server can include an optional `policy` object:

```json
{
  "name": "filesystem",
  "transport": "stdio",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
  "policy": {
    "request_timeout_ms": 30000,
    "stderr_capture_bytes": 16384
  }
}
```

`request_timeout_ms` bounds stdio initialize/list/call requests and SSE HTTP requests. `stderr_capture_bytes` controls how much stdio stderr is retained for timeout and startup diagnostics. MCP JSON-RPC errors are mapped into structured tool execution failures instead of raw protocol blobs. Stdio child processes are killed when their registered client is dropped.

Default test coverage uses Python stdio fixtures for normal registration, timeout, JSON-RPC error mapping, and child cleanup. A real stdio filesystem MCP smoke test is available behind an explicit environment gate:

```powershell
$env:ROVE_MCP_FILESYSTEM_SMOKE = "1"
cargo test --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture
```

By default that smoke test runs `npx -y @modelcontextprotocol/server-filesystem <temp-dir>` and verifies `read_file`. Override `ROVE_MCP_FILESYSTEM_COMMAND` and `ROVE_MCP_FILESYSTEM_ARGS` when testing a locally installed or pinned server. GitHub or database MCP servers should remain optional and secret-gated when added.

Relevant code:

- `runtime/src/tools/mcp_proxy.rs`
- `tests/mcp.rs`
- `tests/fixtures/mcp_mock_server.py`

## 18. RAG

Built-in vector RAG has been removed. Use tools and layered memory for workspace context.


## 19. API Security

API security is middleware around all routes.

Implemented controls:

- bearer token auth when `api.token_auth` is configured;
- CORS origin allowlist;
- per-process rate limiting;
- rejection of remote bind without token unless explicitly marked unsafe.

Limitations:

- no multi-user identity;
- no distributed rate limiting;
- no browser login/session flow. The local Web application supports API bearer
  tokens through its server-side Next.js proxy, not through client-side headers.

These limitations are deployment/product scope for a later phase, not active
runtime gaps for the current local-first target.

Relevant code:

- `src/config.rs`
- `apps/api/src/security.rs`
- `tests/api.rs`

## 20. Testing And Verification

Default Rust checks:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Web checks:

```powershell
cd apps/web
pnpm test
pnpm typecheck
pnpm build
```

Optional browser E2E checks:

```powershell
cd apps/web
pnpm test:e2e
```

Useful focused tests:

```powershell
cargo test interfaces::tui --lib
cargo test interfaces::terminal --lib
cargo test --test cli_repl
cargo test --test api
cargo test --test e2e
cargo test --test mcp

```

TUI terminal verification is split between deterministic renderer/terminal
tests and an opt-in real Unix PTY gate:

```powershell
cargo test interfaces::tui --lib
cargo test interfaces::terminal --lib
python scripts/tui-pty-smoke.py --run
```

The PTY command is intentionally not part of the default Rust gate. On Windows
it reports `status: "skipped"` and exit code `77` until a native ConPTY runner
is added; do not convert that result into a pass claim.

Deterministic local benchmark checks:

```powershell
cargo test --test bench
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

`rove-bench` reads JSON benchmark task definitions, creates isolated local
workspaces under the output directory, runs scripted fake-model tasks through
the real engine/tool/state paths, and prints a JSON report with pass/fail state
and artifact paths. The default `benchmarks/agent-smoke.json` suite has no
network credential requirement and covers echo/tool smoke, file writing, and
resume context behavior.

The current M0-M6 milestone proof map lives in
`docs/runtime/acceptance-matrix.md`.

CI is split:

- `.github/workflows/ci.yml` runs default Rust and Web checks.
- Optional provider, MCP, and browser gates remain opt-in outside default CI.

## 21. Runtime Docs As Source Of Truth

`docs/runtime/` is the current behavior source of truth. Historical design docs
can explain why the runtime exists, but implementation changes should update
the runtime docs first:

- `implementation-status.md` records closed gaps and remaining future scope;
- `implementation-guide.md` records startup paths, runtime flow, artifacts,
  verification, and maintainer procedures;
- `subsystems.md` records subsystem boundaries and intentionally deferred work;
- root `README.md` stays focused on accurate user-facing setup and commands.

Code hygiene is part of the default gate. `src/lib.rs` must not use a global
`#![allow(dead_code)]`; `cargo clippy --all-targets -- -D warnings` is expected
to surface unused stubs. Any local dead-code allowance should carry an inline
reason that explains why the item is intentionally retained.

## 22. Common Maintenance Tasks

### File Size And Module Shape

The historical architecture notes suggested a hard line-count limit for Rust
files. The current runtime docs do not treat that as a binding rule. Prefer
splitting modules when it improves ownership, testability, or reviewability;
keep related code together when a single file makes the behavior easier to
trace. The practical standard is clear responsibility boundaries, not an
absolute line-count threshold.

When adding a new tool:

1. Implement `Tool`.
2. Define schema with `destructive` and `parallel_safe`.
3. Register it in CLI and API engine construction.
4. Add executor or e2e coverage.
5. If Web needs to render the result specially, update Web state/types.

When adding a new event:

1. Add a `StreamEvent` variant.
2. Add `event_name`.
3. Update CLI rendering.
4. Update API persistence/SSE assumptions if needed.
5. Update `RunArtifactRecorder` if artifacts should change.
6. Update `apps/web/lib/rove-types.ts` and reducer handling.
7. Add at least one integration test.

When changing run identity or resume:

1. Check `RunRequest`, `RunHandle`, `StateStore::start_run`, and `RunArtifactRecorder`.
2. Check CLI resume path.
3. Check API job creation and persisted replay.
4. Check `.rove/runs/<run_id>/task_state.json` compatibility.

When changing provider tool-use:

1. Update provider parser tests.
2. Update `ModelEvent` normalization.
3. Check native tool-use normalization in `core/src/model_turn.rs` and durable
   translation in `runtime/src/engine/model_turn.rs`.
4. Check structured history round-trip tests.
5. Preserve the native-before-text action conversion in `build_action_from_model_output`.

## 23. Known Gaps And Risks

These are implementation-level issues to keep in mind before extending the system.

1. The implemented lifecycle evaluator is deterministic and rule-first; it
   does not call a model for ambiguous evidence. An independent Finalizer,
   public and globally enforced multidimensional budgets, structured budget and
   finalization events, and model-on-ambiguity evaluation remain unimplemented.
   Resume uses the materialized `TaskState` lifecycle projection and does not
   yet reconcile a canonical trace tail written after the latest snapshot.

2. Built-in vector RAG is removed. Workspace retrieval is explicit bounded
   tools plus layered file memory; optional external semantic retrieval remains
   future work.

3. TUI real-terminal evidence is platform-scoped. The standard-library PTY
   smoke covers Unix when explicitly enabled, while Windows ConPTY automation is
   not implemented and therefore skips with a typed result. The deterministic
   TestBackend and terminal lifecycle tests do not substitute for that missing
   platform gate.

4. TUI display sanitization is defense in depth. It bounds and redacts common
   reasoning, token, and secret-shaped text, but it is a heuristic projection,
   not a proof that arbitrary provider text contains no secrets. New display
   fields must remain typed, bounded, and covered by negative tests.

5. Web Complete C0's ProductStore, exact product-session continuation,
   transcript projection, browser migration state machine, and typed client are
   implemented. The default M1 shell does not call them yet, so refresh restore,
   API-authoritative UI catalogs and profiles, durable routes, and session
   switching remain C1/C2 work. Final migration UX and live product-shell
   acceptance remain C3 work.

6. Browser evidence is split by route. The default product shell has
   mock-backed `shell.spec.ts` coverage. The deterministic `local-full` runner
   invokes `real-api.spec.ts`, which opens `/dev/workbench`; it proves the
   advanced surface against the live API, not the product shell's complete
   live-API lifecycle. The provider integration runner's browser script also
   still looks for legacy `Task`, `Model`, and `Steps` controls while navigating
   `/`, so that optional step is stale after Web M1 and must not be counted as
   current product-shell evidence.


## 24. Current Verification Baseline

As of the Web M1 integration baseline on 2026-07-25, the following categories
were run and passed on their implementation branches:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd apps/web; pnpm test
cd apps/web; pnpm typecheck
cd apps/web; pnpm build
git diff --check
```

C0 includes focused ProductStore, product-route, exact-session resume,
transcript, migration, stream-finalization, job-start lifecycle, runtime commit
guard, and Web client tests.
