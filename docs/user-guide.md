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
   - Sensitive operations prompt for approval; approve or reject in the dialog.

Notes:
- To try it without configuring a model, run `rove --model fake`.
- Other modes: `rove repl` is line-by-line interaction, `rove exec "<task>"` is one-shot, `rove sessions` lists resumable sessions.

## Browser (Web)

Purpose: a graphical interface for managing workspaces, sessions, and settings.

Steps:

1. Run `powershell -ExecutionPolicy Bypass -File scripts/serve.ps1` at the repo root — one command that builds the console and starts `rove-api`, which serves it at <http://127.0.0.1:8787>. For a real model, set the key env var first and add `-Provider`. (The two-process dev variant is `scripts/dev.ps1` + <http://localhost:3000>.)
2. The home surface lets you pick the workspace inline and jump back into a recent session; typing a task there creates the session and sends the message.
3. In Settings → Providers, create a profile and activate it. When the page is served by the local `rove-api` (the default), you can paste the provider key directly — it is sent once to the local API, verified, and stored in the system credential store. Otherwise (e.g. the page is opened through a remote address), fill in the env var name holding the key instead. Without a provider the composer still takes drafts, but sending stays disabled until one is configured.
4. In a session, Enter sends and Shift+Enter inserts a newline (the send key is configurable under Settings → Keyboard); Ctrl/Cmd+Enter also sends, and Alt+Enter steers a running turn. You can keep sending messages while it runs: they queue above the composer, and can be promoted to the front or withdrawn. The send button turns into Stop while a run is live. When a run has a plan, a todo dock sits above the composer showing done/total progress and the current step — click it to unfold the full step list. A minimap rail beside the transcript places one marker per turn at the turn's own position with a band showing the visible slice; click a marker or use arrow keys to jump.
5. The left rail lists sessions grouped by recency (today, yesterday, this week, older, archived) and workspaces as collapsible project groups. A session row's menu offers rename, pin, archive/restore, branch, copy link, and a two-click delete; Ctrl/Shift-click selects several sessions for batch archive or delete. Pinned sessions get their own shelf at the top, project rows can be dragged to reorder, and hovering a row opens a card with the session's status and settings. The rail resizes from its right edge (240–520 px, arrows work on the focused divider) and collapses with the header button.
6. The work panel on the right stays closed until you open it — use the floating button at the top-right corner of the conversation (or `Ctrl+.`); it remembers whether it was open. The panel is a tab strip: it opens on the changes (evidence) view, `+` opens a launcher for run status, pending approvals, files, changes, review and subagent panes, and every opened file gets its own tab. Tabs close from their × button, middle-click, or Delete/Backspace, reorder by dragging or Alt+←/→, and arrow keys move the selection. The strip's width and order are remembered per session. The maximize button gives the panel the whole area beside the left rail; on narrow screens it becomes a modal drawer. The files tab browses directories with breadcrumbs, and the file viewer syntax-highlights, renders Markdown, caps at ~5,000 lines, and can reveal the file in the OS. A session's row menu also offers evidence export (JSON, HTML, Markdown). Per-turn usage, context estimates, and compaction/pruning facts live on the run tab's evidence block — the conversation itself stays prose.

Note: a pasted key crosses the browser exactly once — into the loopback onboarding endpoint — and lands in the OS credential store; it is never persisted in app state, shown again, or written to session records. Refreshing the page restores the conversation, and incomplete recovery is stated explicitly. Settings → General also offers a skin selector (graphite or warm).

## Desktop app (Windows)

Purpose: a local app with no terminal required.

Steps:

1. Install the MSI or NSIS package (packaging in docs/development.md).
2. In Settings → Providers, enter your key: the input is masked and the key goes to the Windows Credential Manager.

Note: installers are not yet signed, and the installed end-to-end flow has not been formally verified.

## Trusting a project

Purpose: allow project config, `.env`, MCP servers, hooks, and agent definitions to take effect.

Steps:

1. By default, none of these take effect.
2. Run `rove trust` in the terminal, or use the Project Trust settings in Web to grant capabilities item by item.

Note: the grant binds to the exact directory and a config digest; any config change requires re-confirmation.

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
- No sub-agent delegation.
- macOS and Linux packages are not yet verified.
