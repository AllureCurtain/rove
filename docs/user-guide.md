# User guide

For people using Rove; no implementation details.

## Terminal (TUI)

Purpose: start a full-screen agent session in the current directory.

Steps:

1. Install: run `cargo install --path apps/cli` at the repo root to get the `rove` command.
2. Configure a model (once):

   ```powershell
   $env:OPENAI_API_KEY = "<your key>"
   rove provider add openai-main --provider openai --base-url https://api.openai.com/v1 --model gpt-4.1-mini --secret-env OPENAI_API_KEY
   rove provider test openai-main
   rove provider use openai-main
   ```

   Without `--secret-env`, a hidden input prompt appears and the key is stored in the system credential store. OpenAI-compatible gateways (e.g. SiliconFlow) also use `--provider openai`, just with a different `--base-url` and model.
3. `cd` into the directory you want to work in and run `rove`.
4. In the TUI:
   - `/model` switches the model, affecting only the next turn.
   - `Ctrl+R` selects a resumable session. Switching to another session loads its model selection, queued messages and selected run's saved transcript. A failed session switch keeps the current session; unavailable history has its own notice. Merely selecting a session does not execute its queue. An unsent composer draft stays available.
   - Restarting the TUI on a session continues delivery of a queued follow-up: a claim the previous turn already reserved is completed first, then the oldest still-queued message is claimed and run. An `needs_attention` queue is not auto-confirmed — it still needs the same explicit recovery as the Web Recover button.
   - `F2` opens saved history while idle. Left/Right browse earlier/later runs in the current session; Up/Down and PageUp/PageDown scroll; Esc returns to the conversation. Browsing does not change the run you will continue or reactivate old approvals. Missing, corrupt, incomplete or display-limited records are marked explicitly, and you can browse past an unreadable run. Large runs have a bounded display, and nested subagent details are currently omitted with a partial-history notice.
   - Sensitive operations prompt for approval; approve or reject in the dialog.

Notes:
- To try it without configuring a model, run `rove --model fake`.
- Other modes: `rove repl` is line-by-line interaction, `rove exec "<task>"` is one-shot, `rove sessions` lists resumable sessions.

## Browser (Web)

Purpose: a graphical interface for managing workspaces, sessions, and settings.

Steps:

