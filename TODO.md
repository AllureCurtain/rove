# TODO

Updated: 2026-11-14

## In progress

- **Desktop pilot** (deferred per owner): Desktop (Tauri 2) still has unfinished surfaces — signed/clean-machine packaging, native folder selection on all OSes, Linux/Windows packaging, Windows ConPTY automation, code signing. Deferred to a later pass; the Web console pilot covers the runtime contract Desktop shares. The universality plan (capability parity, three-OS CI build matrix, per-OS acceptance matrix, suggested order) is in `docs/desktop-plan.md`.
- ~~**Second real-project pilot**~~ done: scratch-repo matrix all PASS (evidence `evidence/pilot/2026-11-14-pilot2-*`, summary `evidence/pilot/PILOT2_REPORT.md`). Follow-up candidate surfaced: fork at a mid-history `done` run is refused with `product_fork_source_invalid` (binding-identity mismatch) — needs a repro test to decide defect vs correct strictness.

## Next up

Ordered by priority.

1. ~~**Stranded-queue auto-recovery design**~~ recorded (PR #89): auto-confirm stays off — a requeue-exhausted turn already proved the transient window insufficient, and direct abandons are persistent failures where retrying deterministically fails. Recover remains the explicit consent path; a directory-level recover-all shortcut is the open follow-up if friction remains.
2. ~~**Issue #30 — plan approval bar**~~ done (PRs #87 + #88 merged). `PlanApprovalProvider` gate emits `plan_approval_needed`/`plan_approval_resolved` on the first draft of each product-session turn under `Ask` (follow-ups re-draft and re-gate); `POST /jobs/{id}/plan-approvals/{id}` resolves with an approve-mode (which becomes the run's tool-approval policy) or reject; `pending_plan_approvals` on job state; `PlanApprovalBar` above the composer splits approve-mode + reject and persists the mode to `default_approval_policy`.
3. ~~**TUI history follow-up**~~ done. Subagent events now project into saved history without partial (PR #92); display-budget paging recorded as a no (PR #93) — the bound protects memory on long real-provider runs whose folded deltas are already deduplicated by the timeline.
4. ~~**F.5 TUI part**~~ done (PR #90 merged). TUI launch now drains a queued successor the same way the boundary does: after the stranded-claim redrive, it claims the oldest `queued` message and starts the reserved run. `claim_next_tui_successor` shares the stranded path's provider gate so a queue survives onboarding. `needs_attention` stays explicit — queued delivery ≠ auto-recovery.

## Known local-machine issues (not CI)

- `state::index` `external_commit_guard_*` tests fail locally on macOS (`/var`↔`/private/var` SQLite canonicalization); CI on Linux is green. Pre-existing environment flake, not a code defect.
- `apps/desktop/gen/schemas/macOS-schema.json` is untracked but should remain so — sibling schemas are tracked, this one is generated-per-OS; do not `git add -A`.
- Playwright pinned `chromium_headless_shell-1223` missing on this machine + CDN unreachable via local proxy; `1248` is symlinked in as a fallback. e2e runs that need exact browser build should re-run on a machine with the correct pinned shell.

## Blocked

- External interop gates (require credentials or an external environment; run by the repository owner):
  - Credentialed external provider gates on Web and Desktop
  - Real third-party MCP interop (the official filesystem server is rejected for its `anyOf` output schema)
  - Windows ConPTY automation
  - Signed/clean-machine macOS packages and Linux packaging
  - Code signing
  - Full installed Desktop flow
- Add a Windows Defender exclusion: requires administrator rights, handled by the repository owner
- ~~cargo in local Git Bash picks up Git's bundled `/usr/bin/link.exe`, and `vswhere` cannot find the Visual Studio installation~~ Fixed on 2026-10-01 (removed the stale `CachePath` registry override + rebuilt the VS instance directory; see `scripts/fix-vs-registration.ps1` for the process and prerequisites). `vswhere` enumeration is still incomplete, but rustc's MSVC discovery chain is restored

## Recently completed

Keep only the last 10 entries.

- 2026-11-14 Real-provider benchmark suite `benchmarks/coding-agent-live` verified end-to-end on `deepseek-flash` (PR TBD): a `ModelClientFactory` injected at `BenchmarkRunOptions` replaces the hardwired `FakeModelClient`, provider resolved through the same `default_wire_protocol_registry` + credential-from-`auth_env` as the product; per-scenario `oracles_mode: "capability"` drops transcript-pinned oracles for behavioural ones (new `workspace_file_contains` kind). All 5 scenarios PASS (K04 = real cancel+resume with no replayed mutation; K05 = rejected write with `tool_mutations: []`). ~24–44k tokens/case. Deterministic `fake_contract` suite unchanged and still green; live suite is not a CI gate. Defect fixed in-flight: `max_steps` 8→24 for exploratory real models.
- 2026-11-14 Real-provider engineering pilot (P0): supervised `deepseek-flash`/`v4-pro` trial of the Web console via ego-lite. 12-row task matrix all PASS on `main` — understand code, bounded change (regression-replay = real upstream `31cbfdb`), approve/deny, continue+queue, cancel, refresh/restart recovery, duplicate-launch refusal, byte-exact inspect+revert, workspace+fork isolation, long history, typed failure paths. Exposed 7 defects; 4 fixed in-flight (`rove_bin` path `3deb4cd`, per-step budget `717b156`/`6693bbb`, context 128k `5190b6b`, verify-key retention `42b1b44`), 1 open (issue #80), 1 mitigated (stranded-queue PR #83), 1 noted (model behavior). Summary `evidence/pilot/PILOT_REPORT.md` (PR #81); per-run `PILOT.md` pre-registered→actual→verify→verdict.
- 2026-11-14 Transcript projection no longer drops tool-call cards at ~2,000 events (issue #80, PR #82): `MAX_SNAPSHOT_EVENTS` 2000→50000 + reader `MAX_EVENTS_PER_RUN`/`MAX_TOTAL_EVENTS` 20000→50000/25000→60000. Verified real-provider: a 4042-event `deepseek-flash` turn projected whole with `partial_reasons` empty and the `edit_file` rollback card at seq 4040-4042 (PR #84 evidence).
- 2026-11-14 Stranded-queue `needs_attention` deadlock mitigated (PR #83): successor start redrive window ~7s→~30s (4→6 attempts, backoff 1s/2s/4s/8s/15s). Transient provider/workspace failures self-heal inside the window; persistent misconfiguration still escalates and the confirm/Recover path is unchanged.
- 2026-11-14 Real-model usability baseline: context token defaults raised to `soft 96_000`/`hard 128_000`/`reserved 16_000` (was 24k/30k/4k, Fake-model-sized); `[runtime.execution]` now parses in user config + 12 `ROVE_EXECUTION_*` env vars; `COMPACTION_REQUEST_MAX_BYTES` follows the new window. (PR #78)
- 2026-11-14 Per-step model-turn budget for exploratory models: `DEFAULT_MAX_MODEL_TURNS_PER_STEP` 4→16 (`717b156`) then →48 (`6693bbb`) after s3 showed 16 still starved an exploratory step; decisions.md records rationale + `[runtime.execution]` override path. Provider "Verify and save" no longer wipes the pasted key on failed verification (`42b1b44`, PR #77).
- 2026-11-14 `rove_bin()` test infra fixed (`3deb4cd`, PR #76): reads the CLI path from `cargo build --message-format=json` artifact JSON instead of `current_exe`, so `cli_repl`/`cli_review` integration tests pass under the shared/scratch target layout. +4 clippy `needless_bool`/`collapsible_if` fixes for CI's newer clippy.
- 2026-11-14 Documentation + benchmark suite: `docs/system-overview.md` + `docs/architecture.svg` + richer README (PR #74); `benchmarks/coding-agent` v2 deterministic suite — 5 scenarios (read-edit-verify, search-read-edit, failure-recovery, cancel-resume, mutation-rejected) on `fake_contract`, validated green (PR #75).
- 2026-11-10 TUI saved history now restores the selected run and provides F2 read-only browsing across earlier/later runs in the same session. Corrupt, omitted, conflicting and display-limited records stay explicit, including after continuation. History never changes the resume target or reactivates approvals. Schema migration 6 adds the ordered session index. CLI 193 tests, 32 runtime index tests, the runtime state contract, targeted clippy/fmt and fresh blind review passed. A real Unix PTY with offline fixtures verified restart display, older/newer navigation, same-session continuation (three runs total) and clean exit. Initial harness failures from diff-screen matching and premature exit observation are retained locally; no external-provider or full workspace validation claim.
- 2026-11-10 TUI resume selection now switches the visible queue and model with the session and clears the previous conversation. Failed target loads preserve the original session; selecting a session does not execute queued work. Pending explicit model choices survive switching away and back without leaking to another session or reviving after consumption. The empty view explains that older transcript loading remains unavailable. A regression failed on the original behavior; 185 CLI tests, CLI clippy, fmt and fresh independent review passed. No local Web/Desktop or full workspace rerun; real terminal and external-provider interaction not verified here.
- 2026-11-09 API host ownership now precedes recovery, including Desktop startup and legacy migration. Duplicate processes cannot interrupt a live pending approval; OS process death releases ownership, and cancelled SQLite waiters retain it until their workers finish. Concurrent first-launch token publication is atomic. Fresh blind review, 228 API unit tests, 201 API integration tests, 21 Desktop tests, fmt/clippy and a native debug build passed. Real-process duplication, pending-approval preservation and crash/restart recovery passed. Native duplicate Desktop startup was refused; visual confirmation of its system-hosted alert remains unverified because the UI tool cannot access UserNotificationCenter. No frontend or clean-install validation claim for this backend-only build.
