# Post-CDH Agent Kernel and Coding Capability Plan

> Status: **Superseded / Historical**
>
> Replaced on 2026-08-06 by the two independently executable briefs:
> [`Kernel, Message, and Provider Implementation`](../../plans/2026-08-06-kernel-message-provider-implementation.md)
> and [`Project Trust, Execution Environment, and Coding Tools
> Implementation`](../../plans/2026-08-06-project-trust-execution-tools-implementation.md).
> Its former execution and conversation rules are retained only as history and
> are not current instructions.
>
> Created: 2026-08-05
>
> Last reconciled: 2026-08-06
>
> Execution rule: **Subagents are prohibited inside every implementation
> conversation.** The user may run at most two independent top-level
> conversations concurrently, one bound to each implementation worktree, when
> dependency gates and the ownership manifest permit it. A top-level
> conversation must never call, create, delegate to, or resume a Subagent.
>
> Implementation baseline: `main` at
> `f9e88a7553bcc7561550e5b8286c320108c8fd51`, the merge commit for PR #29,
> `feat(product): complete CDH control, evidence, and settings surface (G1-G7)`.
> The post-merge push CI passed both required Rust and Web jobs. M0 records the
> stronger clean-tree acceptance evidence and the optional gates that did not
> run.
>
> Desktop status: no `apps/desktop` host exists. Desktop is a separate future
> D0 program and is not part of this implementation program's completion gate.
>
> Worktree boundary: the retired CDH worktrees and branches are not
> implementation bases. New implementation begins only from the sealed `main`
> baseline and follows the two-worktree ownership, dependency, and conversation
> concurrency rules in Section 0.
>
> Current runtime truth remains under [`docs/runtime/`](../../runtime/README.md).
> This plan must not be used as evidence that the proposed types, hooks,
> execution environments, tools, trust flow, Skills, Subagents, or Desktop
> behavior already exist.

This is the coordinator-level plan for the architecture and coding-capability
work that follows CDH. It records the agreed product direction, the reason the
work is necessary, the target ownership boundaries, the dependency order, the
compatibility rules, and the evidence required before any capability is called
complete.

The goal is not to rewrite rove as Pi or to copy Claude Code. The goal is:

> Build a local-first, durable, resumable coding Agent product with a small,
> stable, composable kernel; Pi-inspired component boundaries; Claude
> Code-level coding-tool discipline; and rove's existing event, approval,
> persistence, recovery, provider, and multi-surface strengths.

---

## 0. Coordinator, worktree, and conversation-concurrency rules

This document is the coordinator-owned source for delivery order, dependency
gates, worktree ownership, compatibility, and acceptance. Detailed future
architecture still belongs under `docs/design/`; Sections 5-13 below are
program constraints and design inputs. M1 must land or update the dedicated
design documents before implementation changes their contracts.

The implementation may keep at most two implementation worktrees:

```text
main                         coordinator integration and release truth
  +-- worktree A            kernel / message / provider track
  +-- worktree B            trust / environment / coding-tool track
                              (at most one top-level conversation per worktree)
```

The concurrency boundary applies inside each conversation, not across
user-opened top-level conversations:

1. No top-level implementation conversation may call, create, delegate to, or
   resume any Subagent. There is no exception for research, review, tests, or
   supposedly disjoint implementation work.
2. The user may open at most two top-level implementation conversations: one
   assigned to worktree A and one assigned to worktree B. These user-created
   conversations are independent primary sessions, not Subagents.
3. Each conversation is pinned to its assigned worktree, branch, milestone,
   and allowed-file set. It must not edit the other worktree or perform
   coordinator integration directly on `main`.
4. The two conversations may execute simultaneously only when their dependency
   gates are already merged and their owned files are disjoint under the M1
   manifest. M0.5 and M1 are bootstrap checkpoints and remain ordered.
5. Each conversation must finish a bounded checkpoint, record commits,
   tests/status, changed files, and remaining risks, and leave a clean handoff
   before changing branches or milestone ownership.
6. Shared hotspots are coordinator-owned unless a milestone handoff explicitly
   assigns them: public serialized types, canonical events, migrations,
   `Cargo.lock`, generated schemas, acceptance reports, and current runtime
   documentation. If concurrent work discovers a shared-hotspot change, that
   work pauses until the coordinator assigns one exclusive owner.
7. No later worktree starts from an unmerged dependency branch. Merge and
   verify the dependency checkpoint first, then create or rebase the dependent
   worktree from the new `main`.
8. Coordinator merges, shared-hotspot edits, full acceptance, release cleanup,
   and pushes to `main` are serialized. Both implementation conversations pause
   while the coordinator integrates a dependency checkpoint.

Before assigning work in either worktree, the coordinator records branch name,
base SHA, allowed files, forbidden hotspots, required tests, rollback floor,
merge order, and whether the checkpoint is concurrency-eligible in Section 14.
Worktree-local generated state (`target/`, `node_modules/`, `.next/`, `.rove/`,
Playwright output, logs) is never committed.

---

## 1. Source-of-truth and reference boundary

### 1.1 Repository truth

Implementation decisions in this plan remain subordinate to:

1. post-CDH source code, tests, generated schemas, and reproducible behavior;
2. [`docs/runtime/`](../../runtime/README.md) for implemented current behavior;
3. root `README.md`, `MEMORY_DOCTRINE.md`, and
   [`docs/ONBOARDING.md`](../../ONBOARDING.md);
4. active target designs under [`docs/design/`](../../design/);
5. this implementation plan.

M0 reconciled the source and line references in this document against the merged
post-CDH tree. A changed line number is not a design change, but changed
behavior is. When later code and this plan disagree, implementation pauses until
the contradiction is recorded and resolved.

### 1.2 Pi reference boundary

The Pi reference checkout was updated before this plan was written. The
inspected revision was `588915ec7` (`docs(agent): clarify JSONL fork entry
replay`). Its implemented `ExecutionEnv`, file tools, coding-agent extension
surface, Session model, and progressive instruction/Skill loading are useful
references.

Pi's Harness V2 document is not complete runtime evidence. At the inspected
revision it explicitly describes `AgentHarness` as type-complete but not
behavior-complete, and execution-bearing scaffold methods still reject with
`HarnessNotImplemented`. This plan may adopt its separation of concerns, but
must not claim validation from scaffold or unchecked design tracks.

Pi's product choices are also not rove requirements. In particular, the
absence or different treatment of first-class approval, durable Plan evidence,
MCP, and Subagents must not be copied. The useful lesson is to keep those
capabilities outside the stable loop mechanics and compose them through narrow
contracts; rove retains the capabilities and safety guarantees that serve its
own product.

### 1.3 Claude Code reference boundary

Claude Code is a closed-source product. rove may learn from public observable
behavior, documented interfaces, and independently inspectable teaching or
analysis material, but this plan makes no claim about Claude Code's complete
internal implementation and must not reproduce proprietary source.

The reusable behavior target is limited to ideas such as exact replacement,
stale-read protection, bounded context, checkpoints, background process
identity, progressive output, layered context recovery, permission separation,
and isolated Subagent context.

### 1.4 CDH boundary

CDH G1-G7 merged through PR #29. G8 Desktop was explicitly out of scope and no
`apps/desktop` host exists. M0 verified the merged controls, lineage, session
model, usage/context/cost, product file/artifact/diff, evidence export, and
Settings/MCP surfaces against source, tests, generated schemas, and current
documentation. This program must consume those contracts rather than port or
reimplement the former worktree versions.

### 1.5 Active-design adoption matrix

M1 owns the decision record that turns this matrix into sealed contracts. No
worker may infer that a referenced proposed design is already implemented.

| Existing design | This program's treatment | Owning milestone |
|---|---|---|
| Agent execution lifecycle | Preserve the implemented ledger/revision/StepRunner slice; implement model-on-ambiguity evaluation, independent Finalizer, multidimensional budgets, and trace-tail reconciliation | M1 contracts, M4 implementation, M10 evidence |
| Agent definition and procedural knowledge | Implement versioned AgentDefinition, immutable AgentRuntimeProfile, root `AGENTS.md`, typed procedure/Skill catalog, capability binding, progressive hydration, and exact resume identity | M1 contracts, M8B implementation |
| MCP Streamable HTTP and Tool Artifacts | Reuse its capability/resource/artifact taxonomy where required, but defer Streamable HTTP/session transport unless a separate accepted slice is added | M1 scope decision; M3/M6 integration only |
| OnCall reference evaluation | Reuse the evaluation method for procedure-aware planning; do not claim that its fixtures or environment are implemented | M8B and M10 evaluation |
| Optional TUI direction | Preserve the current shared Runtime/TUI contract; no TUI redesign is required by this program | Regression-only unless separately approved |

M1 must either update those documents or add a dated replacement design under
`docs/design/`. Silent divergence is not allowed.

---

## 2. Fixed product and architecture decisions

The following decisions are fixed for this program unless a replacement design
records the reason, migration, and affected acceptance criteria.

1. **Product behavior and kernel architecture are different choices.** rove
   should offer a Claude Code-class coding workflow while using Pi-inspired
   composition internally. It is neither a Pi clone nor a Claude Code shell.
2. **There is one Agent loop.** `rove-core` will own the one reusable,
   persistence-agnostic ReAct kernel. `rove-runtime::Engine` remains the
   product's durable coordinator but must delegate model/tool iteration to that
   kernel instead of maintaining another Agent loop.
3. **Plan is an execution strategy, not the universal loop.** Ordinary coding
   starts with lightweight ReAct. Explicit planning, sufficiently long tasks,
   and resume of an existing planned run may use PlanReact. Both strategies use
   the same kernel.
4. **Application/session records are not model protocol messages.** Background
   notifications, control receipts, Skill hydration, Subagent results, UI
   state, lineage, and summaries must not be disguised as
   `rove_models::Message` values.
5. **Canonical events remain the lifecycle truth.** New session or message
   projections cannot create a second writable event lifecycle.
6. **Behavior is internally composable before it is publicly pluggable.** This
   program first establishes stable Rust ports, registration, ordering, failure
   semantics, and tests. It does not promise a dynamic Rust ABI or arbitrary
   third-party UI code.
7. **All built-in local execution uses a Rove Execution Environment.** File
   read/write/edit, search, Shell, background process management, and local
   stdio process spawning must stop owning workspace `PathBuf` values and
   directly invoking host APIs from tool implementations.
8. **The environment cannot grant authority.** Workspace trust, tool policy,
   approval, capability ceilings, and secret policy remain Runtime/operator
   authorities. An environment adapter only implements capabilities already
   allowed by those authorities.
9. **Project Trust is a release boundary.** Selecting or opening an unfamiliar
   repository cannot silently activate repository-owned configuration, MCP
   processes, hooks, Skills, or instructions.
10. **Tools are part of Agent intelligence.** Bounded Read, exact Edit, real
    Diff, durable output, streaming/background Shell, and coding evaluations
    are product-critical work, not utility polish.
11. **Existing rove strengths remain intact.** Persistence, conservative
    resume, canonical events, approval, provider normalization, context/memory
    authority, deterministic local execution, the shared CLI/API/Web Runtime,
    and future Desktop reuse are constraints on the redesign.
12. **A future Desktop is a thin host.** CDH did not create `apps/desktop`. Any
    separately accepted D0 implementation must reuse the same product
    transport, kernel, Runtime, canonical events, state, and components. It
    cannot own a private Agent loop, transcript, control queue, or filesystem
    semantics.
13. **Provider adapters translate protocols; they do not define Agent
    semantics.** OpenAI Responses, OpenAI Chat Completions, Anthropic Messages,
    Ollama, and future protocols convert at the `rove-models` boundary. Tool
    identity, result status, history repair, stop semantics, and schema policy
    require provider-neutral contracts before the wire adapter.
14. **The remaining accepted lifecycle design is part of this program.**
    Model-on-ambiguity evaluation, an evidence-grounded independent Finalizer,
    public multidimensional budgets with global accounting, and trace-tail
    reconciliation must be implemented or explicitly deferred by a replacement
    decision. They cannot disappear during the one-kernel migration.
15. **Named Agent identity is distinct from Skills.** Versioned
    `AgentDefinition` packages compile to immutable `AgentRuntimeProfile`
    snapshots. Workspace instructions, procedures/Skills, memory, reference
    material, and runtime evidence retain distinct authority and persistence.
16. **Product artifacts and MCP Tool Artifacts are different contracts.** CDH
    delivered bounded product artifact listing/download/preview and evidence
    export. Rich MCP result envelopes and Tool Artifact projection remain
    proposed until the MCP design is implemented.
17. **Coding-quality claims require coding-quality evidence.** Fake-provider
    scenarios prove deterministic contracts. A claim that a supported model is
    Claude Code-class requires a separately gated, versioned real-model
    evaluation with declared model, cost, repetitions, and thresholds.

---

## 3. Current post-CDH diagnosis

This section records the M0-reconciled post-CDH baseline. The structural and
product gaps below were rechecked against `main` at `f9e88a7`; they are not a
claim that the proposed replacement contracts already exist.

### 3.0 M0 merged capability inventory

The sealed baseline includes:

- CDH G1 API-authoritative Steer/Follow-up controls with safe-point delivery,
  durable receipts, recovery, revoke/confirmation, and the Composer queue;
- G2 terminal-boundary session Fork with immutable inherited transcript
  segments, fresh child runtime identity, and bounded lineage;
- G3 session-scoped provider/model/reasoning/approval/step-limit configuration
  with revision CAS and immutable per-run snapshots;
- G4 usage, cost, and context occupancy with explicit unavailable states;
- G5 bounded product workspace file browsing, artifact manifest/download/
  preview, image validation, and run/Git diff;
- G6 redacted JSON/HTML/Markdown evidence export;
- G7 workspace-scoped MCP catalog management, typed probes, bounded transports,
  and fail-closed configuration handling;
- the Web Complete C0-C3 product store, continuity, Settings, migration, polish,
  and live local acceptance paths already present before CDH.

No Desktop host, MCP Streamable HTTP session protocol, rich MCP result envelope,
Tool Artifact model projection, AgentDefinition loader, runtime `AGENTS.md`
discovery, procedure catalog, Subagent runtime, or shared Execution Environment
exists at this baseline.

