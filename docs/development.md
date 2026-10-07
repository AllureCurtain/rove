# Development guide

## Requirements

- Git; on Windows, run `git config core.longpaths true` once inside the repo
- Rust stable (`rust-toolchain.toml`)
- Node.js 22, pnpm 10 (Web and Desktop)
- Desktop packaging: Tauri 2 platform dependencies; Windows needs WebView2 and MSVC build tools
- Optional: Python 3 (`scripts/tui-pty-smoke.py`)

## First-time setup

```bash
git clone https://github.com/AllureCurtain/rove.git && cd rove
cargo build -p rove-cli
cd apps/web && pnpm install --frozen-lockfile && cd ../..
cp .env.example .env    # only needed for real-provider integration runs
```

No database initialization: both SQLite databases are created and migrated on first open.

## Environment variables

Config options are authoritative in the environment layer of `apps/bootstrap/src/config.rs`. The root `.env.example` covers only the integration-run variables loaded by the scripts. Provider keys are only referenced by variable name (e.g. `OPENAI_API_KEY`) and never written to any file.

| Variable | Purpose |
| --- | --- |
| `ROVE_DATA_ROOT` | User data root; must be an absolute path. Tests and integration scripts should point at a disposable directory |
| `ROVE_CONFIG_ROOT` | Location of the user provider catalog, default `~/.rove` |
| `ROVE_PROJECT_TRUST_STORE` | Project Trust store path |
| `ROVE_PROVIDER` / `ROVE_MODEL` | Provider and model for this process, e.g. `fake` |
| `ROVE_API_BIND_ADDR` / `ROVE_API_TOKEN` | API bind address and bearer token; the Web proxy also reads `ROVE_API_TOKEN` |
| `ROVE_API_CORS_ORIGINS` / `ROVE_API_RATE_LIMIT_PER_MINUTE` | CORS allowlist and per-minute request cap |
| `ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH` | Allow binding a non-loopback address without auth; only when the risk is understood |
| `ROVE_WEB_API_BASE` | Upstream API address for the Web proxy |
| `ROVE_WEB_DIST` | Web bundle directory for `rove-api` to serve (same effect as `--web-dist`) |
| `ROVE_MCP_CONFIG` | Explicit MCP catalog file |
| `ROVE_DISABLE_NATIVE_FOLDER_PICKER` | Disable the native folder picker, for headless environments |
| `ROVE_STATE_*` / `ROVE_MEMORY_*` / `ROVE_CONTEXT_*` / `ROVE_SHELL_*` / `ROVE_ROUTING_*` | Overrides for the corresponding config sections; semantics in `config.rs` |

## Commands

