# Post-Coding-Tool V2 Master Program

> Status: **Implemented on `program/full-delivery`; final acceptance and PR review pending**
>
> Coordinator branch: `main`
>
> Delivery worktree: `.worktrees/full-delivery`
>
> Starting implementation baseline: `3fbdcab` (`fix(runtime): harden coding
> tool mutations`)
>
> Execution brief:
> [`2026-08-07-post-coding-tool-v2-full-delivery.md`](2026-08-07-post-coding-tool-v2-full-delivery.md)

## 1. Decision Revision

The initially published two-worker topology was superseded before either worker
performed implementation. The program now uses one long-running delivery
conversation, one feature branch, and one worktree to complete every in-scope
checkpoint sequentially. This removes cross-branch contract guessing across
Core, Runtime, MCP, events, persistence, API/Web, evaluation, and Desktop.

The retired briefs remain as explicit historical records:

- [`2026-08-07-runtime-intelligence-program.md`](2026-08-07-runtime-intelligence-program.md)
- [`2026-08-07-protocol-platform-program.md`](2026-08-07-protocol-platform-program.md)

They must not be used to start work.

## 2. Objective

Complete the active post-Coding-Tool V2 architecture in one dependency-ordered
delivery:

1. one shared Agent kernel and extension plane;
2. model-on-ambiguity evaluation, independent finalization, multidimensional
   budgets, reconciliation, persistence, and product surfaces;
3. MCP shared dispatch, Streamable HTTP, rich results, Tool Artifacts,
   capability refresh, and diagnostics;
4. versioned AgentDefinition packages, immutable runtime profiles, Runtime
   `AGENTS.md` discovery, and typed procedural knowledge;
5. procedure-aware lifecycle integration and the deterministic OnCall reference
   Agent/evaluation suite;
6. a designed, implemented, and verified Tauri Desktop host that reuses the
   existing runtime, API, ProductStore, canonical events, and Web UI;
7. bounded external-provider, real-MCP, browser, and platform evidence when the
   required non-production services and credentials are available.

Current behavior remains documented under `docs/runtime/`. Target details stay
in active design documents until their implementation checkpoint passes.

## 3. Starting Contract

The base already provides:

- a modular Rust Workspace and provider-neutral model protocol;
- `rove-core::Agent` and shared model/tool contracts, while durable Runtime
  still retains part of the multi-turn coordinator;
- bounded StepRunner execution, append-only StepRecord facts, immutable
  PlanRevision chains, and deterministic rule-first decisions;
- persistent state, artifacts, resume, ProductStore, canonical events, CLI,
  API, Web, TUI, and deterministic benchmarks;
- persistent Project Trust, Runtime-owned Execution Environment, authoritative
  Tool Schemas, immutable Capability Snapshots, and Coding Tool V2;
- MCP stdio and legacy SSE with the current text-oriented ToolOutput contract;
- a complete browser product shell. The Tauri `apps/desktop` host is now
  implemented on `program/full-delivery` and is not yet merged to `main`.

This section records the starting contract. Current implementation truth is in
code, tests, and `docs/runtime/`; the delivery branch now implements the program
outcomes, while optional external/cross-platform evidence remains unclaimed.

## 4. Finite Scope

The program definition of done includes every outcome in Section 2. It does not
silently expand to include:

- built-in vector RAG or a replacement for tool-based workspace retrieval;
- generic tool dependency-DAG or Shell write-set inference;
- reconstruction of live human approval/input channels after process restart;
- agent-controlled browser/Desktop automation workspace kinds;
- native PTY beyond the current typed unsupported capability;
- production-autonomy or exactly-once remote-mutation claims.

These remain explicit optional/future boundaries unless a later coordinator
revision promotes them with a separate contract and acceptance gate.

## 5. Repository Topology

The root worktree stays on `main` and is used only by the coordinator for plan
maintenance, final review, targeted corrections, merge, and release evidence.

The complete implementation runs in:

| Worktree | Branch | Brief |
|---|---|---|
| `.worktrees/full-delivery` | `program/full-delivery` | [`2026-08-07-post-coding-tool-v2-full-delivery.md`](2026-08-07-post-coding-tool-v2-full-delivery.md) |