### 3.1 Package layering exists, behavior composition is incomplete

The package dependency direction is healthy:

```text
rove-models <- rove-core <- rove-runtime <- rove-app-bootstrap <- apps
```

The remaining problem is not directory layout. It is that stable behavior
boundaries do not yet cover context transformation, session projection,
compaction policy, model request lifecycle, resources, provider lifecycle, or
UI projection.

Pi's important message flow is conceptually:

```text
Agent/application message
  -> context transformation
  -> LLM conversion
  -> provider message
```

Post-CDH rove still persists working history directly as `Vec<Message>` in
[`runtime/src/foundation/types.rs`](../../../runtime/src/foundation/types.rs).
Those messages have model-protocol roles such as system, user, assistant, and
tool. This is adequate for a simple chat loop but creates pressure to encode
future product facts as fake model messages.

### 3.2 The product still has two Agent loops

[`core/src/agent.rs`](../../../core/src/agent.rs) contains an embeddable
Agent loop with model iteration, tool execution, Steer, and Follow-up behavior.
The product path separately coordinates unplanned and planned loops under
`runtime/src/engine/` and currently reuses lower-level Core turn conversion
rather than the complete Core loop.

This is the largest structural debt. Any cross-turn feature can drift between:

- the embedded Core Agent;
- Runtime unplanned execution;
- Runtime planned StepRunner execution.

CDH's need to apply controls consistently across planned and unplanned paths is
a concrete example of why one kernel is required.

### 3.3 A subsystem is not automatically an extension point

Context, Session, compaction, tools, providers, and UI all exist. The gap is
whether another component can compose or replace a policy without editing the
Engine, API, and Web implementation.

| Area | Implemented subsystem | Missing or incomplete composition boundary |
|---|---|---|
| Context | Budgeted context manager and prompt assembly | Ordered `transform_context` pipeline with typed provenance |
| Session | Durable task/session/product state and resume | Typed application entries plus model/UI projectors |
| Compaction | Deterministic/model compaction and fallback | Before/after hooks and replace/supply/decline policy |
| Provider | `ModelClient`, native strategies, routing, registry injection | Run/session interception, auth refresh, request/response lifecycle |
| Tools | `ToolRegistry`, Executor, approval, three Hook families | Environment-neutral execution and broader lifecycle composition |
| Resources | Workspace, memory, current and proposed artifacts | One typed resource capability/query boundary |
| UI | Real Web product shell and canonical transcript projection | Typed command/result/panel projection registration |

The current Runtime Hook families are `PreToolHook`, `PostToolHook`, and
`PostRunHook` in
[`runtime/src/tools/hooks/mod.rs`](../../../runtime/src/tools/hooks/mod.rs).
They are useful but cannot express context transformation, compaction policy,
Session projection, model interception, or resource/UI contribution.

The first target is internal composition. Public plugin packaging and dynamic
loading are separate decisions and are not required for this program.

### 3.4 Skills and workspace instructions remain future behavior

The target design exists in
[`docs/design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`](../../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md),
but post-CDH Runtime does not discover workspace `AGENTS.md` files or provide
Skill progressive disclosure.

The intended behavior is layered instruction discovery plus a bounded Skill
catalog whose name and description may remain available while the full body is
loaded only when selected. Instructions and Skills need provenance, trust, and
authority labels and cannot grant tool permission.

### 3.5 File and Shell tools are below the real coding-product bar

The post-CDH filesystem implementation in
[`runtime/src/tools/fs.rs`](../../../runtime/src/tools/fs.rs):

- reads an entire UTF-8 file with `tokio::fs::read_to_string`;
- has no `offset`, line range, byte range, or continuation contract;
- updates an existing file only through whole-file overwrite;
- creates a display Diff by marking every old line removed and every new line
  added rather than computing localized hunks.

The post-CDH Shell implementation in
[`runtime/src/tools/shell.rs`](../../../runtime/src/tools/shell.rs):

- directly owns a workspace `PathBuf` and launches a host shell;
- waits for process completion;
- truncates captured stdout/stderr after completion;
- discards bytes beyond the model-visible cap;
- exposes no durable process identity, polling, progressive output, completion
  notification, or background lifecycle.

These behaviors can prove that a pipeline works. They are insufficient for
safe and efficient work in a real repository.

### 3.6 There is no shared Execution Environment

Built-in filesystem and Shell tools own a local root and call `tokio::fs` or
`tokio::process` directly. Stdio MCP also constructs a host process directly.
The tool implementation therefore knows both the requested operation and the
host backend.

That coupling blocks clean substitution of:

- a workspace-bounded local directory;
- an isolated Git worktree;
- a container or sandbox;
- a remote execution host;
- a restricted Subagent environment;
- an in-memory deterministic test environment.

### 3.7 Project Trust is a release blocker

Post-CDH CLI/API configuration loading considers workspace `.rove/config.toml` in
[`apps/bootstrap/src/config.rs`](../../../apps/bootstrap/src/config.rs).
The default MCP path is `.rove/mcp_servers.json`, first-party assembly registers
configured MCP servers, and stdio MCP creates the configured host command in
[`runtime/src/tools/mcp_proxy.rs`](../../../runtime/src/tools/mcp_proxy.rs).

The product path remains vulnerable to the unsafe class: repository-controlled
data must not cause a process to start merely because a user opened or selected
an unfamiliar folder. M0.5 adds a fail-closed activation guard before the full
M5 trust store and product surfaces.

### 3.8 Plan is currently too eager

Post-CDH first-party assembly constructs `EngineConfig` with
`plan_enabled: true` in
[`apps/bootstrap/src/assembly.rs`](../../../apps/bootstrap/src/assembly.rs).
This can add a planning model call even for ordinary coding work.

rove should preserve StepRecord, PlanDecision, PlanRevision, bounded
StepRunner, and conservative planned resume. Those are differentiating assets.
The default strategy selection, however, should not force all work through a
JSON Plan.

### 3.9 Other concentration and contract risks

The M0 post-CDH audit measured:

- API handler, Job Supervisor, resume/binding, ProductStore, and transaction
  ownership concentration;
- duplicate lifecycle behavior across Core and Runtime;
- manually mirrored Rust/OpenAPI/Web types and the cost of one contract change;
- any private event, host, resource, or Desktop path introduced by CDH;
- whether current or future host adapters could bypass Runtime approval or
  workspace identity;
- whether background controls could replay after cancellation or restart.

The audit found no CDH-private lifecycle or Desktop host. It retained the
duplicate Core/Runtime loop, schema-mirroring, host-bound execution, and
ownership concentration risks for M1-M9 rather than treating the merged product
surface as proof that those architectural gaps were closed.

### 3.10 Priority model

Risk severity and implementation order are related but not identical. The
architecture foundation is implemented first; release/security blockers must
all be closed before a real Desktop release.

| Severity | Component | Current judgment | Delivery dependency |
|---|---|---|---|
| P0 structural | Core/Runtime dual loop | Largest architecture debt | First implementation program |
| P0 structural | Message/session projection | Blocks clean Skills, Subagents, background tasks, and extensions | With the kernel foundation |
| P0 immediate | Project Trust guard | Untrusted-workspace process execution risk exists now | M0.5 before architectural migration |
| P0 release | Full Project Trust | Persistent, granular, revocable grants | M5 before workspace instructions/Skills or any Desktop work |
| P0 product | File and Shell tools | Insufficient for high-quality real coding | After Execution Environment contract |
| P0 evidence | Agent capability evaluation | Current smoke proves plumbing, not coding competence | Baseline before changes; gate every milestone |
| P0 protocol | Provider tool-call normalization | Adapter direction is sound, but the neutral message/result/stop/schema contracts are too narrow | After typed message contracts, before one-kernel migration completes |
| P1 structural | Extension plane | Existing behavior still invades Engine/API/Web | After message and kernel contracts |
| P1 application | API/ProductStore service boundaries | Must be re-measured after CDH | Refactor only around verified concentration |
| P1 policy | PlanReact default selection | Useful strategy is applied too broadly | After one kernel exists |
| P2 contract | Rust/OpenAPI/Web type mirroring | Contract changes require repeated manual maintenance | After public schemas stabilize |

---

## 4. Assets that must not be damaged

This program must preserve and add regression evidence for:

- canonical `StreamEvent` lifecycle facts;
- `trace.jsonl` as append-oriented event truth;
- `task_state.json` as resumable state;
- `report.json` as a derived summary;
- immutable PlanRevision and append-only StepRecord/PlanDecision evidence;
- conservative resume that does not replay completed mutations or unknown
  external side effects;
- workspace path bounding and Runtime-owned approval;
- CLI/API/Web and any Desktop host sharing the same persistent Runtime;
- normalized providers, routing/fallback, health, and committed-output safety;
- working/session/durable memory authority and compaction degradation;
- exact product-session/runtime bindings and one active product turn;
- local deterministic Fake-provider execution without network credentials;
- secret redaction from config, events, state, reports, exports, fixtures, and
  screenshots;
- CDH control, lineage, artifact, export, and host invariants that actually
  land and pass M0.

The target is a smaller stable kernel surrounded by these strengths, not their
replacement.

---

## 5. Target ownership architecture

### 5.1 High-level composition

```text
CLI / API / Web / Desktop
             |
       rove-app-bootstrap
             |
    rove-runtime::Engine
    durable coordination only
             |
    +--------+------------------------------+
    |                                       |
ExecutionStrategy                    Runtime services
React / PlanReact              state, events, memory, trust,
    |                         extensions, approval, environments
    +-------------------+-------------------+
                        |
              rove-core AgentKernel
          the single reusable ReAct loop
                        |
        model port / tool-executor port / control inbox
                        |
                  rove-models
          normalized provider protocol and routing
```

Names are conceptual until M1 seals the public contracts. Ownership is not:

- Core owns the one model/tool iteration algorithm and its in-memory control
  semantics.
- Runtime owns durable orchestration, strategy selection, policy, context,
  compaction, state, resume, events, extensions, and execution environments.
- Apps own transport and presentation adapters only.

### 5.2 One-kernel rule

After migration, there must be no separately maintained Runtime ReAct loop.
Planned execution may repeat the kernel within a bounded step, but it cannot
reimplement model-call, action normalization, tool execution, tool-result
writeback, Steer, Follow-up, cancellation, or final-answer handling.

Conceptually:

```text
React strategy
  -> AgentKernel.run(scope = run)

PlanReact strategy
  -> Planner / resumed PlanRevision
  -> for each pending step:
       AgentKernel.run(scope = bounded step)
       -> StepRecord
       -> PlanDecision
  -> Finalizer
```

The Runtime adapter provides:

- context snapshots and transforms;
- model client;
- Runtime ToolExecutor;
- control inbox and safe-boundary policy;
- cancellation and budgets;
- event sink and persistence callbacks;
- checkpoint requests;
- finalization policy.

Core cannot open workspace paths, load memory files, decide approval, write
SQLite, or create product state.

### 5.3 Message and projection planes

The target separates five concepts:

```text
Canonical events + typed session/application entries
                    |
             SessionProjector
                    |
             AgentMessage[]
                    |
       ordered ContextTransform pipeline
                    |
          ModelMessage conversion
                    |
       rove_models::Message[]
                    |
              ModelClient
```

Separately, as a presentation projection flow (not concurrent Agent work):

```text
Canonical events
  -> transcript/read-model projector
  -> CLI / API / Web / Desktop presentation
```

Required distinctions:

- **Canonical event**: execution fact and ordering source.
- **Session/application entry**: typed conversation or application material
  with provenance, such as user text, assistant output, background completion,
  compact summary, Skill hydration, or Subagent return.
- **AgentMessage**: provider-independent model-context candidate.
- **Context segment**: budgeted and authority-labelled material selected for a
  turn.
- **Model message**: normalized provider protocol object.
- **UI read model**: display projection, never a writable lifecycle.

A session entry that originates from an execution event must retain that event
identity or derivation provenance. It cannot fork canonical truth. Product-only
catalog data remains ProductStore data; execution facts remain canonical
events.

### 5.4 Persistence migration target

`TaskState.history: Vec<Message>` cannot be removed in one incompatible step.
The migration requires:

1. a new schema version and typed session/context representation;
2. a decoder for existing snapshots;
3. deterministic legacy `Message` to typed-entry conversion where possible;
4. an explicit opaque/legacy entry when conversion is not lossless;
5. checkpoint and report compatibility tests;
6. resume tests from committed old fixtures;
7. no rewriting of historical `trace.jsonl` facts;
8. repair behavior that understands both schema generations;
9. a bounded deprecation period before the legacy writer is removed.

### 5.5 Internal extension plane

The first extension plane is a statically composed Runtime service, not a
dynamic native plugin ABI. It needs deterministic registration order, typed
inputs/outputs, cancellation, timeout, error policy, provenance, and tracing.

Minimum lifecycle families:

| Family | Required decisions |
|---|---|
| Session | create/open/resume, typed entry append, projection |
| Context | contribute, transform, filter, budget, final model conversion |
| Model | before request, safe request mutation, after normalized response |
| Compaction | before, supply/replace/decline, after, degradation |
| Tool | register, before/after execution, result-to-context projection |
| Resource | list/read/reference bounded workspace or artifact resources |
| Provider | register strategy/client, session resolution, auth refresh boundary |
| UI projection | register typed command/result/view descriptors, not executable workspace UI code |

Required invariants:

- no extension can raise approval, trust, or capability ceilings;
- untrusted workspace text cannot register executable code;
- hook ordering is deterministic and observable;
- each hook declares fail-open or fail-closed behavior where the Runtime permits
  a choice; security and mutation hooks are always fail-closed;
- timeouts, cancellation, panic isolation, and bounded output are explicit;
- extension-generated context retains source, authority, and trust labels;
- session and model transformations cannot mutate canonical historical facts;
- provider-specific payloads stay behind `rove-models`;
- UI contributions are typed data rendered by first-party components, not
  arbitrary HTML, JavaScript, or native commands.

### 5.6 Provider-neutral tool-call protocol and adapters

