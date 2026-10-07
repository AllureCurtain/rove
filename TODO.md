# TODO

Updated: 2026-10-07

## In progress

- **External-control productization** (PR series): PR 1 (`rove-api --web-dist` one-origin console + `scripts/serve.ps1` + `/health`), PR 2 (loopback-only `POST /product/provider-onboarding` + browser paste-key form), PR 3 (`evidence/` directory + first committed acceptance run), the interim fix `d2c4b57` (legacy trust-store size bound no longer applied to the live sqlite store), PR 4 (`StreamEvent` payload graph in OpenAPI), PR 5 (`openapi-typescript` generated types + retired response validators + mock↔spec contract test), the follow-up `1482cdc` (router composition on a big-stack thread), PR 6 (`scripts/serve-acceptance.ps1` drives the real-API Playwright suite against the `rove-api --web-dist` hosted bundle as the required `web-serve-e2e` acceptance check; caught two contract bugs — `providers/test` status `pass` vs UI's `ok`, and attaching to an already-evicted live job reported as restore failure instead of terminal), and the mock-alignment fix `eb908e7` (PR #21) merged; second committed acceptance run at `evidence/acceptance/2026-10-07-eb908e7/` (PASS, 12/13, gated check not run)
- **Console chrome pass** on `feat/console-chrome`: 28px ghost attach button, compact model popover, reading band widenable past the 840 default up to the pane's usable width, assistant turns verified cardless across both skins

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
