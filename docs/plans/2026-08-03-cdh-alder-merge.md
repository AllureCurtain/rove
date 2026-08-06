# CDH Alder Merge — Control Capabilities Completion

> Status: **Completed / Merged** — G1-G7 landed through PR #29 at
> `f9e88a7553bcc7561550e5b8286c320108c8fd51` on 2026-08-06
> Delivery branch: `feature/cdh-control-completion`, developed in the historical
> `.worktrees/cdh-merge` checkout from `main @ 3aa51a1`
> Process log: repo-root `CDH_IMPLEMENTATION_LOG.md` (historical delivery record;
> do not copy alder delivery docs)
> Reference only (untrusted): `D:\Study\cc\claw\code\CODING_TASK_36_ROVE\res\alder\workspace\rove`
> Task guide: `D:\Study\cc\claw\code\CODING_TASK_36_ROVE\CODING_TASK_GUIDE-36-ROVE.md`

## 1. Outcome

Complete the Control / evidence / settings product surface that Web Complete left open, by:

1. reviewing the cloud (alder) candidate file-by-file,
2. porting only correct, convention-matching pieces,
3. rewriting hollow or contradictory behavior until it matches the task guide,
4. verifying each group with real API + UI + tests before moving on.

**Desktop (Group 8) is out of scope for this plan.** Do not implement Tauri host work here.

## 2. Fixed delivery rules

- During implementation, work stayed in `.worktrees/cdh-merge`; that checkout is
  now retired and must not be used as a future implementation base.
- One group In Progress at a time; finish server contract + persistence + UI + real tests before the next group.
- Schema migrations 5→6→7→… land in order and never renumber.
- Alder root docs (`SUMMARY.md`, `VERIFICATION.md`, etc.) are **not** copied into this repo.
- Process truth lives in `CDH_IMPLEMENTATION_LOG.md`; this plan is the checklist.
- After each group: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, focused `cargo test`, and `pnpm test` / `typecheck` / `build` under `apps/web`.
- Failures stay in the log; never delete them. Unrun checks are `Not Run` with reason.

## 3. Group order and done criteria

### G1 — Steer + Follow-up (Done)

| | |
|---|---|
| **Goal** | Mid-run steer at safe points; durable follow-up queue with crash-safe server drain; real revoke; UI reads events/store. |
| **Port from alder** | `runtime/engine/control.rs`, safe-point drain in run/plan loops, StreamEvent variants, controls table design, create/list/revoke routes, Web client types, Composer buttons. |
| **Must rewrite / add** | Supervisor **must** call `list_pending_followups`, CAS-claim, start next run, emit `FollowupQueued/Dequeued/Abandoned`; restart drain without double-start; abandon on non-final; queue UI + revoke; idle client must **not** fake drain with a second `send`. |
| **Done** | Store tests (idempotency, CAS, abandon); API/integration path for steer during generate/tool-wait; follow-up Final→new run and crash between complete/claim/start; Web steer/follow-up/revoke against real API. |

### G2 — Session Fork + branch tree (Done)

| | |
|---|---|
| **Goal** | Fork only at terminal run boundaries; child inherits read-only history; independent continuation; branch tree UI. |
| **Port from alder** | forks table, parent columns, idempotent create/list API, basic Fork button. |
| **Must rewrite / add** | Terminal-boundary validation; reject active/corrupt sources; inherited transcript projection (no writable copy of parent events); branch tree + switch; delete parent keeps child relation; fix any PRODUCT_BEHAVIOR-style claims to match code. |
| **Done** | Store + API tests for reject/idempotent/terminal; UI shows parent/child and fork point after refresh. |

### G3 — Session model & reasoning (Done)

| | |
|---|---|
| **Goal** | Session-scoped model/reasoning with revision CAS; server resolves profile/key/model; mid-run immutable; UI wired. |
| **Port from alder** | model_config table, run_models history, GET/PUT routes, job assembly read. |
| **Must rewrite / add** | Composer / QuickModelControl → session config; conflict reload; unsupported reasoning disabled with reason; “applies from next run” messaging. |
| **Done** | CAS conflict test; mid-run change does not alter active run; UI round-trip on real API. |