#### 5.6.1 Current architecture is directionally correct

Post-CDH rove already uses the right top-level pattern:

```text
ToolDescriptor
  -> provider-neutral ModelToolSchema
  -> ModelClient / ProviderClient
  -> selected WireProtocol.build_request
  -> OpenAI Responses / OpenAI Chat / Anthropic / Ollama wire payload

provider byte stream
  -> bounded FrameBuffer
  -> selected StreamDecoder
  -> provider-neutral ModelEvent
  -> Core model turn
  -> Action::ToolCall / ToolBatch
  -> ToolRegistry validation and Runtime ToolExecutor
  -> tool result in the next model turn
```

The principal types and boundaries are:

- [`models/src/protocol.rs`](../../../models/src/protocol.rs): `Message`,
  `ToolCallRef`, `ModelToolSchema`, and usage;
- [`models/src/traits.rs`](../../../models/src/traits.rs): `ModelClient` and
  normalized streaming `ModelEvent`;
- [`models/src/provider/wire.rs`](../../../models/src/provider/wire.rs):
  `WireProtocol`, `WireRequest`, and `StreamDecoder`;
- [`models/src/provider/client.rs`](../../../models/src/provider/client.rs):
  production `ProviderClient` assembly;
- `models/src/provider/protocols/`: independent OpenAI Responses, OpenAI Chat
  Completions, Anthropic Messages, and Ollama request/stream adapters;
- [`core/src/model_turn.rs`](../../../core/src/model_turn.rs): conversion of
  normalized model events to Core actions;
- [`core/src/tools.rs`](../../../core/src/tools.rs): registry lookup,
  provider-independent argument validation, and execution.

Current outbound mappings are already isolated correctly:

| Internal concept | OpenAI Responses | OpenAI Chat Completions | Anthropic Messages |
|---|---|---|---|
| Tool definition | `type=function`, top-level name/description/parameters | `type=function` with nested `function` | name/description/`input_schema` |
| Assistant call | `function_call` item | assistant `tool_calls[]` | assistant `tool_use` block |
| Arguments | JSON string | JSON string | input object |
| Tool result | `function_call_output` | `role=tool` | user `tool_result` block |
| Correlation | `call_id` | tool call `id` / `tool_call_id` | `tool_use_id` |

This layer should be evolved, not replaced. In particular, the following
strengths must remain:

- provider payload types do not enter Core/Runtime tool execution;
- operational metadata such as destructive/parallel-safe/capability is omitted
  from the model-visible schema;
- `WireProtocolRegistry` uses open validated protocol IDs and rejects duplicate
  protocol registration;
- framing, HTTP bounds, auth/header redaction, error classification, retry, and
  committed-output fallback remain model-layer responsibilities;
- native tool calls win over compatibility JSON text actions;
- provider adapters have deterministic payload/stream fixture tests;
- routing does not switch provider after visible text or tool use commits.

#### 5.6.2 The neutral protocol is currently too lossy

The current `Message` shape is one text string plus optional `tool_calls` or one
`tool_call_id`. It does not retain:

- ordered text/thinking/tool-call content blocks;
- tool result name;
- success/error/unknown-effect status;
- text versus image/artifact result blocks;
- assistant stop reason;
- response/model/protocol provenance;
- same-provider replay metadata versus cross-provider-safe content;
- a provider-independent call identity distinct from the wire call ID.

This produces concrete behavior gaps:

1. Runtime records failed execution as text such as `Error: ...`. Anthropic's
   adapter emits a `tool_result` without `is_error: true`, so the target model
   cannot reliably distinguish execution failure from successful textual
   output.
2. Tool name is absent from persisted tool-result messages. Some compatible
   Chat APIs require or benefit from the result name, and UI/model projections
   cannot verify call/result name correlation.
3. Interleaved provider content is flattened into one assistant string plus a
   separate call list, so original content-block order cannot be replayed.
4. `ModelEvent::Done` carries no normalized stop reason. Length truncation,
   content filtering, normal stop, tool use, incomplete response, abort, and
   provider error cannot be handled through one Core contract.
5. Reasoning/signature content is currently discarded rather than leaked,
   which is the safer current behavior. If model reasoning continuity is added,
   opaque same-provider replay state needs an explicit origin-bound contract;
   it must not be replayed blindly across models/providers.

Typed session/application messages from Section 5.3 should therefore project
to a richer provider-neutral model exchange rather than extending the current
flat `Message` with unrelated optional fields indefinitely.

Conceptually:

```text
AssistantTurn
  content: Text | ThinkingReference | ToolCall
  stop_reason
  usage
  model/protocol provenance
  safe diagnostics

ToolCall
  internal_call_id
  tool_name
  arguments
  optional origin-bound wire reference

ToolResult
  internal_call_id
  tool_name
  content blocks
  status: ok | error | rejected | partial | unknown_effect
  mutation/artifact references
```

Exact serialized types and ownership are sealed in M1/M2. Provider-specific
JSON must still remain behind `rove-models`; an origin-bound wire reference is
opaque to Core and cannot grant capability.

#### 5.6.3 Internal call identity must be separated from wire IDs

Post-CDH history persists the provider-returned ID directly in
`ToolCallRef.id`, then copies it into `Message.tool_call_id`. This works for a
same-provider next turn but is not a complete cross-provider contract:

- OpenAI Responses IDs may contain compound or long values;
- Anthropic requires a bounded restricted ID alphabet;
- compatible Chat endpoints impose different length/name rules;
- a session may change provider/model between product turns;
- a synthesized/missing provider ID currently uses adapter-specific fallback
  strings;
- normalizing only the call side would break its result correlation.

The target requires two identities:

- **internal call ID**: stable Rove identity used by events, persistence,
  approval, execution, UI, artifacts, and parent/child correlation;
- **wire call reference**: provider/protocol/target-bound opaque identity used
  only when projecting a request for that target.

Projection rules:

1. Same compatible provider/protocol/model replay preserves a valid original
   wire reference when required by that API.
2. Cross-provider or incompatible-model replay deterministically generates a
   target-valid alias.
3. Call and result use the same alias map in one projection.
4. Mapping detects collisions after truncation/sanitization and resolves them
   deterministically.
5. Raw wire IDs are never used as Runtime approval IDs or artifact IDs.
6. Mapping does not mutate canonical Session history.
7. Missing or invalid required wire identity is a typed projection error or a
   typed synthetic failed result, never an empty successful ID.

The internal ID should be allocated when a normalized call start is accepted,
not only after arguments finish, so progress, cancellation, approval, and a
terminal incomplete-call fact can correlate without adopting an unstable wire
fragment ID. Execution eligibility is still withheld until the complete turn
and arguments pass validation.

#### 5.6.4 Cross-provider history requires one shared projector

History repair is currently split across layers:

- `ContextManager` includes complete native call/result rounds atomically and
  drops an incomplete suffix;
- product follow-up code under `apps/api` closes missing tool results with a
  synthetic unknown-effect text result;
- each wire adapter independently maps roles/content;
- no shared layer normalizes target-provider IDs, unsupported content, or
  same-provider opaque replay state.

This logic belongs in one provider-neutral `ModelHistoryProjector` (conceptual
name) between the context pipeline and `WireProtocol`:

```text
typed Agent/model context
  -> validate call/result rounds
  -> close or reject missing results using explicit policy
  -> remove aborted/incomplete assistant turns when unsafe to replay
  -> downgrade unsupported images/content with a visible typed reason
  -> preserve or strip origin-bound reasoning/signature state
  -> normalize tool call IDs and names for the target
  -> merge/project system/developer segments for target capabilities
  -> enforce target role/block ordering
  -> WireProtocol request conversion
```

Required semantics:

- every emitted tool call has exactly one correlated result before a later
  ordinary conversation turn, unless the target API explicitly supports a
  different typed state;
- a synthesized result has `is_error/status=unknown_effect`, the tool name,
  provenance, and a safe reason code;
- duplicate/empty/orphan result IDs are rejected or represented as explicit
  corrupt/partial history, not silently ignored;
- incomplete/aborted assistant output is never replayed as completed content;
- unsupported images become one bounded placeholder or artifact reference,
  not an attempted remote fetch;
- same-provider encrypted reasoning/signatures are retained only for the exact
  compatible target and are stripped or safely downgraded otherwise;
- transformations produce safe diagnostics and projection metadata without
  changing canonical events or persisted source entries;
- Product, CLI, embedded, resume, compaction, and Subagent paths use the same
  projector.

Target projection must occur after routing selects a concrete client. A
pre-commit fallback may select a different protocol, model, or capability set;
therefore a wire-ready history compiled once before `RoutingModelClient`
selection would be incorrect. Each `ProviderClient` should project the same
neutral request for its own target immediately before `WireProtocol` request
construction, with projection diagnostics available to the shared model-event
path.

#### 5.6.5 Tool result status must survive the full round trip

The existing Runtime `ToolExecutionMetadata.status` distinguishes ok, error,
rejected, and partial success, but this status is lost when history is reduced
to `Message::tool(content, id)`.

The new model-facing result contract must include:

- internal call ID and tool name;
- normalized status and `is_error` projection;
- bounded text/image/artifact content blocks;
- safe error code and retryability/unknown-effect signal where appropriate;
- mutation/artifact references for UI/evidence, with a separately bounded
  model projection;
- original event/result provenance.

Provider mapping then becomes explicit:

- Anthropic sets `tool_result.is_error` for error/rejected/unknown outcomes;
- OpenAI Responses emits the required `function_call_output`, encoding a safe
  bounded error result when the wire protocol has no separate error flag;
- Chat Completions emits `role=tool`, correct `tool_call_id`, and target-required
  name fields when supported;
- providers without native tools receive an explicit compatibility projection
  rather than an untyped role fallback.

An approval rejection, schema failure, unknown tool, timeout, cancellation, and
unknown external effect must not all collapse into indistinguishable successful
text.

#### 5.6.6 Stream normalization needs one validated state machine

Current protocol decoders independently accumulate partial arguments. This is
necessary at the wire edge, but common correctness rules are duplicated or
missing:

- `StreamDecoder::finish` defaults to success even with pending tool-call
  fragments or no terminal `Done`;
- clean EOF without a provider terminal event can appear as a completed Core
  turn;
- accumulated argument bytes and call counts do not have one shared semantic
  bound beyond frame/request limits;
- some adapters substitute names such as `tool` or IDs such as
  `toolu_unknown` instead of rejecting missing required fields;
- malformed argument JSON is converted to a JSON string and reaches later
  validation as if tool-call assembly itself succeeded;
- OpenAI Chat partial IDs can change from an index fallback to a provider ID;
- Core currently ignores `ToolUseDelta`, so it does not validate start/delta/end
  correlation or expose a safe bounded progress contract;
- normalized terminal reason does not protect against a truncated but
  parseable tool call on every provider.

The target has a common bounded `ToolCallAssembler`/turn state machine after
wire-specific frame parsing:

```text
turn_start
  -> content/tool_call start
  -> bounded deltas keyed by stable stream item identity
  -> tool_call end with valid object arguments
  -> terminal stop reason
  -> validated AssistantTurn
```

It must enforce:

- one legal start/end sequence per call;
- stable name and wire identity;
- unique calls within a turn;
- max call count, name/ID bytes, per-call argument bytes, and total argument
  bytes;
- strict JSON object arguments before execution eligibility;
- explicit handling of unknown frames and multiple choices;
- terminal event required for successful completion;
- pending fragments at EOF/timeout become `StreamInterrupted` or a typed
  incomplete turn;
- length/content-filter/error/aborted/deferred stop reasons are distinct;
- no tool call executes from a turn whose stop semantics make its arguments
  potentially truncated;
- raw partial arguments are not persisted or rendered as safe reasoning; UI
  may receive bounded status/byte-count progress instead;
- provider error frames remain redacted and bounded.

Duplicate or replayed wire events are handled explicitly. An exact duplicate
may be ignored only when the protocol supplies a stable event/item identity and
the assembler proves it is identical; a conflicting duplicate is a protocol
violation. Neither case may execute the same internal call twice. Completed
call ordering follows the provider's content/item order, not map-key sort order,
so parallel result writeback remains deterministic.

A terminally complete turn with a well-identified call but invalid arguments
may produce a correlated typed validation-error result so the model can repair
the call; it never executes the tool. Valid sibling calls may proceed only when
the overall turn is complete and Runtime batch policy permits it. If the turn
ended through length, incomplete, aborted, filtered, or ambiguous semantics,
none of its potentially truncated calls execute even when a best-effort parser
could produce valid-looking JSON.

Wire decoders still understand provider frame shapes. The shared assembler owns
provider-neutral turn correctness so a new adapter cannot accidentally weaken
the rules.

#### 5.6.7 Tool Schema needs a canonical profile and target compiler

The current model schema is raw JSON, passed almost unchanged to every
provider. Runtime argument validation is a hand-written subset supporting
selected `type`, `enum`, required/properties, `additionalProperties=false`,
array bounds/items, string length, and numeric bounds.

Risks:

- unsupported JSON Schema keywords may be ignored by Runtime validation;
- a provider may reject a schema accepted by ToolRegistry;
- a provider may silently enforce less than Runtime expects;
- OpenAI Responses currently sends `strict: false` unconditionally;
- provider/model/gateway support for strict schemas differs;
- duplicate ToolRegistry names silently replace the previous tool;
- tool names, descriptions, root schema shape, schema bytes, and total tool
  count are not validated once against target limits;
- `ToolRegistry` uses HashMap iteration, so model tool order and the Responses
  adapter's own cache-key input can vary even though Runtime's separate tool
  signature is sorted.

The target introduces a **Rove Tool Schema Profile**:

1. Define the accepted JSON Schema dialect/subset and a required object-root
   rule for ordinary function tools.
2. Validate and compile schemas at tool registration or assembly, before a
   provider request.
3. Use a standards-compliant validator, or reject every unsupported keyword;
   never silently accept a constraint Runtime will not enforce.
4. Reject duplicate internal tool names rather than overwrite.
5. Canonically sort tools for signatures, prompt caching, requests, tests, and
   deterministic behavior.
6. Validate name/description/schema byte limits and normalize only at the
   target projection boundary.