Do not create additional worktrees or feature branches for this program. The
single delivery branch preserves checkpoint commits and pushes them as durable
review evidence, but it does not stop for coordinator integration between
checkpoints.

## 6. Execution Doctrine

The delivery conversation must implement the program, not merely plan or
scaffold it. For each checkpoint it must:

1. inspect current source, tests, schemas, and runtime documentation;
2. define the exact compatibility and security contract from evidence;
3. implement the complete checkpoint without placeholders or knowingly fake
   behavior;
4. add success, failure, boundary, migration, and regression coverage;
5. run the focused and required broad gates with real process exits;
6. review its own diff for correctness, duplication, secret exposure, unsafe
   authority, stale docs, and unrelated churn;
7. update current documentation only for reproducibly implemented behavior;
8. create and push a coherent checkpoint commit;
9. continue automatically to the next checkpoint only after every required
   condition is satisfied.

A failing, skipped-required, flaky, or incomplete gate blocks progression. The
worker fixes the issue rather than weakening checks, deleting tests, marking an
acceptance item Met, or deferring an in-scope obligation. Optional external
gates may be recorded as not run only when credentials/services are genuinely
unavailable and deterministic coverage is complete.

## 7. Dependency Order

The single delivery branch follows this order:

1. baseline characterization and program guard tests;
2. one shared Agent kernel and extension plane;
3. lifecycle Evaluator, Finalizer, budgets, reconciliation, and surfaces;
4. MCP typed foundation, shared dispatcher, identity/safety/pagination, and
   Streamable HTTP;
5. shared rich result, ArtifactRef, event, persistence, and compatibility
   contracts;
6. AgentDefinition, immutable profiles, Runtime `AGENTS.md`, procedure schema,
   catalog, selection, and hydration;
7. rich MCP mapping, durable Tool Artifacts, capability refresh, persistence,
   resume, and product diagnostics;
8. procedure/lifecycle integration and the complete deterministic OnCall suite;
9. Desktop D0, Tauri implementation, packaging, and platform evidence;
10. integrated program acceptance, current-document reconciliation, branch
    push, and pull request creation.

The detailed exit gate for every checkpoint is in the execution brief. A later
checkpoint cannot be used to hide an incomplete earlier one.

## 8. Quality and Security Gates

Every checkpoint preserves repository invariants and includes:

- focused tests first, then formatting, workspace Clippy with `-D warnings`,
  and proportional workspace tests;
- Web test/typecheck/build and browser E2E for browser-visible contracts;
- additive defaults, migrations, and old-artifact tests for serialized changes;
- negative tests for approval, paths, URLs, redirects, secrets, retries,
  cancellation, resume, MIME/resource handling, and untrusted instructions;
- deterministic local execution without credentials or network;
- bounded input, output, concurrency, timeout, storage, retry, and history;
- current/proposed documentation boundaries and a clean worktree after commit.

No pass may be claimed from prose, a skipped real-service gate, or a generated
report without a real process exit code.

## 9. Pull Request Gate

The delivery branch may create a pull request to `main` only after all ten
dependency-order items are complete and the final integrated gate passes.

The pull request must include:

- exact base and head SHAs plus the checkpoint commit list;
- implemented contracts and compatibility/migration behavior;
- security review and negative-test evidence;
- every required verification command with its real result;
- optional external gates explicitly marked passed, failed, or not run;
- known residual risks limited to declared out-of-scope or environment-bound
  evidence;
- final clean `git status` and confirmation that generated state is uncommitted.

The delivery conversation may push the branch and open the PR. It must not
merge, squash, rebase, or close the PR. Final review and merge remain owned by
the coordinator conversation.

## 10. Program Completion

The program is complete only when:

1. every in-scope checkpoint exists in code and tests on the delivery branch;
2. there is one kernel, event lifecycle, durable truth, safety path, artifact
   authority, and product session truth;
3. deterministic, compatibility, migration, security, Web, benchmark, and
   Desktop gates pass from the final branch state;
4. optional external evidence is reported honestly;
5. current documentation agrees with reproducible behavior;
6. the branch is pushed, the worktree is clean, and a complete PR is open;
7. the coordinator independently reviews and merges the PR to `main`.