| Purpose | Command |
| --- | --- |
| Keyless TUI | `cargo run -p rove-cli -- --model fake` |
| REPL / one-shot | `cargo run -p rove-cli -- repl --model fake` / `cargo run -p rove-cli -- exec --model fake "<task>"` |
| Product mode (one process) | `powershell -ExecutionPolicy Bypass -File scripts/serve.ps1` — builds `apps/web` (`pnpm build:web` → `web-dist/`), then runs `rove-api --web-dist`; add `-Provider` for a real provider, `-SkipWebBuild` to reuse an existing bundle. Liveness: `GET /health` |
| Start API + Web (fake) | `powershell -ExecutionPolicy Bypass -File scripts/dev.ps1` (add `-Provider` for a real provider) |
| API only | `cargo run -p rove-api` (add `--web-dist apps/web/web-dist` to also serve the built console on the same origin) |
| Web dev | `cd apps/web && pnpm dev` |
| Desktop dev / package | `cd apps/desktop && pnpm dlx @tauri-apps/cli@2 dev` / `... build --target x86_64-pc-windows-msvc` |
| Format & lint | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` |
| Targeted Rust tests | `cargo test -p <crate>`; integration tests `cargo test -p rove-integration-tests --test it <module>::` (e.g. `e2e::`); API contract `--test api` |
| Full Rust tests | `cargo test --workspace` (slow; before commits and in CI) |
| Update OpenAPI snapshot | After API changes: `$env:ROVE_UPDATE_OPENAPI=1; cargo test -p rove-integration-tests --test api openapi_snapshot` (in Git Bash: `ROVE_UPDATE_OPENAPI=1 cargo test ...`); rewrites `apps/api/openapi.json` |
| Web tests | `cd apps/web && pnpm test && pnpm typecheck && pnpm build` |
| Web E2E | `pnpm test:e2e` (mocked backend), `pnpm test:e2e:prod <spec>` |
| Web style-token check | `pnpm lint:style-tokens` (local gate; CI does not run it) |
| Benchmarks | `cargo run -p rove-bench -- <suite>` |
| Local full-stack smoke | `scripts/integration-smoke.ps1` |
| Hosted-bundle acceptance | `scripts/serve-acceptance.ps1` — builds `apps/web/web-dist`, runs `rove-api --web-dist` against an isolated scratch state root, and drives the `real-api.spec.ts` Playwright suite against it (`-SkipWebBuild`/`-SkipCargoBuild` to reuse artifacts, `ROVE_SERVE_ACCEPTANCE_ADDR` to move the bind address) |
| Aggregated acceptance | `scripts/product-acceptance.ps1` or `.sh`; writes `PRODUCT_ACCEPTANCE_REPORT.json`, never edit by hand. A completed run's report plus a hand-written record land under `evidence/acceptance/<date>-<sha>/` (see `evidence/README.md`) |
| Real-provider integration | `scripts/provider-integration.ps1 -Provider <type> -Model <id> -ApiBase <url> -ApiKeyEnv <VAR>` |
| State migration smoke | `scripts/state-migration-smoke.ps1` |
| TUI PTY smoke (Unix) | `python scripts/tui-pty-smoke.py --run` |
| Fix MSVC linker env | In PowerShell, `. scripts/msvc-env.ps1` first, then run cargo (use when cargo reports missing `link.exe` or link failures) |

When compiles are slow: use `cargo check -p <crate>` and targeted tests day to day instead of `--workspace`; desktop (Tauri) makes `--workspace` noticeably slower. Use `cargo clean` to reclaim space when `target/` grows large.

## Test strategy

- Unit tests: `#[cfg(test)]` in each crate's sources, testing per-module invariants.
- Per-package integration tests: `runtime/tests/` (tools, state, MCP, hooks, memory contracts), `apps/bootstrap/tests/` (state migration, etc.).
- Cross-package contracts: `tests/` (`rove-integration-tests`). Most modules (`e2e`, `mcp`, `recovery`, `tool_safety`, `event_contract`, `workspace_architecture`, etc.) share a single `it` binary (`tests/it.rs`); shared helpers live in `tests/support/`. Only tests that need exclusive process-global state get their own binary: `api` (mutates env vars) and `secret_authority_unarmed` (requires the secret registry to have never been activated). New test files go into `tests/it.rs`; do not add new `[[test]]` targets.
- Web: `vitest` unit tests colocated with sources (`*.test.ts(x)`); Playwright in `apps/web/tests/e2e/`, mostly using `product-api-mock.ts` to mock the backend, while `real-api.spec.ts` is the `local-full` scenario against a real API.
- Gates that must be enabled explicitly — a skipped run only proves the "skipped" path, not interop:
  - `ROVE_LOCAL_STRESS=1`: `--test it stress::`, local stress and soak
  - `ROVE_MCP_MEMORY_SMOKE=1` / `ROVE_MCP_FILESYSTEM_SMOKE=1`: real MCP servers
  - Real providers: `provider_smoke` tests and `scripts/provider-integration.ps1`, credentials read from `.env`
  - `ROVE_TUI_REAL_USE_GATE`: installed-TUI real-use gate
- Changes to safety or approval behavior must add negative tests. Tests must never point at production services.

## Regression checklist