7. Maintain a reversible internal-name to exposed-name map when a target's
   syntax requires aliases.
8. Compile canonical schema to a target schema result classified as
   `exact`, `weakened`, or `unsupported` with diagnostics.
9. Support strict policy as `off`, `prefer`, or `require`; `require` fails
   before network use when the target cannot honor it.
10. Cache compiled validators and provider projections by canonical tool
    signature plus protocol/model capability version.
11. Keep destructive, approval, scheduling, and environment metadata out of
    provider schema payloads.

The initial profile should be self-contained: remote `$ref` resolution and
network-loaded schemas are forbidden, reference depth/recursion and regex-like
constraints are bounded, and canonical schema bytes are hashed after stable
normalization. Runtime validation is strict by default. Any argument coercion
or repair must be an explicit tool-owned preparation step with tests and
diagnostics; Provider adapters cannot silently change model arguments to make
them pass execution validation.

Each model request also needs an immutable tool-catalog snapshot containing a
catalog revision, canonical tool signature, schema hashes, and the
canonical-name/target-alias map. An inbound target alias maps back to exactly
one canonical tool in that snapshot. Before execution, Runtime verifies that
the call is still compatible with the advertised descriptor; a removed or
materially changed tool returns a typed stale-catalog error rather than running
different code under the old schema.

Dynamic MCP/extension discovery cannot bypass this rule. Remote MCP names,
descriptions, annotations, and schemas are untrusted inputs: they are bounded,
validated, compiled, collision-checked, and snapshotted before model exposure.
Deferred exposure of an already operator-approved tool is distinct from tool
registration and must produce an explicit catalog revision; neither mechanism
can grant permission.

A schema must not be silently dropped just to make a compatible gateway accept
the request. Any deliberate weakening is visible, bounded, and still followed
by authoritative Runtime validation.

#### 5.6.8 Model and protocol capabilities must be explicit

Provider type alone is not enough. Official endpoints, relays, compatible
gateways, and model generations differ. A target capability snapshot should
cover at least:

- native tool use;
- parallel tool-call support;
- tool-choice modes (auto/none/required/specific tool);
- strict JSON Schema support and known schema restrictions;
- max tools and schema/name/description limits;
- tool result name requirement;
- text/image input and tool-result image support;
- same-model reasoning/signature replay;
- response IDs/deferred responses where implemented;
- context window and output limits;
- supported stop reasons and usage/cache fields.

Capability origin and confidence must be recorded: built-in protocol default,
model catalog, server discovery, operator override, or unknown. Operator
overrides cannot bypass Runtime safety and should fail explicitly when the
wire endpoint disproves them.

Post-CDH product vocabulary also mixes vendor and API selection:
`provider_type=openai` maps to Chat Completions, while
`provider_type=openai-responses` names a separate product type and wire
protocol. Existing persisted profiles make that mapping a compatibility
contract; it must not be silently changed so that `openai` suddenly means
Responses. The target should distinguish:

```text
provider family / credential domain
model identity
wire API or model.api
target capability snapshot
```

Migration may add an explicit API selection or derive one from versioned model
metadata, while retaining legacy profile decoding. Official OpenAI can prefer
Responses for newly created compatible profiles only through an explicit
versioned product decision and migration, not an adapter side effect.

The normalized model request should eventually include explicit tool controls:

```text
tools
tool_choice: auto | none | required | named
parallel_tool_calls: allowed | forbidden
schema_strictness: off | prefer | require
```

Current unconditional or implicit defaults, such as OpenAI Responses
`parallel_tool_calls: true`, should be resolved from Runtime execution policy
and target capabilities. A model may emit multiple calls, but Runtime remains
authoritative about whether they execute concurrently.

This likely requires evolving `ModelClient::stream(messages, tools)` into one
versioned neutral `ModelRequest` carrying context messages, tool definitions,
tool-choice/parallel/strictness controls, request identity, and safe projection
metadata. Raw provider options remain adapter-owned. Model-request extensions
may inspect or transform the neutral request under policy; they cannot receive
resolved secrets or mutate raw provider JSON outside a protocol-specific,
first-party boundary.

#### 5.6.9 Provider switching and routing semantics

There are two different cases:

- **pre-commit routing fallback:** current `RoutingModelClient` may retry or
  switch target before visible text/tool use commits; this remains valid;
- **cross-turn provider/model switch:** persisted history is projected for a
  new target and requires the shared history transformation above.

Required invariants:

- no retry/fallback after committed text, tool call, or unknown provider-side
  effect;
- target selection and capability snapshot are recorded before request;
- projection hash and tool-schema signature are part of prompt diagnostics;
- provider switch never rewrites canonical history;
- target-specific wire IDs and opaque reasoning state are regenerated/stripped
  only in the request projection;
- a failed projection is typed before network execution;
- resume uses the persisted source history plus the currently resolved target,
  not an old serialized wire request;
- same-provider replay optimizations cannot make cross-provider continuation
  impossible.

#### 5.6.10 Provider protocol security and bounds

The provider boundary must additionally enforce:

- max messages, content blocks, tools, schema bytes, call count, ID/name bytes,
  argument bytes, result bytes, and stream duration;
- safe Unicode/surrogate handling before wire serialization;
- no raw provider frame, auth/header, reasoning signature, tool output secret,
  or rejected schema echoed into normal errors/events;
- bounded unknown-frame diagnostics;
- result content/MIME validation before multimodal projection;
- deterministic role/order validation;
- no provider annotation or model-request field granting Runtime permission;
- no adapter-specific filesystem/process behavior;
- zeroization/short lifetime for resolved credentials where supported by the
  existing auth boundary.

#### 5.6.11 Verification matrix

Unit/golden tests are required for every production wire protocol:

| Area | Required cases |
|---|---|
| Tool definitions | empty/non-empty tools, canonical order, duplicate names, invalid root/schema, exact/unsupported strict mode, name alias collision |
| Request mapping | text + multiple calls + success/error results, system projection, target IDs, result names, images/unsupported downgrade |
| Streaming | fragmented/interleaved calls, multiple calls, changing/missing IDs, malformed JSON, oversized args, duplicate end, EOF without terminal, timeout, provider error, length stop |
| Round correlation | zero/one/many results, orphan/duplicate/missing result, unknown effect, approval rejection, schema failure |
| Cross-provider | Responses -> Anthropic, Anthropic -> Responses, Chat -> Anthropic, invalid long/special IDs, same-provider opaque replay, cross-provider stripping |
| Persistence | old TaskState migration, resume, repair, compaction, provider switch after restart, no duplicate tool execution |
| Routing | retry before commit, no fallback after text/tool commit, capability mismatch before request |
| Security | redaction, bounds, unsupported MIME/content, malicious schema, hostile compatible endpoint frames |

The cross-provider matrix must assert the actual outgoing request body and the
next-turn tool result behavior, not only that deserialization succeeded.

#### 5.6.12 Incremental delivery slices

This design is not implemented as one large rewrite. The Provider Tool Protocol
milestone is split into reviewable slices:

1. **PVA - Registration and stream hardening:** reject duplicate/invalid tools,
   sort tool definitions, add accumulated bounds, require terminal stream
   state, normalize stop reasons, and add malformed/incomplete fixtures.
2. **PVB - Typed tool results and identity:** preserve tool name/status,
   distinguish internal versus wire IDs, emit Anthropic `is_error`, and keep
   call/result correlation through persistence.
3. **PVC - Shared history projector:** centralize orphan closure, aborted-turn
   filtering, target ID mapping, system/content compatibility, and
   cross-provider tests. Remove product-only duplicate repair logic only after
   parity is proven.
4. **PVD - Schema compiler and capabilities:** canonical schema profile,
   compiled Runtime validator, strictness policy, tool choice, parallel-call
   policy, target capabilities, deterministic cache signatures.
5. **PVE - Rich content and replay metadata:** ordered content blocks,
   image/artifact result projection, and origin-bound reasoning/signature
   replay only when product scope requires them.

Each slice updates current runtime documentation only after its code and tests
merge. Later slices may refine serialized types, so M1 must seal the migration
envelope before PVB/PVE persist new fields.

Dependency gates are incremental:

- M4 one-kernel implementation requires PVA-PVC, not the optional rich-content
  PVE work;
- M7 Coding Tool V2 requires PVD's authoritative schema validation and request
  controls before its schemas are treated as production contracts;
- PVE is required before multimodal tool results or provider reasoning replay
  are enabled, but does not block unrelated Project Trust or Execution
  Environment work;
- every completed slice is independently tested, documented, and mergeable;
  the umbrella M3 closes only when all slices required by the accepted product
  scope are complete.

---

## 6. Project Trust design

### 6.0 Immediate fail-closed guard

Full Project Trust depends on the M1 contracts and lands in M5, but the current
activation risk cannot wait behind the message/provider/kernel migration. M0.5
must add the smallest compatible guard that prevents an unknown workspace from
activating repository-owned executable behavior.

Until M5 replaces it with the full trust system:

- workspace `.rove/config.toml` is split into a safe bootstrap subset and a
  deferred executable/sensitive subset;
- workspace MCP commands, hooks, executable Skills, external prompt paths,
  provider endpoint/credential-name overrides, and inherited shell policy are
  disabled unless an explicit operator-owned grant exists;
- CLI/API/Web return a typed `trust_required`/restricted state instead of
  silently falling back to activation;
- opening, listing, or safely inspecting a workspace cannot spawn a process;
- compatibility for intentionally configured existing workspaces requires an
  explicit migration/acknowledgement, never implicit trust-by-history.

### 6.1 Threat being addressed

Opening a repository is not consent to execute repository-owned code. The
following inputs are untrusted until the user explicitly trusts the canonical
workspace and, where applicable, approves the executable configuration:

- `.rove/config.toml`;
- `.rove/mcp_servers.json`;
- workspace instructions;
- Skills or hooks found in the workspace;
- task scripts and suggested commands;
- environment files and provider credential references;
- nested repository/worktree configuration.

### 6.2 Trust state

The initial contract should support at least:

```text
unknown
restricted
trusted
revoked
```

- `unknown`: no repository-owned executable configuration is activated.
- `restricted`: bounded read-only inspection may be allowed; mutations,
  processes, MCP, hooks, and workspace Skills remain disabled or require an
  explicit higher-friction decision.
- `trusted`: repository configuration may be considered, but still cannot
  override operator policy or self-grant approval.
- `revoked`: previously granted trust is invalid and all workspace executable
  integrations are disabled until a new decision.

The exact initial UI may expose fewer labels, but persisted semantics must be
unambiguous.

The state label is not itself a blanket grant. Activation grants are granular
and independently revocable for at least workspace instructions, project
configuration, MCP/process definitions, hooks/executable extensions, provider
endpoint/credential selectors, and external paths. Trusting instructions does
not trust an MCP command; trusting one digested MCP definition does not trust a
new or changed definition.

### 6.3 Storage and identity

Trust records must live outside the selected workspace, in operator-controlled
application state. A repository cannot edit its own trust decision.

A record must bind to a canonical workspace identity and include enough
information to detect material replacement. For Repo workspaces this may
include canonical root, repository identity, and relevant filesystem identity.
For Folder workspaces it must not rely only on a user-editable display name.

Executable configuration needs a separate digest/snapshot. A trusted folder
does not imply silent approval of a newly changed MCP command, hook executable,
or external path. Material executable-config changes invalidate or re-prompt
the relevant grant.

Revocation semantics must cover active state, not only the next startup. M5
must define whether each running MCP/process/hook is terminated, quarantined,
or allowed to finish; block new calls immediately; emit a canonical audit fact;
invalidate incompatible resume identity; and preserve evidence without
restarting an unknown effect.

### 6.4 Activation order

```text
select/open path
  -> canonicalize and classify workspace
  -> load only operator-safe/global bootstrap config
  -> resolve trust record
  -> show bounded repository configuration summary
  -> record explicit decision
  -> validate project config under operator ceilings
  -> assemble Runtime and optional executable integrations
```

No stdio MCP child, hook process, workspace Skill executable, or Shell command
may start before this gate.

### 6.5 Surface behavior

- CLI and Desktop must expose the same trust semantics.
- Web cannot assert trust merely because a browser sent a path; the server owns
  workspace identity and trust state.
- Native folder selection returns a candidate path, not an execution grant.
- Trust denial is a normal typed state, not a generic startup failure.
- A user may inspect the exact safe summary of files/commands being requested
  before trusting.
- Trust never exposes raw provider secrets or repository file contents in
  normal logs.

### 6.6 Trust acceptance tests

At minimum, fixtures must prove that an untrusted repository cannot:

- spawn an MCP process;
- replace provider endpoints or credential environment names silently;
- activate a hook or Skill executable;
- escape the workspace with a symlink/junction;
- make a trusted parent implicitly trust an unrelated nested repository;
- retain an executable grant after the relevant config digest changes;
- grant itself tool approval through instructions or metadata.
- keep accepting new calls after the relevant grant is revoked;
- use a trust decision through an unauthenticated/cross-origin API path;
- alias a trusted workspace through case, junction, symlink, worktree, drive,
  UNC, or directory-replacement identity tricks.

---

## 7. Rove Execution Environment

### 7.1 Purpose

The Execution Environment separates **what a tool intends to do** from **where
and how the operation is carried out**. It is a Runtime-supplied,
invocation-scoped capability backend.

It is not:

- a wrapper around one `PathBuf`;
- an alternative approval system;
- a promise that container or remote execution already exists;
- a place for provider or UI logic;
- a way for tools to request undeclared authority.

### 7.2 Conceptual structure

Exact Rust names are sealed in M1, but the target shape is:

```rust
pub trait ExecutionEnvironment: Send + Sync {
    fn identity(&self) -> &ExecutionEnvironmentIdentity;
    fn filesystem(&self) -> &dyn WorkspaceFileSystem;
    fn processes(&self) -> &dyn ProcessHost;
    fn artifacts(&self) -> Option<&dyn ArtifactSink>;
    fn capabilities(&self) -> &ExecutionCapabilities;
}
```

The initial implementations should be:

