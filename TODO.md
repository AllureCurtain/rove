# TODO

Updated: 2026-10-31

## In progress

- **Subagent delegation** (issue #31, Fork A — inline subagent in the parent run, 5-stage PR series): PR 1 (contract) adds `StreamEvent::SubagentEvent { parent_call_id, event }` at contract version 4 — a wrapped envelope attributing every child event to the parent `delegate`/`task` call; OpenAPI snapshot, generated web types, `STREAM_EVENT_NAMES`, and `tests/event_contract.rs` updated in lockstep; OpenAPI field uses a `$ref` (`schema_with`) to bound the recursive schema. Remaining: PR 2 runtime `delegate` tool + `ToolContext` sub-run spawner, PR 3 API per-subagent stop, PR 4 web projection (`parent`/`nesting` reducer arms + `waiting-for-subagents`), PR 5 web UI (transcript topology card + `subagents` work-panel tab).
- **External-control productization** (PR series): PR 1 (`rove-api --web-dist` one-origin console + `scripts/serve.ps1` + `/health`), PR 2 (loopback-only `POST /product/provider-onboarding` + browser paste-key form), PR 3 (`evidence/` directory + first committed acceptance run), the interim fix `d2c4b57` (legacy trust-store size bound no longer applied to the live sqlite store), PR 4 (`StreamEvent` payload graph in OpenAPI), PR 5 (`openapi-typescript` generated types + retired response validators + mock↔spec contract test), the follow-up `1482cdc` (router composition on a big-stack thread), PR 6 (`scripts/serve-acceptance.ps1` drives the real-API Playwright suite against the `rove-api --web-dist` hosted bundle as the required `web-serve-e2e` acceptance check; caught two contract bugs — `providers/test` status `pass` vs UI's `ok`, and attaching to an already-evicted live job reported as restore failure instead of terminal), the mock-alignment fix `eb908e7` (PR #21) merged, PR #24 (`fix/export-path-redaction-serve-acceptance`: POSIX `serve-acceptance.sh` mirror + `path_variants` `/private` toggle for macOS canonical workspace paths) merged; second committed acceptance run at `evidence/acceptance/2026-10-07-eb908e7/` (PASS, 12/13, gated check not run). A third acceptance run was executed on `aac86d3` on 2026-10-31 and produced FAIL (10/13) — test-api passed after the path-redaction fix, but `web-e2e` and `web-serve-e2e` both failed on a local-machine Playwright issue: the pinned `chromium_headless_shell-1223` binary is missing and the CDN is unreachable through the local proxy, so `1248` was symlinked in as a fallback and does not drive modifier-click, viewport-geometry, or scroll-anchor assertions correctly. Artifacts kept locally under `evidence/acceptance/2026-10-31-aac86d3/` (uncommitted); re-run on a machine with the correct pinned browser build to get a green record.
- **Console chrome pass** on `feat/console-chrome`: 28px ghost attach button, compact model popover, reading band widenable past the 840 default up to the pane's usable width, assistant turns verified cardless across both skins
- **Web console UI polish line** (issues #27, #28, #29, #32 — frontend only, `apps/web`): #27 tool cards merged (PR #33), #28 preview tab merged (PR #34), #29 turn-change cards merged (PR #36 — rollback action marked backend-pending on the issue; no product-facing revert endpoint exists); #32 polish batch on `feat/polish-batch` — minimap hover magnification + nearest-turn popover, context-ring click popover with token breakdown and the used/remaining toggle, first-run "Getting started" checklist on home (provider / workspace / first prompt)

## Next up

Ordered by priority.

1. F.4, TUI part: long sessions in the terminal can load earlier history on demand (CLI)
2. F.5, TUI part: queued successor messages resume delivery after a TUI restart (CLI)
3. Generate Web API types from `apps/api/openapi.json` instead of the hand-written `product-api-types.ts` and `rove-types.ts` (frontend)

## Blocked

- External interop gates (require credentials or an external environment; run by the repository owner):
  - Credentialed external provider gates on Web and Desktop
  - Real third-party MCP interop (the official filesystem server is rejected for its `anyOf` output schema)
  - Windows ConPTY automation
  - macOS/Linux packaging
  - Code signing
  - Full installed Desktop flow
- Add a Windows Defender exclusion: requires administrator rights, handled by the repository owner
- ~~cargo in local Git Bash picks up Git's bundled `/usr/bin/link.exe`, and `vswhere` cannot find the Visual Studio installation~~ Fixed on 2026-10-01 (removed the stale `CachePath` registry override + rebuilt the VS instance directory; see `scripts/fix-vs-registration.ps1` for the process and prerequisites). `vswhere` enumeration is still incomplete, but rustc's MSVC discovery chain is restored

## Recently completed

Keep only the last 10 entries.

- 2026-10-31 PR #24 merged: POSIX `serve-acceptance.sh` (mirrors the .ps1) + `path_variants` now emits the `/private`-toggled spelling for macOS canonical workspace paths — fixes the real export-redaction leak that was failing `test-api` on macOS
- 2026-10-31 Web-console real-usability pass against a live DeepSeek provider: reasoning streams end-to-end (`reasoning_effort` on the Chat Completions wire → `reasoning_content` → `thinking_delta`, contract v3) into a collapsible thinking block; onboarding surfaces a per-workspace `provider_credentials` grant prompt; stranded queued successors get a one-click Recover session row with auto-drain; `StreamEvent`, OpenAPI snapshot, generated web types, and copy updated together (935 web tests + event-contract tests green)
- 2026-10-06 Web console rebuild per `docs/web-console-design.md` v2 complete, one PR per phase: P0 skin and shell (PR #4), P1 conversation surface (PR #5), P2 session rail (PR #6), P3 work panel (PR #7), P4 polish (PR #8 — TodoDock composer fold on `runState.plan`, proportional minimap with viewport band, copy/inline-style cleanup, `--z-*` ladder and same-hue `--cp-elev-*` elevation channels)
- 2026-10-05 Promoted `docs/web-console-design.md` to v2 (active implementation contract): added the graphite skin, shell/sidebar/composer/work-panel specs, and the P0–P4 phase plan; kept the verified defect list as acceptance criteria
- 2026-10-05 CI: concurrency groups cancel superseded runs per ref; rust job installs libwayland-dev + wayland-protocols for rfd/ashpd
- 2026-10-04 GitHub-facing cleanup for open-sourcing: renamed the env files to the standard .env.example/.env pair, standardized on rebase merge (repo now rebase-only + auto-delete-branch), aligned the PR template with the coding-standard minimal skeleton, added SECURITY.md
- 2026-10-04 CI speedup: `Swatinem/rust-cache` caches the registry + `target/`; routine clippy/test exclude `rove-desktop`, which moved to a separate paths-gated workflow
- 2026-10-04 Open-source cleanup: removed the v1 skin runtime switch (`ROVE_PRODUCT_UI_VERSION`, `uiVersion`, `data-presentation`) and the `/dev/product-ui-v2` design mock; added MIT `LICENSE`; removed stale `.gitignore` entries
- 2026-10-03 Removed references to external projects and local machine paths (84 files): rewrote code comments as design rationale, neutralized example paths and identities, made two local scripts parameter-driven; added the Web console design draft `docs/web-console-design.md`
- 2026-10-02 Committed the OpenAPI snapshot `apps/api/openapi.json` (regenerate with `ROVE_UPDATE_OPENAPI=1`; a consistency test in `tests/api.rs` keeps it in sync with the utoipa annotations)
- 2026-10-01 Fixed local MSVC linker discovery: removed the stale registry `CachePath` override + rebuilt the VS instance directory; cargo now builds directly from Git Bash / pwsh / a clean PATH (`scripts/fix-vs-registration.ps1`)
- 2026-10-01 Added `scripts/msvc-env.ps1`: one-command loading of the VS build environment when cargo cannot find the MSVC linker (Git's GNU link.exe wins / vswhere cannot find the installation)
- 2026-10-01 Structural refactor phase 1 done: test build after a runtime change went from 146s to about 66s; phase 2 (splitting runtime) evaluated and deferred
- 2026-10-01 Extracted ProductStore into the `rove-product-store` crate; lib test unit time for api-only changes went from 26s to 10s