Before committing:

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] Tests pass for affected crates; run `cargo test --workspace` when shared boundaries changed (events, state, approval, MCP, providers, artifacts)
- [ ] Web changed: `pnpm test`, `pnpm typecheck`, `pnpm build`; also `pnpm test:e2e` when browser-visible flows, SSE, approval/input/cancel/resume, or the API proxy changed
- [ ] API changed: utoipa annotations, `apps/api/openapi.json` snapshot, `pnpm check:api-types` (generated `apps/web/generated/api-types.ts`), and both implementations are in sync
- [ ] Schema changed: a new schema migration exists and an old database upgrades cleanly
- [ ] UI changed: loading, empty, and error states were all checked
- [ ] Touching tools, API, providers, state, MCP, artifacts, or Web: input size/path/timeout/concurrency have limits; untrusted content cannot become instructions or permissions; secrets are sanitized; approvals sit at the right boundary; retries are safe for side effects; recovery does not replay completed work
- [ ] Relevant docs updated (see the doc update rules in AGENTS.md)

## Git process

- Never commit directly to `main`; all changes (including docs) go through PRs.
- Branch from latest `origin/main` under `.worktrees/<topic>`, named `<type>/<topic>`:

  ```bash
  git fetch origin main
  git worktree add .worktrees/<topic> -b feature/<topic> origin/main
  ```

- Commit message format `type(scope): subject`, where type is `feat|fix|docs|refactor|test|build|ci|chore` and scope is `runtime|api|web|cli|...`. The subject states the observable outcome; one commit does one thing.
- One PR delivers one independently verifiable capability; describe scope and non-goals in the body (template `.github/PULL_REQUEST_TEMPLATE.md`). Merge with rebase merge — it is the only merge method enabled on the repository and keeps main linear; a PR's commits must therefore be split by logical step, not left as WIP/fixup noise. If a branch accumulated cleanup commits, squash them into place with `git rebase -i main` before pushing — whatever remains lands verbatim on main.
- Review: small changes may be self-reviewed; safety-related changes, shared runtime boundaries, or cross-package behavior with uncertainty require a blind review by a fresh session — give it the requirements, acceptance criteria, and repo location, but not the implementation approach.
- Worktree Rust builds share the root `target/` directory instead of growing
  a multi-GB build cache per worktree. The mechanism is an untracked
  `.cargo/config.toml` at the repository root (`build.target-dir = "target"`,
  resolved relative to the root): Cargo's ancestor config search reaches it
  from any directory inside the repository, including `.worktrees/<topic>`.
  Keep that file untracked — a committed copy inside each worktree would
  resolve to a per-worktree `target/` and win as the closer config. On a fresh
  machine, recreate it or set `CARGO_TARGET_DIR` to the root `target/`.

- Immediately after merging, clean up the branch and worktree:

  ```bash
  git worktree remove --force .worktrees/<topic>
  git branch -d feature/<topic>
  git push origin --delete feature/<topic>
  ```

- Code comments are omitted by default; write one only when the "why" is not obvious. Code, comments, and commit messages are in English.

## Release

- CI (`.github/workflows/ci.yml`, ubuntu): on PRs and pushes to main, runs Rust fmt/clippy/test plus Web test/typecheck/build; pushes to ordinary branches do not trigger, avoiding double runs per PR. Rust uses `Swatinem/rust-cache` for the registry and `target/`; routine clippy/test exclude `rove-desktop`, which is covered separately by `.github/workflows/desktop.yml` when `apps/desktop`, the root `Cargo.toml`/`Cargo.lock`, or the workflow itself changes (with the WebKit dependency install). The routine job installs `libwayland-dev` + `wayland-protocols` because `rove-api`'s `rfd` dependency builds `wayland-sys` crates through its ashpd backend; full Tauri/WebKit stays desktop-only.
- Release gate (`.github/workflows/release-gate.yml`, manual trigger, windows): deterministic `local-full` integration; also runs the real-provider gate when credentials are configured.
- Desktop packaging: in `apps/desktop`, `pnpm dlx @tauri-apps/cli@2 build --target x86_64-pc-windows-msvc`, producing MSI and NSIS. The Web static bundle comes from `pnpm build:desktop` in `apps/web`.
- Not yet done: code signing, a formal release channel, macOS/Linux packages. Rollback means reinstalling the previous installer; the user data directory is not removed on uninstall.