- `LocalWorkspaceEnvironment`: real local filesystem/process adapter bounded
  to one canonical workspace;
- `InMemoryExecutionEnvironment`: deterministic tests with no host process
  authority;
- `WorktreeExecutionEnvironment`: either a configured local adapter with a
  distinct identity or a dedicated wrapper when Worktree lifecycle requires
  additional guarantees.

Container and remote adapters remain future implementations. The interfaces
must not claim they work until tested end to end.

### 7.3 Invocation flow

```text
model tool call
  -> ToolRegistry schema/argument validation
  -> Runtime policy + Project Trust + approval
  -> Runtime ToolExecutor
  -> ToolInvocationContext { env, cancellation, identity, event sink }
  -> tool expresses an operation
  -> environment adapter performs the bounded backend action
  -> typed result + mutation/process/artifact metadata
  -> canonical events, state, model projection, UI projection
```

Tools receive the environment through invocation context. A registered tool
must not be permanently tied to one session root or retain ambient host
authority between calls.

### 7.4 WorkspaceFileSystem responsibilities

The filesystem port must own backend-specific behavior for:

- workspace-relative path parsing and normalization;
- canonical identity and symlink/junction/reparse-point containment;
- bounded metadata and directory listing;
- true byte-range and line-range reads;
- streaming reads and bounded hashing;
- text encoding/BOM and line-ending observation;
- file version/observation identity;
- atomic create and replace;
- compare-and-swap replacement;
- temporary file creation within a safe boundary;
- fsync/flush policy where required;
- mutation serialization per canonical path;
- special-file and denied-internal-path rejection;
- cleanup with typed partial/unknown outcomes.

Path containment must remain enforced in Runtime policy and in the local
adapter as defense in depth. A path validated before an operation cannot be
blindly trusted after symlink or identity changes.

#### 7.4.1 Observation store

Read/Edit stale protection needs a Runtime-owned observation store; an opaque
token alone is not a contract. M1/M6 must seal and implement:

- run/session/workspace/environment binding and a non-forgeable opaque ID;
- canonical path/file identity, observed version/hash/range, encoding, and
  line-ending metadata without storing unbounded file content;
- maximum observations per run, metadata bytes, TTL, invalidation, and cleanup;
- invalidation after mutation, workspace/environment change, trust revocation,
  and incompatible resume;
- whether exact observations survive restart; unsupported persistence must fail
  stale rather than accepting a model-supplied hash;
- concurrency semantics when two calls observe or mutate the same path;
- redaction and denial for secret/internal paths.

### 7.5 ProcessHost responsibilities

The process port must own:

- explicit executable/shell request shape;
- canonical working directory;
- environment inheritance and allowlisting;
- process identity;
- stdout/stderr streaming;
- bounded model-visible windows plus a bounded durable spool;
- foreground wait and background detach semantics;
- timeout, cancellation, terminate, and kill escalation;
- status polling and completion notification;
- child-tree cleanup;
- exit metadata and typed unknown-effect state;
- host-specific quoting and error normalization.

Stdio MCP process creation must eventually use this port or a narrower
Runtime-owned process-transport port backed by it. It must not remain a separate
ambient process authority.

### 7.6 Artifact integration

CDH landed a bounded product artifact manifest/download/preview surface over
existing run files and registered artifact files. Environment operations must
reuse or extend that Runtime-owned storage and its opaque IDs rather than create
another product artifact system. Large Diff/output/checkpoint content may flow
through an injected sink, but it remains workspace/session bound,
authenticated, bounded, redacted, retained by policy, and referenced by opaque
IDs.

This does not mean MCP Tool Artifacts are implemented. Rich MCP result blocks,
resource metadata, model projection, MIME handling, and transport/session
semantics remain governed by the proposed MCP design and require a separate
accepted implementation slice.

### 7.7 Environment identity and resume

Runs and checkpoints need a stable, redacted environment identity sufficient
to detect an incompatible resume target. It may include environment kind,
workspace identity, and adapter/schema version, but never raw secrets.

Resume rules:

- completed file mutations are not replayed;
- unknown process effects are not restarted automatically;
- a background process that cannot be reattached becomes a typed lost/unknown
  state;
- a changed environment identity requires explicit compatible migration or
  fail-closed resume;
- switching from local to container/remote is not silently treated as the same
  execution environment.

---

## 8. Coding Tool V2 contracts

### 8.1 Read V2

Conceptual request:

```text
read_file
  path
  start_line? or byte_offset?
  line_limit?
```

Runtime policy owns hard byte, line, time, and output caps. Model arguments may
request a smaller range but cannot raise those caps.

Conceptual result:

```text
path
content
returned line/byte range
file size when available
encoding/BOM/line-ending metadata when relevant
truncated
continuation position
observation_id / file_version
```

Required behavior:

- the backend performs a true bounded/ranged read; it must not read the entire
  file merely to truncate model output;
- line numbers are stable and useful for a subsequent targeted read/edit;
- truncation is explicit and provides a continuation;
- binary, image, special, oversized, invalid-encoding, missing, and changed
  files produce typed outcomes;
- returned content is bounded before it becomes model context, an event, a
  report, or a UI object;
- the Runtime stores the authoritative observation; the model cannot invent a
  valid observation token;
- a full-file hash, when needed, is computed with bounded streaming and is not
  inserted into model context as file content.

### 8.2 Exact Edit V2

Initial conceptual request:

```text
edit_file
  path
  old_text
  new_text
  observation_id
  replace_all = false
```

Required invariants:

1. `old_text` and `new_text` cannot be identical.
2. Strict exact matching is attempted; no implicit whitespace/fuzzy rewrite is
   allowed in the first version.
3. Zero matches fail without mutation.
4. Multiple matches fail without mutation unless `replace_all` is explicitly
   true.
5. A normal single edit must refer to text observed in a prior bounded Read.
6. The authoritative file version is checked again immediately before write.
7. The match count may be established by streaming the full file inside the
   backend. This is filesystem I/O, not model context expansion.
8. Mutation of one canonical path is serialized across concurrent calls.
9. Replacement is atomic where the backend supports it and otherwise reports
   the weaker guarantee explicitly.
10. BOM, encoding, line endings, and final-newline semantics are preserved.
11. A real localized unified Diff is computed from actual before/after content.
12. The model-visible Diff is bounded; a complete large Diff uses the existing
    post-CDH artifact contract when available.
13. Checkpoint/preimage metadata is recorded before mutation according to a
    bounded retention policy.
14. A stale observation, identity change, permission failure, or post-open path
    change fails without claiming success.

The model should normally provide the smallest clearly unique original region,
often two to four adjacent lines. `replace_all` is explicit for intentional
renames or global replacement.

Claude Code's observable exact replacement discipline is a behavior reference,
not a requirement to copy its full-read prerequisite. rove should permit a
bounded prior Read while retaining stale-version and uniqueness guarantees.

Quote normalization or another narrow convenience fallback may be considered
later only as an explicit, tested mode. It must not weaken the strict default.

### 8.3 Write/Create V2

`write_file` should primarily create new files. For an existing file:

- whole-file overwrite is not the default coding edit path;
- overwrite requires an explicit operation and expected version;
- destructive approval follows policy;
- create fails on unexpected existence unless explicitly configured;
- parent creation, encoding, atomicity, and mutation metadata are typed;
- content and Diff limits are enforced before persistence and model/UI
  projection.

Legacy whole-file write compatibility may remain temporarily for existing API
or benchmark fixtures, but it must be visibly deprecated and cannot be the
prompted default tool for modifying source files.

### 8.4 File lifecycle operations

Real coding work also requires bounded lifecycle operations, not only create and
edit:

- `delete_file` requires an observed expected identity, destructive approval,
  a checkpoint/non-reversible result, and typed missing/changed/partial states;
- `move_file`/rename validates and observes both source and destination, refuses
  unexpected overwrite by default, preserves containment and metadata, and
  reports whether the backend supplied atomic rename;
- directory creation/removal is explicit, bounded, and never recursively
  deletes through an unresolved model path;
- cross-filesystem moves, case-only renames, Windows sharing violations,
  symlink/junction/reparse points, executable bits, permissions/ACLs, BOM,
  line endings, and final-newline behavior have platform-specific conformance
  tests;
- multi-file operations report per-path outcomes and cannot describe partial
  mutation as an atomic success.

### 8.5 File checkpoints and rewind

The target supports bounded recovery from Agent-created file mutations without
treating `report.json` as the source of truth.

M1 seals the design and M7 implements the accepted first version, including:

- whether a checkpoint stores a preimage, reverse patch, or artifact reference;
- per-file and per-run byte limits;
- secret/internal-file exclusions;
- checkpoint ordering with canonical tool events and TaskState;
- restart and cleanup behavior;
- conflict behavior when the user edits a file after the Agent mutation;
- whether rewind is a new approved mutation rather than history deletion.

Rewind cannot silently overwrite newer user work and cannot delete canonical
evidence that the original mutation happened.

### 8.6 Shell V2

Shell execution must support both bounded foreground commands and durable
background observation.

Conceptual operations:

```text
start_process(command, cwd?, mode)
poll_process(process_id, cursor?)
send_process_input(process_id, data)
terminate_process(process_id)
```

The public tool set may use different names, but the backend lifecycle must
provide:

- a stable process identity;
- progressive stdout/stderr chunks with sequence/cursor identity;
- bounded model-visible output;
- retention of additional output in a bounded spool/artifact rather than
  silent discard;
- explicit foreground timeout versus background continuation;
- completion notification represented as a typed session/application entry and
  canonical event, not a fabricated user message;
- process-tree termination and cleanup;
- no automatic restart of unknown side effects after cancellation or resume;
- a clear Desktop close/quit policy;
- typed unsupported behavior for adapters that cannot provide a feature.

M1 must decide whether the first production ProcessHost supports a PTY. If PTY
is deferred, interactive full-screen programs and terminal-emulation claims are
explicit non-goals; `send_process_input` remains bounded pipe input and must not
be presented as terminal compatibility.

A string denylist remains defense in depth, not the security boundary. Project
Trust, Runtime approval, environment capabilities, workspace bounds, and
operator policy remain authoritative.

### 8.7 Search, listing, and discovery

`search_code`, directory listing, glob/discovery, artifact readers, and future
file tools must use the same `WorkspaceFileSystem` and observation/path
semantics. They cannot become alternate direct host-filesystem paths.

The initial search/list contract must specify and test:

- literal versus regex modes, pattern/schema byte bounds, timeout, file-count,
  match-count, result-byte, depth, and concurrency limits;
- include/exclude globs, hidden files, repository ignore behavior, binary and
  invalid-encoding handling, and whether ignored files can be requested only by
  an explicit bounded option;
- stable deterministic ordering, line/range metadata, truncation reason, and a
  continuation cursor that is invalidated by incompatible workspace changes;
- symlink/junction containment, special-file rejection, permission errors, and
  per-entry partial diagnostics without leaking secret file content;
- consistent behavior for local and in-memory adapters.

### 8.8 Tool-result projection and context reclamation

Tool output has at least three consumers with different needs:

- the next model turn;
- durable evidence/reporting;
- UI inspection.

One raw string cannot serve all three indefinitely. Tool results need typed
metadata and separate bounded projections. Context reclamation should first
drop or replace obsolete large tool-result bodies while keeping identity,
summary, mutation, and artifact references; semantic compaction runs only after
that cheaper reclamation is insufficient.

---

## 9. Execution strategy, controls, Subagents, and background work

### 9.1 Strategy selection

The target strategy contract is explicit:

```text
react
plan_react
auto   # optional only after its behavior is specified and evaluated
```

Selection precedence should be:

1. explicit per-run user/API request within operator caps;
2. exact resume of the persisted prior strategy;
3. product/session default;
4. lightweight default `react`.

PlanReact is appropriate for explicit plan mode, longer multi-stage tasks,
tasks requiring durable step evidence, and exact resume of a planned run. An
ordinary edit or question should not pay for a mandatory planning call.

Any later `auto` mode must record why a strategy was selected, remain bounded,
and degrade to ReAct without pretending that a failed planner produced a plan.

### 9.2 Steer and Follow-up

Post-CDH Steer/Follow-up contracts must be adapted to the one kernel:

- one control inbox contract;
- one safe-boundary rule;
- one idempotent receipt/application path;
- one cancellation/restart policy;
- no planned/unplanned duplicate application code.

Runtime owns durable receipts and safe policy. Core owns the in-memory point at
which a control may affect the next model turn. An unapplied control cannot be
smuggled into resume as an ordinary message.

### 9.3 Future runtime Subagent feature

This subsection describes a possible rove product capability only. It does not
authorize the coding Agent executing this plan to call, create, resume, or
delegate to a Subagent. Section 0 permits separate user-opened top-level
conversations in the two owned worktrees; it does not permit either
conversation to create child Agents, including for any separately accepted M8C
scope.

Subagents are a later consumer of the same kernel, not a second bespoke loop.
The first design must require:

- independent context and typed session scope;
- explicit parent task and call identity;
- bounded budget and concurrency;
- a restricted Execution Environment capability set;
- no inherited approval or trust escalation;
- typed result returned to the parent projection;
- no raw child transcript injection by default;
- cancellation and unknown-effect semantics;
- canonical provenance and artifact references;
- Worktree isolation only through a Runtime-created environment, never a path
  supplied by model text.

### 9.4 Background Agent and process work

Background completion is represented through typed entries/events and a
notification projection. It is not appended as a fake user instruction.

Background work must have:

- durable identity and owner session/run;
- explicit lifecycle and output cursor;
- a bounded retention/cleanup policy;
- attention/completion state visible to all product surfaces;
- no silent replay after restart;
- confirmation before continuing from an uncertain predecessor effect.

### 9.5 PlanEvaluator, Finalizer, and trace-tail reconciliation

The one-kernel migration must finish the accepted execution-lifecycle design,
not freeze its current partial implementation:

- `PlanEvaluator` remains rule-first and uses a bounded model call only for
  genuinely ambiguous decisions;
- `Replanner` changes only future work and cannot rewrite StepRecords;
- `plan_react` uses an independent evidence-grounded Finalizer for success,
  partial, blocked, failed, and budget-exhausted outcomes;
