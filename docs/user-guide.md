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

1. Run `powershell -ExecutionPolicy Bypass -File scripts/dev.ps1` at the repo root. For a real model, set the key env var first and add `-Provider`.
2. Open <http://localhost:3000> and pick a folder or repository as the workspace.
3. In Settings → Providers, create a profile, fill in the env var name holding the key, test the connection, then activate.
4. Create a session and type a task. You can keep sending messages while it runs: they queue, and can be promoted to the front or withdrawn.
5. Use the Inspector on the right to view files, diffs, artifacts, and usage; you can also export evidence (JSON, HTML, Markdown).

Note: the browser never sees your key. Refreshing the page restores the conversation, and incomplete recovery is stated explicitly.

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

Steps: run `rove review` in the terminal (optionally with `--base <rev>` or `--commit <sha>`), or click Review in a Web session.

## Where data lives

- Run records, memory, and session data live in the user data directory: `%LOCALAPPDATA%\rove` on Windows; `rove state paths` shows the resolved locations.
- Data from older versions that sat in the project's `.rove/` can be preview-migrated with `rove state migrate`, then applied with `--apply`.

## Current limitations

- Local single user only: no accounts, cloud sync, or billing.
- Shell tools are policy-constrained and approval-gated, but not container-sandboxed.
- No vector search; the agent learns the project by reading files and searching code.
- No sub-agent delegation.
- macOS and Linux packages are not yet verified.