### G4 — Usage / context / cost (Done)

| | |
|---|---|
| **Goal** | Real token aggregates; versioned price snapshots per run; context occupancy; no fake zero cost. |
| **Port from alder** | usage aggregation module, inspector usage section. |
| **Must rewrite / add** | Per-run pricing snapshot (source/version/currency); context window + estimate/compaction; Fork/resume no double-count tests. |
| **Done** | Unit + session usage API tests; UI shows unavailable when unpriced. |

### G5 — Files / artifacts / images / Diff (Done)

| | |
|---|---|
| **Goal** | Safe workspace file browse/read; artifact manifest + download/preview; image limits; run+git diff. |
| **Port from alder** | files.rs, diff.rs, FilesPanel/DiffPanel. |
| **Must rewrite / add** | Artifact binary stream/preview; image format/byte/pixel limits; SVG/HTML not executed; path/symlink/secret tests. |
| **Done** | Traversal/secret rejection tests; artifact fetch; UI panels on real API. |

### G6 — Evidence export (Done)

| | |
|---|---|
| **Goal** | Machine JSON + offline-readable HTML and/or Markdown; redacted; complete control/fork/usage fields. |
| **Port from alder** | export.rs JSON+HTML + redaction walker. |
| **Must rewrite / add** | Ensure steer/follow-up/fork/usage in payload; Markdown if required by guide; secret canary test. |
| **Done** | Export tests with canary secrets scrubbed; download from UI. |

### G7 — Settings gap-fill (Done; main Settings is baseline)

| | |
|---|---|
| **Goal** | Close remaining settings holes without regressing Web Complete C2. |
| **Port from alder** | Only MCP pieces that add real value beyond main. |
| **Must rewrite / add** | Prefer main SettingsShell; real MCP test classification where feasible; provider test failure reasons; no dual-track platform APIs. |
| **Done** | No regression of existing settings tests; any new MCP/provider paths covered. |

### G8 — Desktop

**Out of scope.** Do not implement in this worktree/plan. Leave for a later Desktop program.

### Hardening + acceptance (after G1–G7) — Done

- **Done.** Concurrent multi-workspace steer/follow-up/fork/model operations (`product_concurrent_multi_workspace_control_operations_stay_isolated_and_serialized`); SSE drop mid-flight (`api_sse_stream_dropped_mid_flight_loses_no_events_on_reconnect`); API restart (five `api_restart_*` / `api_startup_*` tests).
- **Done.** Long session / deep tree / large dir / large diff smoke (`product_long_session_deep_tree_large_dir_and_large_diff_stay_bounded`), plus the directory-scan bound at unit level in `apps/api/src/product/files.rs`.
- **Done.** PowerShell + POSIX acceptance entry writing real `PRODUCT_ACCEPTANCE_REPORT.json` (no fake PASS).

## 4. Explicit non-goals

- Redesigning Product UI V2 information architecture or palette.
- Copying alder delivery documentation into the product docs tree.
- Shipping Tauri / Windows installer in this plan.
- Claiming PASS without executed commands and evidence.

## 5. Historical worktree / branch

```text
main @ 3aa51a1
  `-- feature/cdh-control-completion  (.worktrees/cdh-merge)
        `-- PR #29 -> main @ f9e88a7
```

The CDH and `.worktrees/web-control-complete` checkouts are retired. Neither is
an authorized base for post-CDH work.

## 6. Progress pointer

The completed step-by-step record is `CDH_IMPLEMENTATION_LOG.md` at the
repository root. Future work follows the two independent implementation briefs
`docs/plans/2026-08-06-kernel-message-provider-implementation.md` and
`docs/plans/2026-08-06-project-trust-execution-tools-implementation.md` from a
fresh, synchronized `main` baseline.