- Finalizer model failure produces a deterministic ledger-based answer and does
  not change the underlying completion status;
- Finalizer never treats procedure adherence or assistant prose as proof of a
  successful mutation;
- state repair/resume reconciles canonical trace facts newer than the latest
  materialized TaskState snapshot before deciding what may run again;
- cancellation at Planner, evaluator, tool wait, replanner, and Finalizer emits
  exactly one terminal lifecycle fact.

### 9.6 Multidimensional budgets and resource quotas

`max_steps` is only a compatibility input. M1 defines and M4 implements a
run-pinned budget envelope covering at least model turns, tool calls, plan step
attempts/revisions, Subagent depth/count/concurrency, background processes,
wall time, tokens, provider cost when known, model-visible output, durable spool
bytes, artifact/checkpoint bytes, and reserved Finalizer capacity.

Budget accounting is global across Planner, kernel turns, evaluator, replanner,
Finalizer, Subagents, and retries. Per-step or per-Agent defaults cannot exceed
operator caps. Limits and consumption enter runtime identity, checkpoint,
events, report, config dump, API/UI diagnostics, and resume compatibility.
Waiting for explicit human approval/input has a separately recorded policy and
cannot silently consume or pause every budget dimension.

---

## 10. AgentDefinition, workspace instructions, and procedures/Skills

This plan schedules implementation after the kernel, message, trust, extension,
and environment contracts. The detailed source design remains
[`docs/design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`](../../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md).

### 10.1 AgentDefinition and runtime profile

M8B must not implement Skills without the identity and authority contract that
makes them reproducible. It delivers:

- schema-versioned, named, versioned `AgentDefinition` packages with explicit
  source selectors and no search-order shadowing;
- validation of manifest, referenced paths/content, runtime compatibility,
  capability requirements, prompt slots, size limits, secrets, and the ban on
  executable auto-hooks;
- a legacy profile mapping for the current prompt/config behavior;
- one immutable `AgentRuntimeProfile` per run with definition, instruction,
  prompt-slot, capability, procedure-catalog, memory-policy, degradation, and
  content hashes;
- runtime identity, events, checkpoints, reports, repair, resume, config dump,
  API, and UI diagnostics for the resolved profile;
- explicit failure for an invalid explicit selector and a typed, policy-owned
  degradation path for an invalid default selector;
- operator caps that Agent defaults can tighten but never raise.

### 10.2 Workspace instructions

Required behavior:

- deterministic parent-to-child discovery and scope;
- explicit supported filenames and compatibility aliases;
- nearest-scope precedence without erasing higher-authority Runtime policy;
- bounded file count, individual bytes, aggregate bytes, depth, and links;
- provenance and trust labels on every loaded segment;
- no activation before Project Trust allows workspace instructions;
- no permission grant through prose;
- prompt insertion through the typed context pipeline, not ad hoc string
  concatenation in Engine;
- conflict diagnostics that do not expose secrets;
- exact snapshot/version metadata for resume reproducibility.

### 10.3 Procedures and Skills

Required progressive disclosure:

```text
startup/context catalog: Skill name + bounded description + provenance
selection: metadata and eligibility
hydration: full instructions/resources only when chosen
execution: capability references resolved under Runtime policy
```

Skills remain distinct from memory, tools, workspace instructions, and
reference retrieval. A Skill can describe a procedure but cannot register a
tool, execute a command, or raise approval solely through its text.

The first implementation uses the procedural-knowledge design rather than an
untyped directory of prompts. Machine-readable metadata must cover source,
version/hash, trust derived from source/operator policy, applicability,
platform, required capability IDs, risk/effects, freshness, validation, and
rollback/non-reversible behavior. Eligibility filtering always precedes
lexical/model ranking. `no_match`, `degraded`, and `error` are distinct outcomes.
Only selected summaries/outlines enter planning; StepRunner hydrates bounded
sections by pinned hash and records application/deviation. Self-authored updates
remain untrusted candidates until a later explicit review boundary.

### 10.4 Integration points

Skills and workspace instructions must use:

- the extension registry for discovery/contribution lifecycle;
- Project Trust for activation;
- typed session/context entries for provenance;
- context budgets and compaction rules;
- Execution Environment only through approved tools;
- RuntimeIdentity/checkpoint metadata for reproducible resume;
- canonical events for safe selection/hydration facts where observability is
  required.

---

## 11. Agent capability evaluation

### 11.1 Evidence gap

The existing deterministic benchmark proves that model, tool, state, trace,
report, cancellation, and resume plumbing can run. It does not by itself prove
that the Agent reads economically, edits correctly, recovers from errors, or
finishes realistic coding tasks.

### 11.2 Baseline before migration

M0/M1 must record the pre-change baseline on fixed local repositories and Fake
provider scripts. The baseline includes:

- task success/failure;
- bytes and lines returned to model context;
- model turns and tool calls;
- incorrect or over-broad mutations;
- Diff quality;
- plan calls on simple tasks;
- cancellation/resume behavior;
- wall time and peak output size where deterministic measurement is possible.

### 11.3 Required deterministic coding scenarios

The V2 suite must include at least:

1. locate and read a small relevant range in a large file;
2. perform one unique exact edit;
3. reject a missing old string without mutation;
4. reject a non-unique old string without `replace_all`;
5. reject a stale observation after an external edit;
6. preserve BOM/line endings/final newline;
7. create a new file without overwriting an unexpected existing file;
8. produce a localized Diff and mutation record;
9. stream and poll a long command while retaining bounded complete output;
10. cancel a command and avoid unknown-effect replay;
11. deny hostile untrusted workspace MCP/config activation;
12. run the same file tools against an in-memory environment;
13. resume old TaskState fixtures without duplicate mutations;
14. apply Steer/Follow-up through the one kernel;
15. trim obsolete tool bodies before semantic compaction;
16. select ReAct without a planner call for a simple coding task;
17. preserve planned ledger behavior for an explicit long task;
18. project one failed tool execution to Anthropic with explicit error status;
19. switch a persisted tool round between Responses and Anthropic while
    preserving internal correlation and target-valid wire IDs;
20. reject clean EOF, length truncation, malformed JSON, or oversized arguments
    before any incomplete tool call executes;
21. reject unsupported/invalid Tool Schema at registration or target projection
    instead of silently ignoring constraints;
22. preserve deterministic tool order, signature, and request cache identity
    across process restarts;
23. delete and rename observed files without traversal, unexpected overwrite,
    or loss of newer user work;
24. return bounded, deterministic search/list pages with valid continuations;
25. exhaust each global budget dimension without starting one extra model,
    tool, Subagent, or process action;
26. produce evidence-grounded Finalizer output and deterministic fallback;
27. reconcile a trace tail newer than TaskState without replaying a mutation;
28. resolve, pin, persist, and resume one AgentRuntimeProfile plus root
    `AGENTS.md` and one procedure/Skill without authority escalation;
29. revoke trust while an MCP/process capability is active and block new work;
30. run filesystem/process conformance on Windows and Linux for every claimed
    local adapter guarantee.

### 11.4 Real-model evaluation and claim levels

Provider-backed evaluation supplements deterministic tests. It must use
versioned fixtures, bounded cost, exact provider/model metadata, and explicit
credentials. A skipped provider gate proves only the skip path and cannot be
used to claim interoperability or Agent quality.

Evidence is reported at distinct levels:

- **contract-complete:** deterministic Fake/fixture tests prove runtime,
  protocol, migration, safety, and tool semantics;
- **provider-interoperable:** a named provider/model passes the bounded native
  request/stream/tool/resume matrix;
- **coding-capable:** a named provider/model passes the versioned coding suite
  across repeated runs and adversarial variants;
- **Claude Code-class claim:** requires an accepted comparative methodology and
  thresholds; this plan does not make that claim from workflow similarity.

The program may complete at contract-complete when credentials are unavailable,
but release notes and README must use that narrower claim. A supported-model
coding-quality claim requires the corresponding real-model gate to run.

### 11.5 Quality metrics

At minimum report:

- task completion rate;
- exact file assertions;
- unintended mutation count;
- stale/conflict safety failures;
- context bytes/tokens by source;
- tool calls and model turns;
- planning overhead;
- Diff precision;
- resume duplication count;
- trust/approval negative-test pass rate;
- output retained versus model-visible output;
- latency and provider cost when applicable.

M1/M10 must set numeric non-regression thresholds before implementation results
are visible. Required safety thresholds are zero unintended mutation, zero
approval/trust bypass, zero duplicate mutation on resume, and zero external-to-
trusted authority escalation. Quality thresholds include a declared completion
rate floor, maximum context/tool/planning overhead regression, Diff precision,
and repeated-run variance. Changing a threshold after results are known requires
a recorded rationale and a new baseline.

---

## 12. API, ProductStore, schemas, and UI composition

### 12.1 Application service boundary

M1 uses the M0 post-CDH concentration inventory before authorizing refactoring.
The target is
thin transport handlers over explicit application services, potentially
including:

- workspace/trust service;
- product session and lineage service;
- job/run supervisor;
- control receipt service;
- transcript/session projector;
- provider/session-default service;
- resource/artifact service;
- evidence export service.

This is not authorization for a broad module shuffle. A service is extracted
only when it owns a real transaction, invariant, or testable policy and reduces
cross-layer duplication.

### 12.2 Public schema workflow

M1 must choose the workflow before M2 begins emitting new serialized entries,
events, or public API fields. M9 completes consumer migration and cleanup. The
accepted workflow is one of:

- generated Web types/clients from the canonical OpenAPI schema;
- generated shared schema artifacts with verified Rust and TypeScript
  consumers;
- retained manual types plus strict automated conformance tests.

The decision must account for defaults, tagged unions, 64-bit values, dates,
SSE events, migration compatibility, and reviewability. Code generation is not
adopted merely to reduce typing if it obscures safety-critical semantics.

### 12.3 UI extension boundary

The product UI may register first-party typed renderers for known message,
tool, resource, and artifact descriptors. Workspace content cannot ship
executable React/JavaScript/native UI extensions in this program.

Unknown entry/result types remain visible through a bounded safe fallback with
identity and provenance; they must not disappear or be rendered as success.

---

## 13. Deferred Desktop relationship

### 13.1 Post-CDH reconciliation

M0 confirmed that no `apps/desktop` host exists. Desktop implementation is not
authorized by this plan and is not part of M0-M10 completion. A future D0 plan
may begin only after the kernel, Project Trust, Execution Environment, and
Coding Tool V2 contracts are merged and stable. A docs/prototype-only host spike
may validate build constraints but cannot create product authority.

### 13.2 Desktop invariants

- Tauri/native commands are narrow host capabilities, not an alternate API.
- Native folder selection returns a candidate workspace and enters Project
  Trust; it never directly starts an Agent or MCP process.
- Runtime lifecycle, readiness, crash, restart, close, quit, and background
  process behavior are explicit.
- Stable app data, logs, trust records, ProductStore, and workspace state have
  documented locations and migration.
- Secrets use an OS-appropriate server/native boundary and never enter WebView
  state.
- Navigation, custom protocols, CSP, local HTTP, command allowlists, updater,
  installer, signing, and notarization are independently reviewed.
- Web reports unsupported native capabilities honestly.
- Desktop cannot bypass API auth, approval, canonical events, workspace
  identity, or environment capabilities.

### 13.3 Future D0 release gate

A downloadable build is not a usable release merely because it launches. The
future Desktop release gate requires:

- Project Trust hostile-repository tests;
- workspace/path and local protocol negative tests;
- embedded Runtime startup/restart/cleanup evidence;
- file Edit and background Shell end-to-end scenarios;
- session continuation and conservative resume;
- no orphan MCP/process tree after normal quit or crash recovery;
- installer/uninstaller and upgrade/migration checks on the first supported OS;
- secret and log inspection;
- signed artifact/provenance decisions appropriate to release scope;
- complete current documentation and an explicit unsupported-platform matrix.

---

## 14. Delivery order

No implementation branch for a later milestone starts before its dependency
contract is merged. The dependency graph may identify disjoint worktree tracks,
and Section 0 permits their top-level conversations to run concurrently only
after the bootstrap gates and ownership manifest are merged.

### Dependency-gated two-worktree schedule

After this plan is merged, create these two isolation worktrees from the same
then-current `origin/main` and record the exact SHA:

| Worktree | Suggested path/branch family | Primary ownership |
|---|---|---|
| A | `.worktrees/agent-kernel` / `feature/agent-kernel-*` | `models/`, `core/`, typed message/projection, provider protocol slices, kernel/lifecycle |
| B | `.worktrees/agent-trust-tools` / `feature/agent-trust-tools-*` | Project Trust, Execution Environment adapters, Coding Tool V2, focused bootstrap integration |

Bootstrap is ordered; each checkpoint is merged and verified on `main` before
the next one begins:

1. B: M0.5 immediate trust guard.
2. A: M1 design/contract seal and coordinator-owned ownership manifest.

After M1 merges, the following two lanes are concurrency-eligible under the
manifest. Each lane remains internally ordered and uses separate merge
checkpoints:

1. A: M2, then M3A-M3C.
2. B: create its next milestone branch from the new `main`; implement M5, then
   the M6 parity migration.

After those lanes merge, continue through the remaining dependency gates:

1. A: start from the new `main`; implement M4 and M3D in the dependency order
   sealed by M1.
2. B: start from `main` containing M3D and M6; implement M7.
3. A: implement M8A and M8B as separate checkpoints.
4. B or a short coordinator branch: implement M9; use `main` for the final M10
   evidence/documentation seal only after all implementation PRs merge.

When one lane is blocked, the other dependency-independent lane may continue.
Every top-level conversation remains confined to its worktree and may never use
a Subagent. The coordinator serializes merges and refreshes both worktrees from
the resulting `main` before dependent work resumes.

Shared hotspots are assigned by the M1 manifest rather than by this coarse
track table. Until then neither worktree may independently change canonical
events, TaskState schema, ProductStore migrations, generated OpenAPI/Web types,
root dependency/lock files, acceptance reports, or current runtime docs.

### M0 - Post-CDH truth and safety reconciliation