1. On macOS/Linux, run `bash scripts/serve.sh`; on Windows, run `powershell -ExecutionPolicy Bypass -File scripts/serve.ps1` at the repo root — one command that builds the console and starts `rove-api`, which serves it at <http://127.0.0.1:8787>. Configure a real model in Settings → Providers. To keep provider environment overrides at startup, export them and add `--provider` (POSIX) or `-Provider` (Windows). The POSIX launcher accepts `--workspace /absolute/path` for an existing project and does not execute a repository `.env` file. (The two-process dev variant is `scripts/dev.ps1` + <http://localhost:3000>.)
2. The home surface lets you pick the workspace inline and jump back into a recent session; typing a task there creates the session and sends the message. On a fresh install a short "Getting started" checklist sits above the composer — configure a provider, open a workspace, send the first prompt — with a button per unfinished step the UI can drive; the offline demo is labelled explicitly and does not count as a configured real provider; it disappears once all three are in place, or when dismissed.
3. In Settings → Providers, create a profile and activate it. When the page is served by the local `rove-api` (the default), you can paste the provider key directly — use **Verify and save** to send it once to the local API, verify it, and store it in the system credential store. While a new key is entered, connection tests and model listing wait until it is saved; they otherwise use the saved credential or configured environment variable. An invalid address or model is explained beside the form. Editing a saved profile with an empty credential field preserves its stored key; changing authentication style requires a new key or an explicit environment variable. The default selection applies to new sessions; use the model menu beside the composer to change an existing session. Otherwise (e.g. the page is opened through a remote address), fill in the env var name holding the key instead. Without a provider the composer still takes drafts, but sending stays disabled until one is configured. If the active workspace has not granted provider credentials yet, a prompt appears on the same panel; the conversation prompt follows that session's model even if the default for new sessions differs — click Authorize there once per workspace, or messages sent with the provider will fail to start.
4. In a session, Enter sends and Shift+Enter inserts a newline (the send key is configurable under Settings → Keyboard); Ctrl/Cmd+Enter also sends, and Alt+Enter steers a running turn. You can keep sending messages while it runs: they queue above the composer, and can be promoted to the front or withdrawn. The send button turns into Stop while a run is live. A confirmed stop keeps any received partial answer and does not become active again when an older transcript arrives; a subsequent task can still start normally. While the current conversation briefly reloads its completed run, clicking Send or using the send shortcut queues that explicit send until restoration finishes. Editing the draft or switching sessions cancels the deferred send. A pending approval or question changes the activity line to waiting for you; it does not claim the blocked tool is already executing. When a run has a plan, a todo dock sits above the composer showing done/total progress and the current step — click it to unfold the full step list. A minimap rail beside the transcript places one marker per turn at the turn's own position with a band showing the visible slice; click a marker or use arrow keys to jump — hovering the rail magnifies markers near the cursor and pops a card naming the nearest turn. The context ring beside the composer opens a small popover with the token breakdown (input, output, cached, window fill) and a used/remaining toggle. Its Context tools action opens the run panel, where manual compaction is available while idle. Before usage has been reported, a Context tools icon opens that same panel directly. If the server could not start queued messages (the session shows "needs attention"), a Recover session button appears above the queue and confirms them all at once. Reasoning-capable providers (DeepSeek, OpenAI reasoning models) stream their chain-of-thought into a collapsible "thinking" block above the answer when the session's reasoning level is set (Settings → session model); the reply itself never contains the reasoning.
5. The left rail lists sessions grouped by recency (today, yesterday, this week, older, archived) and workspaces as collapsible project groups. A session row's menu offers rename, pin, archive/restore, branch, copy link, and a two-click delete; Ctrl/Cmd-click or Shift-click selects several sessions for batch archive or delete. Pinned sessions get their own shelf at the top, project rows can be dragged to reorder, and hovering a row opens a card with the session's status and settings. The rail resizes from its right edge (240–520 px, arrows work on the focused divider) and collapses with the header button.
6. The work panel on the right stays closed until you open it — use the floating button at the top-right corner of the conversation (or `Ctrl+.`); it remembers whether it was open. The panel is a tab strip: it opens on the changes (evidence) view, `+` opens a launcher for run status, pending approvals, files, changes, review and subagent panes, and every opened file gets its own tab. Tabs close from their × button, middle-click, or Delete/Backspace, reorder by dragging or Alt+←/→, and arrow keys move the selection. The strip's width and order are remembered per session. The maximize button gives the panel the whole area beside the left rail; on narrow screens it becomes a modal drawer. The files tab browses directories with breadcrumbs, and the file viewer syntax-highlights, renders Markdown, caps at ~5,000 lines, and can reveal the file in the OS. Expanded tool cards show the call's input arguments verbatim after its outputs; a path like `src/main.rs:12` in the header summary, the input payload, or a result list is a link that opens that file (at the line, when the reference carries one) as a work-panel tab, and a URL opens externally. A call that wrote or edited files collapses to a change card — A/M/D mark, the path, and +/− counts — that expands to the diff hunks and opens that file's tab from its path. The card's rollback action calls `POST /product/sessions/{id}/mutations/revert` on the server: a create is undone only when its exact recorded bytes still match, and an edit in a repository workspace is reversed only when both the current file and reconstructed original match their recorded byte identities. A running session, changed file, symlink, incomplete patch or older record without byte identities is refused. Close external editors or other tasks writing that file before rollback; the session lock does not coordinate unrelated processes. A session's row menu also offers evidence export (JSON, HTML, Markdown). Loading older history keeps the current reading position while rows settle; a new scroll or navigation gesture takes control immediately. Per-turn usage, context estimates, and compaction/pruning facts live on the run tab's evidence block — the conversation itself stays prose. An HTML file's preview button opens an embedded `preview` tab right in the panel — the page runs on an isolated loopback origin (it cannot read product credentials) and can be reloaded or popped out to a real browser tab. Only workspace previews work this way; third-party sites that send `X-Frame-Options` cannot be framed.

When a turn ends with an automatic execution summary, the conversation shows its recorded outcome with expandable **Execution details**. Successful results start collapsed; incomplete, failed or uncertain results show their details immediately. The label describes the recorded execution outcome, not an independent verification of correctness. Model answers stay unchanged, and the message copy action includes the original complete summary.

Note: a pasted key crosses the browser exactly once — into the loopback onboarding endpoint — and lands in the OS credential store; it is never persisted in app state, shown again, or written to session records. Refreshing the page restores the conversation, and incomplete recovery is stated explicitly. Settings → General also offers a skin selector (graphite or warm).

## Desktop app (Windows)

Purpose: a local app with no terminal required.

Steps:

1. Install the MSI or NSIS package (packaging in docs/development.md).
2. In Settings → Providers, enter your key: the input is masked and the key goes to the Windows Credential Manager.

Note: installers are not yet signed, and the installed end-to-end flow has not been formally verified.

## Desktop source build (macOS)

A local Apple Silicon `.app` can be built from `apps/desktop` with `pnpm dlx @tauri-apps/cli@2 build --bundles app --ci`. The result is `target/release/bundle/macos/Rove.app` (or the configured Cargo target directory). Folder selection, offline tasks and session continuation after restart have been checked locally. Signed distribution and clean-machine installation remain unverified. In Settings → Providers, macOS/Linux use the shared **Verify and save** password form. The input clears at submission and the local host saves the verified key in the OS credential store; when verification fails the key stays in the field so the endpoint or model can be corrected before retrying. Windows retains its native credential window. Linux needs an available OS keyring service.

## Closing Desktop

By default, closing the Desktop window hides it to the system tray: the app, its embedded API, and any running task keep working, and the tray menu's **Show Rove** brings the window back. Settings → General → **Closing the window** switches the behavior to **Quit**, which exits the app on window close; the choice is stored in the host config and survives restarts. The tray menu's **Quit Rove** and the application-menu Quit always exit regardless of this setting.

