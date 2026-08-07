# Post-Coding-Tool V2 Master Program

> Status: **Active coordinator program / Ready for Wave 1**
>
> Coordinator branch: `main`
>
> Worker worktrees: `.worktrees/runtime-intelligence` and
> `.worktrees/protocol-platform`
>
> Starting implementation baseline: `3fbdcab` (`fix(runtime): harden coding
> tool mutations`)

## 1. Objective

Complete the active post-Coding-Tool V2 architecture with no more than two
registered worker worktrees at any time. The program covers:

1. one shared Agent kernel and extension plane;
2. the remaining execution lifecycle: ambiguity evaluation, independent
   finalization, multidimensional budgets, reconciliation, and product
   surfaces;
3. versioned AgentDefinition packages, immutable runtime profiles, runtime
   `AGENTS.md` discovery, and typed procedural knowledge;
4. MCP Streamable HTTP, shared dispatch, rich result envelopes, Tool Artifacts,
   capability refresh, and diagnostics;
5. the deterministic OnCall reference Agent/evaluation suite;
6. a separately designed and verified Tauri Desktop host that reuses the
   existing runtime, API, ProductStore, canonical events, and Web UI;
7. bounded external-provider, real-MCP, browser, and platform evidence when the
   required credentials and services are available.

This plan is the coordinator source for sequencing and integration. Current
runtime behavior remains documented under `docs/runtime/`; target details
remain in the active design documents until each implementation slice lands.

## 2. Starting Contract

The baseline already has:

- a modular Rust Workspace and provider-neutral model protocol;
- `rove-core::Agent` plus shared model/tool contracts, while the durable Runtime
  still retains part of the multi-turn coordinator;
- bounded planned StepRunner execution, append-only StepRecord facts, immutable
  PlanRevision chains, and deterministic rule-first decisions;
- persistent state, artifacts, resume, ProductStore, canonical events, CLI,
  API, Web, TUI, and deterministic benchmarks;
- persistent Project Trust, a Runtime-owned Execution Environment,
  authoritative Tool Schemas, immutable Capability Snapshots, and Coding Tool
  V2;
- MCP stdio and legacy SSE with the current text-oriented ToolOutput contract;
- a complete browser product shell, but no Tauri `apps/desktop` host.

The implemented baseline does not yet provide the seven program outcomes in
Section 1. A future design is not implementation evidence.

## 3. Scope Boundary

The program definition of done includes the seven outcomes in Section 1. The
following remain outside this program unless the coordinator explicitly
promotes them in a later revision:

- built-in vector RAG or a replacement for tool-based workspace retrieval;
- generic tool dependency-DAG inference and Shell write-set inference;
- reconstruction of live human approval/input channels after process restart;
- agent-controlled browser/Desktop automation workspace kinds;
- native PTY support beyond the current typed unsupported capability;
- claims of production autonomy, exactly-once remote mutation, or provider/MCP
  interoperability without real evidence.

These exclusions keep the program finite. They must remain visible in current
status documents where relevant.

## 4. Repository and Worktree Topology

The coordinator uses the root `main` worktree. It owns:

- this master plan and both worker briefs;
- shared-contract decisions and small prerequisite commits that cannot safely
  be split;
- review, conflict resolution, full verification, merge order, documentation
  reconciliation, and pushes to `origin/main`;
- refreshing worker branches after every integration barrier.

Two worker worktrees are reused for the entire program:

| Worktree | Branch | Brief |
|---|---|---|
| `.worktrees/runtime-intelligence` | `program/runtime-intelligence` | [`2026-08-07-runtime-intelligence-program.md`](2026-08-07-runtime-intelligence-program.md) |
| `.worktrees/protocol-platform` | `program/protocol-platform` | [`2026-08-07-protocol-platform-program.md`](2026-08-07-protocol-platform-program.md) |

Do not create a third worker worktree. Keeping the same two conversations is
allowed, but their branches may not run past an integration barrier on a stale
base.

## 5. Integration Protocol

Each wave follows this protocol:

1. The coordinator supplies an exact `origin/main` base and active wave.
2. Both workers confirm a clean branch and execute only their active wave.
3. Workers commit coherent checkpoints, run their required gates, push their
   branch, and stop at the wave exit gate.
4. The coordinator reviews both diffs and real test exits, selects merge order,
   fixes shared issues separately when needed, and merges without rewriting
   worker history.
5. The coordinator runs proportional full gates, updates current documentation
   only for implemented behavior, and pushes `main`.
6. Both worker branches refresh from that exact new `main` before another wave
   starts.

Workers must not merge each other, merge their own branch into `main`, force
push, or silently edit the other lane's primary ownership. A required shared
hotspot change is reported to the coordinator with the smallest requested
contract and supporting test.

## 6. Delivery Waves

### Wave 0 - Program seal

Coordinator-only work:

- publish this master plan and both worker briefs;
- create both worker branches from the same pushed `main` commit;
- record the exact base in the startup instructions;
- preserve current/proposed documentation boundaries.

Exit: both registered worktrees are clean, share the same base, and have a
self-contained Wave 1 brief.

### Wave 1 - Kernel and MCP protocol foundation

Runtime Intelligence:

- make one shared Runtime-neutral Agent kernel the only multi-turn model/tool
  coordinator used by embedded and durable execution;
- converge control and extension hooks without moving persistence, approval,
  workspace, or product authority into `rove-core`;