Status: **Sealed on 2026-08-06.** The coordinator handoff in Section 19 records
the merge, baseline, cleanup, verification, and first implementation boundary.

Deliverables:

- record the actual CDH merge SHA and merged PR/branch evidence;
- inspect post-CDH `git status`, source, tests, generated schemas, migrations,
  docs, and any Desktop host;
- run the aggregate gates required by CDH;
- inventory canonical events, TaskState, ProductStore, controls, lineage,
  artifacts, export, host capabilities, and public types;
- re-check every diagnosis in Section 3;
- identify user-owned/untracked files and active retained worktrees;
- capture the deterministic coding baseline;
- amend this plan where merged reality differs.

Exit gate:

- Main is the sealed implementation baseline;
- current docs describe CDH accurately;
- no CDH behavior is accidentally assigned for duplicate implementation;
- failures and optional unrun gates are recorded honestly.

### M0.5 - Immediate Project Trust guard

This is a narrow security change before the broad architecture migration. It
must not grow into the full M5 UI/store program.

Deliverables:

- classify workspace config fields into safe bootstrap versus deferred
  executable/sensitive activation;
- prevent an unknown workspace from starting MCP/process/hook behavior or
  applying executable prompt/provider overrides;
- return a typed restricted/trust-required state across CLI/API/Web paths;
- add hostile-workspace negative fixtures and compatibility behavior for
  existing explicit configurations;
- record the temporary guard and its M5 replacement/migration contract.

Exit gate:

- opening or inspecting an unknown repository starts no repository-owned
  process and cannot change provider credential/endpoint authority;
- the guard is fail-closed, does not self-grant through workspace prose, and
  preserves the current explicit operator-config path;
- M5 has an explicit migration from the temporary guard.

### M1 - Kernel, message, provider, extension, trust, and environment design seal

This is the architecture-first milestone. It is design, characterization, and
contract work before broad implementation.

Deliverables:

- seal the one-kernel ownership and Runtime ports;
- seal typed session/application entry and projection contracts;
- seal the provider-neutral AssistantTurn/ToolCall/ToolResult, stop-reason,
  internal/wire identity, history projection, and Tool Schema migration
  envelope from Section 5.6;
- seal legacy TaskState migration;
- seal extension ordering/failure/authority semantics;
- seal Project Trust identity and activation flow;
- seal `ExecutionEnvironment`, `WorkspaceFileSystem`, and `ProcessHost` ports;
- seal the observation store, file lifecycle, search/list, PTY support decision,
  and product-artifact versus MCP-Tool-Artifact boundary;
- seal remaining execution-lifecycle contracts: model-on-ambiguity evaluation,
  Finalizer/fallback, multidimensional budgets, and trace-tail reconciliation;
- seal AgentDefinition/AgentRuntimeProfile/procedure capability and identity
  contracts, including the legacy profile path;
- publish the canonical truth/derivation matrix for canonical events,
  TaskState, typed session entries, ProductStore rows, UI read models, reports,
  and artifacts, including ordering, idempotency, and causality keys;
- choose the Rust/OpenAPI/Web public schema workflow before M2 changes public
  serialized contracts;
- define compatibility, public API/event impact, feature flags, differential
  cutover evidence, and rollback floors;
- set numeric benchmark and safety non-regression thresholds before new results
  are measured;
- add characterization tests around all loop/control/resume/tool behaviors that
  will move;
- update or supersede every row in the Section 1.5 design-adoption matrix;
- commit the two-worktree ownership manifest: base SHA, branch names, allowed
  files, shared forbidden hotspots, required tests, and merge order.

Exit gate:

- no unresolved owner exists for model iteration, tool iteration, control
  application, context transformation, execution backend, durable truth,
  budget accounting, profile/procedure identity, or public schema generation;
- migration and rollback are reviewable before serialized code changes;
- security checklist is complete for the target contracts;
- worktree A and B have disjoint owned files and can be operated concurrently
  without ambiguous ownership of canonical events, migrations, generated
  schemas, or lockfiles;
- every concurrency-eligible checkpoint has explicit branch, base, test,
  handoff, and shared-hotspot rules.

### M2 - Typed message/session projection

Ownership boundary: M2 defines the provider-neutral types, canonical derivation,
serialized migration, and dual-reader/writer compatibility. It does not change
provider wire payload behavior. M3 consumes these types and owns wire
projection, stream assembly, target aliases, and schema/capability compilation;
it must not introduce a second persisted identity or result type.

Deliverables:

- add typed session/application entries and provenance;
- add Session-to-Agent and Agent-to-model projection;
- represent ordered assistant content, normalized stop reason, stable internal
  tool-call identity, and typed ToolResult name/status/content without exposing
  provider payloads;
- separate UI transcript projection from model context projection;
- migrate TaskState/checkpoint/report readers and writers compatibly;
- preserve canonical events as truth;
- implement the M1 truth/derivation matrix with idempotency, ordering, causality,
  size, redaction, and unknown-variant behavior;
- route existing context/memory/compaction through typed context segments;
- add old-state fixtures, repair, and resume tests.

Exit gate:

- background/control/Skill/Subagent-shaped fixture entries do not require fake
  model roles;
- existing sessions resume without lost or replayed mutations;
- model payload tests prove only projected model messages cross the provider
  boundary.

### M3 - Provider Tool Protocol V2

M3 is tracked and merged as explicit submilestones rather than one long-lived
umbrella branch:

- **M3A/PVA:** registration, canonical ordering, accumulated bounds, terminal
  stream state, and normalized stop reasons;
- **M3B/PVB:** typed result/error semantics and internal/wire identity mapping
  using the M2 storage contract;
- **M3C/PVC:** one target-aware history projector and parity removal of duplicate
  product repair logic;
- **M3D/PVD:** canonical Tool Schema compiler/validator, request controls, and
  target capability snapshots;
- **M3E/PVE:** rich content and origin-bound replay metadata only if M1 accepts
  that product scope. Otherwise it is an explicitly deferred follow-up.

M4 may start after M3A-M3C merge. M7 cannot start until M3D merges. Each
submilestone has its own PR, documentation update, rollback note, and gates.

Deliverables:

- deliver M3A-M3D and any M1-accepted M3E scope from Section 5.6.12 as
  separately reviewable slices;
- harden tool registration, deterministic ordering, stream terminal state, and
  accumulated tool-call bounds;
- retain tool result name/status and map error semantics correctly for each
  provider;
- separate stable internal call IDs from origin-bound wire IDs;
- centralize target-aware history projection and remove duplicated repair only
  after parity tests pass;
- define and enforce the canonical Tool Schema profile and Runtime validator;
- add target capability snapshots, strictness, tool-choice, and parallel-call
  request controls as their slices require;
- add request/stream/cross-provider/persistence/routing/security test matrices
  for OpenAI Responses, OpenAI Chat Completions, Anthropic Messages, and the
  applicable Ollama/external-adapter paths;
- preserve provider payload isolation and pre-commit-only fallback.

Exit gate:

- failed/rejected/unknown tool outcomes cannot be projected as successful
  Anthropic results;
- provider switching rewrites target wire IDs without changing canonical
  history or breaking call/result correlation;
- incomplete, truncated, malformed, duplicated, or oversized streamed calls
  cannot execute;
- ToolRegistry rejects duplicate/invalid definitions and request ordering/cache
  signatures are deterministic;
- unsupported schema/capability combinations fail before network use or use an
  explicit reviewed weakening policy;
- the cross-provider request-body and restart/resume matrix passes.

### M4 - One Agent kernel and extension plane

Entry gate: M2 plus M3A-M3C are merged. M3D/M3E may continue independently
where their accepted dependencies do not overlap the kernel-owned files.

Deliverables:

- define or evolve the Core kernel ports;
- make Runtime React execution delegate to the Core kernel;
- make each planned step use the same kernel under bounded scope;
- unify Steer, Follow-up, cancellation, tool-result writeback, and final action
  handling;
- remove or reduce duplicate Runtime loop mechanics;
- add the internal extension registry and first context/model/compaction/session
  hooks;
- retain Tool hooks through an adapter or compatible migration;
- implement global budget accounting and reserved Finalizer capacity across the
  shared kernel/lifecycle;
- complete rule-first/model-on-ambiguity evaluation, independent Finalizer with
  deterministic fallback, and trace-tail reconciliation;
- run differential old/new trace-and-outcome fixtures behind a temporary
  cutover flag before deleting duplicate loops;
- preserve event order, ledger, persistence, and deterministic output.

Exit gate:

- one source path owns ReAct iteration;
- a repository search and architecture test find no private app/Runtime loop;
- planned and unplanned tests prove the same kernel behavior;
- embedded Core compatibility is either preserved or migrated with a documented
  semver story;
- CDH controls pass through one safe-boundary implementation;
- every budget dimension, evaluator/finalizer path, and trace-tail recovery path
  has deterministic evidence;
- the compatibility flag can roll back before the recorded schema floor without
  dual-executing model calls or mutations.

### M5 - Project Trust

Entry gate: M0.5 is merged. M5 replaces the temporary guard through an explicit
migration and must not regress its fail-closed negative tests.

Deliverables:

- persistent operator-owned trust store and canonical identity;
- granular capability grants and CLI/API/Web trust contracts;
- deferred project config and MCP activation;
- executable-config digest/change handling;
- active revocation/termination/quarantine semantics and canonical audit facts;
- authenticated/origin-safe trust-decision endpoints;
- restricted inspection behavior;
- hostile-repository and nested-workspace negative tests;
- migration for existing known workspaces without silently trusting them.

Exit gate:

- selecting an unknown folder cannot spawn a process or activate executable
  workspace integrations;
- every trust decision is explicit, persisted outside the repository, revocable,
  granular, identity-safe across platform aliases, and bounded by operator
  policy.

### M6 - Execution Environment foundation

M6 is a behavior-parity migration of current foreground filesystem/search/Shell
and stdio MCP authority onto sealed ports. New background Shell features belong
to M7, preventing M6 and M7 from implementing the same lifecycle twice.

Deliverables:

- implement local and in-memory environment adapters;
- inject environment through ToolInvocationContext;
- implement the bounded Runtime-owned observation store;
- move filesystem/search operations onto `WorkspaceFileSystem`;
- move the current foreground Shell behavior onto `ProcessHost`;
- route stdio MCP process creation through the Runtime process capability or
  document a narrower equivalent with the same trust/policy owner;
- persist redacted environment identity for resume diagnostics;
- add conformance suites reusable by every adapter.

Exit gate:

- built-in file/search/Shell tools do not own workspace roots or directly call
  host filesystem/process APIs;
- direct host access is confined to named adapters;
- the in-memory adapter runs deterministic tool tests;
- local path, symlink/junction, cancellation, timeout, and cleanup tests pass.

### M7 - Coding Tool V2

Entry gate: M3D's canonical Tool Schema and authoritative Runtime validation
contracts are merged.

Deliverables:

- true bounded/ranged Read with observations and continuation;
- strict exact Edit with uniqueness and stale-version checks;
- create-first Write and explicit compatible overwrite;
- observed delete/move/rename and bounded directory lifecycle operations;
- real localized Diff and an implemented bounded checkpoint/rewind path;
- bounded deterministic search/list/glob with continuation and ignore policy;
- foreground/background Shell with process identity and progressive output;
- the M1 PTY decision implemented or exposed as an explicit unsupported
  capability without terminal-emulation claims;
- typed large-output/artifact integration when available;
- result projection and obsolete-tool-output reclamation;
- prompt/tool descriptions that guide precise reads and edits;
- deterministic coding benchmark suite.

Exit gate:

- all Section 8 invariants and Section 11 deterministic scenarios pass;
- large-file tasks do not place whole files in model context;
- stale/non-unique edits cannot mutate files;
- delete/move/rename and rewind cannot overwrite newer user work or cross the
  workspace boundary;
- output truncation never falsely implies that discarded evidence is retained;
- a real local coding scenario succeeds through CLI and the product surface.

### M8 - Strategy and Agent knowledge integration

M8 has two required ordered submilestones and one optional follow-up. They do not
share a long-lived implementation branch.

**M8A - Strategy and context efficiency:**

Deliverables:

- default ordinary product tasks to ReAct under explicit compatible migration;
- preserve explicit/resumed PlanReact and its ledger;
- integrate context/tool-result reclamation before semantic compaction;
- extend evaluation for strategy selection and context efficiency.

**M8B - AgentDefinition, instructions, and procedures/Skills:**

- add AgentDefinition loader/validator and immutable AgentRuntimeProfile with a
  legacy compatibility profile;
- add workspace instruction discovery under Project Trust;
- add the typed procedure/Skill catalog, deterministic eligibility/selection,
  stable capability binding, and progressive hydration;
- persist exact profile/instruction/catalog/selection/hydration identity through
  events, artifacts, checkpoint, report, repair, and resume;
- run the OnCall-inspired selection/adherence/evidence evaluation without
  treating reference content as trusted procedure.

**M8C - Optional product Subagent/background Agent:**

This is not required for M0-M10 completion. It may begin only under a separately
accepted scope with deterministic fixtures, global budget enforcement,
restricted Execution Environment, typed parent/child result, canonical
provenance, cancellation, and unknown-effect semantics. Implementing rove's
Subagent feature does not authorize coding agents working on this repository to
spawn Subagents; Section 0's per-conversation prohibition remains absolute.

Exit gate:

- simple tasks do not make an unnecessary planning call;
- planned resume retains existing conservative guarantees;
- Agent/profile/procedure identity is immutable and reproducible on resume;
- Skills/instructions cannot grant permission and remain bounded/provenanced;
- procedure selection distinguishes no-match/degraded/error and cannot promote
  external content or self-authored current-run content to trust;
- any separately accepted M8C work uses the same kernel and a restricted
  environment rather than private loops.

### M9 - Application services, schema enforcement, and product adapters

Deliverables:

- extract only the post-CDH API/ProductStore services justified by measured
  concentration;
- enforce and complete consumer migration to the M1-selected Rust/OpenAPI/Web
  schema workflow;
- add typed UI projections for new entries/tool/process/resource states;
- adapt CLI/API/Web without creating new lifecycle truth;
- remove compatibility code only after consumers and migrations are proven.