A real quit — the setting above set to **Quit**, the tray Quit item, or the application-menu Quit — cancels active tasks and waits up to five seconds for the local API to save their terminal state. Repeated quit requests share that wait. Unapproved tool calls remain unexecuted. On reopening, cancelled tasks stay cancelled; a forced termination or shutdown timeout may instead leave interrupted work that requires explicit recovery.

If another Rove window or API host already uses the same data directory, a second start is refused. Return to the running app, or close it before reopening. Changing the API port does not permit sharing an active data directory. A process crash releases ownership automatically; do not delete `.rove-api.lock` to clear it. Separate data roots can run independently.

## Trusting a project

Purpose: allow project config, `.env`, MCP servers, hooks, and agent definitions to take effect.

Steps:

1. By default, none of these take effect.
2. Run `rove trust` in the terminal, or use the Project Trust settings in Web to grant capabilities item by item.

Note: the grant binds to the exact directory and a config digest; any config change requires re-confirmation.

## Execution budgets

Purpose: bound how much work a run and each plan step may do.

Defaults: `max_steps` (default 20) caps planned step attempts. Each plan step additionally gets its own model-turn ceiling, `max_model_turns_per_step` (default 48), so one step can alternate thinking with tool calls and still synthesize a conclusion without draining the whole run.

Override: the user config (`~/.rove/config.toml` by default) may set any `[runtime.execution]` field, and once the `project_configuration` trust capability is granted a workspace `.rove/config.toml` may do the same — `max_plan_steps`, `max_step_attempts`, `max_model_turns`, `max_model_turns_per_step`, `max_tool_calls`, `max_tool_calls_per_step`, `max_plan_revisions`, `max_model_repairs`, `max_finalization_turns`, `max_wall_time_ms`, `max_total_tokens`, `max_cost_microunits`. The numeric fields also accept `ROVE_EXECUTION_*` environment variables (`ROVE_EXECUTION_MAX_MODEL_TURNS_PER_STEP`, `ROVE_EXECUTION_MAX_MODEL_TURNS`, `ROVE_EXECUTION_MAX_TOOL_CALLS`, `ROVE_EXECUTION_MAX_TOOL_CALLS_PER_STEP`, `ROVE_EXECUTION_MAX_STEP_ATTEMPTS`, `ROVE_EXECUTION_MAX_PLAN_STEPS`, `ROVE_EXECUTION_MAX_PLAN_REVISIONS`, `ROVE_EXECUTION_MAX_MODEL_REPAIRS`, `ROVE_EXECUTION_MAX_FINALIZATION_TURNS`, `ROVE_EXECUTION_MAX_WALL_TIME_MS`, `ROVE_EXECUTION_MAX_TOTAL_TOKENS`, `ROVE_EXECUTION_MAX_COST_MICROUNITS`); `evaluator_mode` and `finalizer_policy` remain config-file-only. Precedence is user config < project config < env.

```toml
[runtime.execution]
max_model_turns_per_step = 24
```

Zero is rejected at load; a per-step limit may not exceed its matching global limit.

The context window ceiling follows the same layering: `context_hard_limit_tokens` (default 128_000) refuses a prompt that outgrows the window, `context_soft_limit_tokens` (default 96_000) triggers automatic compaction, and `context_reserved_tokens` (default 16_000) keeps headroom for the answer. Set `ROVE_CONTEXT_HARD_LIMIT_TOKENS`, `ROVE_CONTEXT_SOFT_LIMIT_TOKENS` and `ROVE_CONTEXT_RESERVED_TOKENS` — or the matching `[runtime]` keys — for models with smaller windows; `reserved` and `soft` must both stay below `hard`.

## Reviewing changes (Review)

Purpose: have the agent review current changes read-only, without modifying any files.

Steps: run `rove review` in the terminal (optionally with `--base <rev>` or `--commit <sha>`). The Web session's review pane lists finished reviews.

## Where data lives

- Run records, memory, and session data live in the user data directory: `%LOCALAPPDATA%\rove` on Windows; `rove state paths` shows the resolved locations.
- Data from older versions that sat in the project's `.rove/` can be preview-migrated with `rove state migrate`, then applied with `--apply`.

## Current limitations

- Local single user only: no accounts, cloud sync, or billing.
- Shell tools are policy-constrained and approval-gated, but not container-sandboxed.
- No vector search; the agent learns the project by reading files and searching code.
- Sub-agent delegation renders in two places: a `task` tool's card in the transcript carries a topology block naming the delegated child (status, model, steps, elapsed) with a stop button while it runs, and the work panel's **subagents** tab lists the run's delegations and shows the selected child's own transcript (prompt as the user row, then its tools and answers) with a disabled composer that reads "driven by the main agent". `POST /jobs/{job_id}/subagents/{call_id}/stop` cancels one delegated subagent without cancelling the job.
- macOS and Linux packages are not yet verified.