- preserve planned/unplanned behavior and serialized compatibility.

Protocol & Platform:

- introduce a typed internal MCP protocol/result foundation with a compatible
  projection to the existing public ToolOutput;
- implement one shared JSON-RPC dispatcher for stdio and legacy SSE;
- complete stable identity, conservative safety, discovery pagination, and
  concurrent/notification/error regression coverage;
- do not add Streamable HTTP yet.

Hotspot rule: Runtime Intelligence owns `core/src/` and Runtime engine loop
changes. Protocol & Platform remains inside MCP/provider assembly modules and
must not redesign public Core ToolOutput or canonical events in this wave.

### Integration Barrier 1

The coordinator proves that embedded Agent, CLI, API, Web, benchmark, planned,
and unplanned paths share the intended kernel while current MCP stdio/SSE
behavior remains compatible. Any common rich-result contract needed later is
sealed here as an additive coordinator change.

### Wave 2 - Lifecycle completion and Streamable HTTP

Runtime Intelligence:

- add bounded model-on-ambiguity evaluation after deterministic rules;
- add an independent evidence-grounded Finalizer and deterministic fallback;
- implement public multidimensional budgets and global enforcement;
- complete trace-tail reconciliation, canonical events, persistence, resume,
  repair, configuration, and CLI/API/Web lifecycle surfaces.

Protocol & Platform:

- implement MCP Streamable HTTP POST JSON/SSE, negotiated sessions, GET/DELETE,
  headers and secret references, reconnect, timeout, cancellation, commit-point
  tracking, retry budgets, and typed indeterminate outcomes;
- prove behavior with a deterministic local mock and keep stdio/SSE regression
  coverage.

### Integration Barrier 2

The coordinator seals public result/artifact references, lifecycle event
ordering, schema defaults, migration behavior, and shared Runtime identity
fields before the next wave. No worker independently invents a parallel event
or artifact lifecycle.

### Wave 3 - Agent packages/procedures and rich MCP artifacts

Runtime Intelligence:

- implement AgentDefinition/Profile types, legacy prompt mapping, package
  loading/validation, hashing, immutable run identity, and diagnostics;
- implement root then nested `AGENTS.md` discovery with bounded scope and prompt
  metadata;
- implement procedure schema, validation, trust/provenance, deterministic
  catalog selection, and progressive hydration without granting permission by
  prose.

Protocol & Platform:

- complete rich MCP content and structured result mapping;
- implement the quota-, hash-, retention-, redaction-, and MIME-bounded Tool
  Artifact store and model/planner/UI/audit projections;
- add capability refresh, atomic catalog replacement, run pinning, persistence,
  resume, API/Web diagnostics, and download safety.

### Integration Barrier 3

The coordinator verifies that profile, procedure, capability, result, and
artifact identities are stable across checkpoint/resume and remain separate
trust authorities.

### Wave 4 - Integrated intelligence, evaluation, and Desktop

Runtime Intelligence:

- integrate selected procedures with PlannerContext, StepRunner, deviation,
  Evaluator, Finalizer, persistence, and product diagnostics;
- implement the versioned OnCall fixture/oracle/evidence package, deterministic
  tools, direct ToolRegistry baseline, lifecycle/procedure/MCP scenarios,
  failures, cancellation, resume, safety gates, and comparable ablations;
- keep provider experiments opt-in and separate from deterministic claims.

Protocol & Platform:

- first seal a Desktop D0 design and implementation plan;
- add a Tauri 2 host that reuses the shared Web UI and server-owned runtime/API
  contracts without creating Desktop-only session or event truth;
- add bounded filesystem/secret bridge commands, packaging, platform tests, and
  release evidence.

### Wave 5 - Program acceptance

Coordinator-led convergence:

- run full Rust, Web, browser, benchmark, migration, resume, and security gates;
- run Desktop packaging/platform gates on supported hosts;
- run external provider and real MCP gates only with explicit credentials and
  non-production services;
- produce machine-readable evidence with real exit codes;
- update `docs/runtime/`, acceptance matrices, root guidance, and archive or
  retire superseded active plans without rewriting historical evidence.

## 7. Quality and Security Gates

Every wave must preserve repository invariants and include:

- focused tests first, then `cargo fmt --all --check`, workspace Clippy with
  `-D warnings`, and proportional workspace tests;
- Web test/typecheck/build and browser E2E for browser-visible contracts;
- additive defaults, migrations, and old-artifact tests for serialized changes;
- negative tests for approval, paths, URLs, redirects, secrets, retries,
  cancellation, resume, MIME/resource handling, and untrusted instructions;
- deterministic local execution without credentials or network;
- bounded input, output, concurrency, timeout, storage, retry, and history;
- docs that distinguish current implementation, active work, optional evidence,
  and proposed targets.

No pass may be claimed from a skipped real-service gate or a report without a
real process exit code.

## 8. Program Completion

The program is complete only when:

1. all in-scope waves are implemented and integrated on `main`;
2. there is one shared kernel, event lifecycle, durable truth, tool safety path,
   artifact authority, and product session truth;
3. deterministic and compatibility gates pass from a clean checkout;
4. optional external gates are reported honestly as passed, failed, or not run;
5. current documentation agrees with reproducible behavior;
6. both worker worktrees are clean and no generated state is committed;
7. `origin/main` contains the reviewed result.