Exit gate:

- transport handlers are not owners of kernel/runtime policy;
- contract changes have one canonical schema/conformance path;
- unknown typed content fails visibly and safely across product surfaces.

### M10 - Program stabilization and release evidence

Deliverables:

- remove expired compatibility writers/flags only after the support window and
  rollback floor permit it;
- run clean-tree Rust, Web, browser, local-full, product-acceptance, migration,
  repair/resume, provider-fixture, trust, environment, tool, and coding suites;
- run the declared Windows/Linux conformance matrix;
- run optional external-provider/real-model gates only when configured and
  report every unrun gate honestly;
- compare final results against the sealed pre-change thresholds and baseline;
- update all current runtime docs, acceptance matrix, README claims, generated
  schemas, and the machine-readable acceptance report;
- clean implementation worktrees/branches/generated state and merge through
  reviewed PRs into `main`.

Exit gate:

- one kernel, migration, trust, budget, environment, Coding Tool V2, and M8A/B
  exit gates all pass on `main`;
- the repository and GitHub branch/PR state are clean and current;
- contract-complete versus provider-interoperable versus coding-capable claims
  match the gates that actually ran;
- Desktop remains explicitly unimplemented and governed by a future D0 plan.

---

## 15. Compatibility and migration rules

### 15.1 Serialized state

- Every TaskState/checkpoint change increments or explicitly handles schema
  compatibility.
- Old fixtures remain in tests until the supported migration window closes.
- Repair and cleanup understand both old and new state during migration.
- Canonical traces are not rewritten to simulate a new schema.
- Reports remain derived and can be regenerated from facts where supported.

### 15.2 Events and APIs

- Prefer additive events/fields with defaults before removal.
- Event producers, trace persistence, SQLite indexing, API SSE/OpenAPI, Web
  parsing, Desktop adapters, reports, and tests change together.
- No compatibility dual-fire may create ambiguous duplicate lifecycle facts.
- Unknown new variants must produce a safe explicit partial/unsupported state.

### 15.3 Tools

- Preserve public tool names where possible: `read_file`, `write_file`,
  `search_code`, and `run_shell`.
- Add `edit_file` as the normal source modification contract.
- Additive Read arguments use safe defaults and server-owned hard caps.
- Existing whole-file update behavior receives a bounded compatibility period
  and explicit deprecation; it is not silently reinterpreted.
- Tool call IDs, native provider tool-use IDs, mutation metadata, approval, and
  report correlation remain stable.

### 15.4 Core embedding

The existing embedded `rove-core::Agent` contract needs a deliberate migration:

- either evolve it into the new kernel facade while preserving common callers;
- or retain a compatibility wrapper over the same one kernel.

A compatibility wrapper may adapt configuration and events. It may not contain
another loop.

### 15.5 Rollback

Each implementation milestone must identify:

- schema rollback floor;
- feature/config flag where behavior can safely be selected;
- whether newly emitted state remains readable by the previous build;
- how active/background processes are handled;
- how trust records and environment identity are preserved;
- what cannot be rolled back after an irreversible migration.

Rollback cannot discard canonical evidence or replay work.

---

## 16. Verification strategy

Run focused tests first, then expand in proportion to the milestone.

### 16.1 Rust default gate

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Required focused suites will include existing and new tests for:

- embedding/kernel contracts;
- end-to-end Runtime execution;
- API/SSE/state compatibility;
- tool safety and approval negatives;
- MCP trust/process behavior;
- persistence/repair/resume;
- benchmarks and coding evaluations;
- Execution Environment adapter conformance.

### 16.2 Web gate

From `apps/web/`:

```powershell
pnpm test
pnpm typecheck
pnpm build
pnpm test:e2e
```

Browser-visible lifecycle, process, trust, resource, approval, input,
cancellation, resume, or API proxy changes require real-API coverage in
addition to deterministic mocked fault injection.

### 16.3 Aggregate acceptance and clean evidence

At M0, every public-contract milestone, and M10, run the repository-owned
aggregate path appropriate to the change:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/product-acceptance.ps1
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

The POSIX `scripts/product-acceptance.sh` must retain schema/check parity. The
machine report is generated from real exit codes on a clean commit; it is never
hand-edited. Gated provider/MCP/real-model checks include an exact reason when
not run. Browser-visible changes require the live-API path, not only mocked E2E.

Execution Environment, file identity, path, line-ending, atomic replace, and
process-tree claims run on Windows and Linux. A platform-specific skip is an
unsupported result, not a cross-platform pass.

### 16.4 Structural gates

Add enforceable checks where practical:

- built-in tools cannot import/call direct host fs/process APIs outside adapter
  modules;
- app surfaces cannot instantiate a private Agent loop;
- production Web modules and any future Desktop host cannot create private
  canonical events;
- workspace content cannot register executable UI/runtime extensions;
- provider payload types do not escape `rove-models`;
- public wire variants stay synchronized with the chosen schema workflow;
- repository implementation records identify at most one authorized top-level
  conversation per worktree, prove that each stayed within its ownership, and
  show no Subagent creation or delegation.

### 16.5 Security gate

For every milestone involving tools, trust, providers, state, MCP, artifacts,
or API, verify:

- input size, path, timeout, output, queue, and concurrency bounds;
- trust and approval at the correct boundary;
- no permission through instructions, Skills, hooks, MCP annotations, or model
  text;
- secret redaction in all durable and visible surfaces;
- retry/idempotency safety for mutations and controls;
- cancellation and resume treatment of unknown effects;
- remote URL/MIME/filename/resource validation when applicable;
- typed, visible failure rather than optimistic success.

---

## 17. Completion criteria

This program is complete only when all applicable milestones have merged into
`main` and the following are true:

1. one reusable Core kernel owns ReAct model/tool iteration;
2. Runtime planned and unplanned strategies delegate to that kernel;
3. typed session/application entries are distinct from model protocol messages;
4. provider adapters consume one bounded neutral ToolCall/ToolResult/stop/schema
   contract, preserve error semantics, and project history safely across target
   providers;
5. context, Session, compaction, model, tool, resource, provider, and UI
   projection behavior have stable internal composition boundaries;
6. Project Trust prevents repository activation before explicit granular
   consent and supports identity-safe revocation of active grants;
7. all built-in local file/process tools execute through Rove's Execution
   Environment;
8. Read is truly bounded and Edit is exact, unique, stale-safe, atomic where
   supported, and Diff-aware; observed delete/move/rename, search/list,
   checkpoint, and rewind preserve newer user work and workspace bounds;
9. Shell supports process identity, progressive output, background lifecycle,
   and conservative cancellation/resume;
10. ordinary coding defaults to lightweight ReAct while planned work preserves
    rove's ledger/recovery strengths;
11. rule-first/model-on-ambiguity evaluation, independent Finalizer/fallback,
    global multidimensional budgets, and trace-tail reconciliation are complete;
12. versioned AgentDefinition and immutable AgentRuntimeProfile identity are
    validated, persisted, visible, and exactly resumable through the legacy
    compatibility path;
13. workspace instructions and procedures/Skills are bounded, progressively
    disclosed, provenanced, trust-gated, and unable to grant permission;
14. deterministic coding evaluation proves contracts and meets sealed safety/
    regression thresholds; stronger provider/coding-quality claims require the
    corresponding real-model gates;
15. CLI/API/Web consume shared Runtime contracts without private loops or event
    truth;
16. all serialized/API migrations and old-state resume tests pass;
17. current `docs/runtime/` agrees with the merged implementation;
18. optional external-provider, remote, container, M8C Subagent, future Desktop,
    or unsupported-platform claims are made only from gates that actually ran;
19. every implementation conversation stayed within one assigned worktree and
    ownership manifest without calling, creating, resuming, or delegating to
    any Subagent;
20. M10 leaves local `main` clean, synchronized with `origin/main`, with no open
    program PR, stale implementation worktree, or generated evidence staged.

Prose, type scaffolding, or a passing pipeline smoke alone is not completion.

---

## 18. Non-goals for the first implementation waves

- Rewriting rove into Pi's package layout.
- Removing canonical events, durable state, approval, or conservative resume.
- Implementing a dynamic Rust plugin ABI before internal contracts stabilize.
- Running arbitrary workspace-provided UI code.
- Claiming container or remote execution from an interface alone.
- Adding built-in vector RAG.
- Loading every Skill or workspace document into every prompt.
- Using fuzzy source rewriting as the default Edit behavior.
- Silently changing existing `provider_type=openai` profiles from Chat
  Completions to Responses.
- Persisting provider wire payloads as canonical Session history.
- Making PlanReact mandatory for every task.
- Treating a trusted workspace as authorization for every future command.
- Automatically restarting background commands or unknown side effects.
- Reimplementing CDH features that already merged and passed M0.
- Implementing Desktop in this program; it requires a future D0 design/plan.
- Treating top-level worktree concurrency as permission to cross ownership,
  bypass dependency gates, edit `main` directly, or call any Subagent.

---

## 19. Coordinator handoff record

### CDH merge and baseline

- PR #29, `feat(product): complete CDH control, evidence, and settings surface
  (G1-G7)`, merged as `f9e88a7553bcc7561550e5b8286c320108c8fd51`.
- The `f9e88a7` push CI completed successfully for Rust fmt/clippy/workspace tests
  and Web unit/typecheck/build jobs.
- G8 Desktop was out of scope; `apps/desktop` does not exist.
- The pre-seal root contained a user-owned README rewrite and this untracked
  plan. M0 reconciled and committed both rather than discarding them.
- Retired CDH/product worktrees, their ignored generated state, absorbed local
  branches, and patch-equivalent remote branches were removed after evidence
  was captured. No old implementation worktree is an authorized future base.

### Merged capability inventory

Section 3.0 is the summary. The detailed implemented evidence remains in
`docs/runtime/acceptance-matrix.md`, source/tests, generated OpenAPI, and the
CDH completion log. Product artifacts and evidence export are implemented;
MCP Tool Artifacts and Desktop are not.

### Verification

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `pnpm test`, `pnpm typecheck`, `pnpm build`, and `pnpm test:e2e` from
  `apps/web/`
- `scripts/product-acceptance.ps1` generated the tracked clean-source
  `PRODUCT_ACCEPTANCE_REPORT.json`; its real exit codes and source block are the
  authoritative aggregate result.
- `scripts/integration-smoke.ps1` is the live local API/Web acceptance result.
- Real external-provider and optional official MCP filesystem-server gates
  remain opt-in; an unrun gate is not interoperability evidence.

### Plan reconciliation

- Updated the baseline and CDH/current documentation to merged reality.
- Corrected the execution boundary to prohibit Subagents inside every
  conversation while allowing up to two user-opened top-level conversations,
  one per owned worktree, after dependency and ownership gates.
- Added M0.5 Trust guard, lifecycle Finalizer/evaluator/budget/trace-tail work,
  AgentDefinition/profile/procedure scope, observation/file lifecycle/search,
  claim levels, cross-platform conformance, and clean evidence requirements.
- Split overlapping M2/M3, M6/M7, and M8 responsibilities; selected the schema
  workflow in M1; moved Desktop to a future D0 program; made M10 the program
  integration/evidence seal.

### First implementation milestone

- Milestone: M0.5 immediate Project Trust guard.
- Worktree B: `.worktrees/agent-trust-tools`, branch
  `feature/agent-trust-guard`, created on 2026-08-06 from exact `origin/main`
  SHA `b972df42f0f90d5ce7776c1962cbefa6b4941ade`.
- Worktree A: `.worktrees/agent-kernel`, branch `feature/agent-kernel-m1`,
  created from the same exact SHA. It remains idle until M0.5 is merged and
  verified, then refreshes from the new `main` before M1 begins.
- Initial ownership: focused `apps/bootstrap` config/assembly, MCP activation
  boundary, new narrowly owned trust-guard module/types, focused CLI/API/Web
  restricted-state adapters, and dedicated negative tests.
- Forbidden until M1 assigns ownership: broad TaskState/session schema changes,
  new canonical lifecycle families, provider protocol migration, kernel loop
  changes, ProductStore refactors, generated-schema workflow changes, or
  Desktop code.
- Required focused evidence: bootstrap config tests, MCP disabled/no-spawn
  negatives, tool safety, API restricted-state/auth/origin tests, and explicit
  compatibility tests; then full Rust/Web and aggregate gates proportional to
  public behavior.
- Rollback floor: the `f9e88a7` implementation baseline plus the accepted
  documentation/plan commit. Rollback cannot restore implicit workspace process
  activation or discard canonical evidence.

---

## Changelog

- 2026-08-05: Created the proposed post-CDH master plan. Recorded the product
  position, one-Core-kernel decision, typed message/session projection,
  internal extension plane, Project Trust, Rove Execution Environment, bounded
  Read and exact Edit, background Shell, ReAct/PlanReact policy, future
  Skills/workspace instructions/Subagents, coding evaluation, Desktop gate,
  compatibility rules, and M0-M10 dependency order. No implementation or active
  worktree behavior is claimed.
- 2026-08-05: Added the Provider Tool Protocol audit and target design. Kept
  the existing `ProviderClient`/`WireProtocol` direction, documented neutral
  message/result/stop/schema gaps, cross-provider history and ID projection,
  common stream assembly, capability negotiation, security bounds, a provider
  test matrix, PVA-PVE incremental slices, and a dedicated M3 milestone.
- 2026-08-06: Sealed M0 against merged PR #29. Added strict serial main-thread
  execution with no Subagents, a two-worktree isolation schedule, M0.5 Trust
  guard, active-design adoption, lifecycle/profile/budget/trace-tail scope,
  observation and missing file/search operations, claim-level evaluation,
  cross-platform and clean-evidence gates, corrected milestone ownership, and a
  Desktop-independent M10 program seal.
- 2026-08-06: Corrected the execution rule after operator clarification. Kept
  the absolute per-conversation Subagent prohibition, allowed up to two
  user-opened top-level conversations bound one-to-one to worktrees A and B,
  retained serialized coordinator merges, and limited concurrent work to
  dependency-ready, manifest-disjoint checkpoints.
