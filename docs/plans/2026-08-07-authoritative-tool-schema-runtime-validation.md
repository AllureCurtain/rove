# Authoritative Tool Schema and Runtime Validation

> Status: **Implemented and Verified (2026-08-07)**
>
> Branch: `feature/kernel-tool-schema-wave2`
>
> Base: `559bc1e` (wave-one Kernel/Message/Provider and Project
> Trust/Execution Environment merged)
>
> Dependency boundary: this work must merge before Coding Tool V2 starts.
> Coding Tool V2 is explicitly out of scope for this branch.

## 1. Outcome

This wave makes the actual `ToolRegistry` the single authoritative source for
the tool catalog used by model requests, runtime policy, planning, execution,
resume identity, and MCP registration.

After this work:

- malformed or unsupported tool schemas fail before registration or model I/O;
- one validated descriptor is pinned at registration and reused for model
  projection and argument validation;
- duplicate names and ambiguous capability bindings fail without overwriting
  existing tools;
- batch registration is atomic and registry iteration is deterministic;
- provider capability and tool-schema checks run before stream dispatch;
- Runtime derives an immutable, redacted capability snapshot from the real
  registry and pins it to runtime identity and plan revisions;
- MCP catalogs are validated and committed atomically, with conservative local
  safety metadata and namespaced capability IDs.

## 2. Current Problems

The current registry stores tools in a `HashMap`, calls `Tool::schema()` each
time a consumer asks for metadata, and silently replaces an existing tool with
the same name. It does not validate a schema when registering a tool. This can
make provider-visible schemas differ from execution-time validation and allows
an MCP alias collision to replace a local or previously discovered tool.

`rove-models` currently treats `ModelToolSchema` as unvalidated data.
`ProviderCapabilities::validate_tools` checks only streaming/tool-call support,
and Core performs that check after consuming a provider stream. A custom model
client can therefore observe invalid schemas or unsupported tool configuration
before the request fails.

Runtime stores a tool signature and already has
`PlanRevision.capability_snapshot_id`, but does not build a capability snapshot
from the registry or populate that revision field. Planner sees only the goal
and history, so its plan is not grounded in the tools available to the run.

## 3. Contracts Implemented in This Wave

### 3.1 Bounded model tool schema

`rove-models` owns provider-neutral validation for `ModelToolSchema`. The
supported executable JSON Schema subset is:

- schema nodes are JSON objects;
- `type`: `object`, `array`, `string`, `number`, `integer`, `boolean`, or
  `null`;
- `description`, `properties`, `required`, `additionalProperties`, `items`,
  and `enum`;
- `minLength`, `maxLength`, `minItems`, `maxItems`, `minimum`, and `maximum`;
- `default` as a bounded annotation only; it does not alter execution input.

Unsupported keywords and malformed keyword values fail closed. Object roots
are required because all tool invocations use a JSON object. Required fields
must name declared properties, bounds must be ordered, enum values must match
the declared type, and duplicate tool names are invalid.

Validation applies explicit limits to tool count, encoded schema bytes, schema
depth, schema nodes, properties, required fields, enum values, tool-name bytes,
description bytes, and property-name bytes. The exact public constants live in
`rove-models`; tests assert every boundary and prove failure occurs before
model dispatch.

### 3.2 Compiled registry entries

Core registration evaluates `Tool::schema()` exactly once, validates its model
projection, validates operational metadata, and stores the tool together with
the pinned descriptor and model schema. Descriptor reads, model projection,
and invocation validation all consume this pinned entry.

The registry provides typed fallible single and batch registration. Batch
registration validates the complete candidate catalog, including collisions
with existing entries, before mutating the registry. Its externally visible
ordering is lexical by registered tool name.

The existing infallible `register` method remains only as a trusted-code
compatibility wrapper and panics with a deterministic message when a built-in
descriptor is invalid. Untrusted and dynamically discovered tools, including
MCP, must use the fallible API.

### 3.3 Capability identity

Operational descriptors gain an optional stable `capability_id`, separate from
the existing availability/degradation status. A capability ID may bind to only
one registered tool in a snapshot. IDs are bounded, non-empty when present,
and use a portable namespaced syntax.

First-party tools receive stable IDs. MCP tools receive IDs namespaced by their
configured server identity and exact remote tool identity. Provider-facing
aliases remain execution names; capability IDs are not authorization and do
not bypass policy, approval, workspace, or environment checks.

### 3.4 Pre-dispatch model validation

Core validates the full tool catalog and selected provider capabilities before
calling `ModelClient::stream`. The same preflight applies to planner calls even
though the planner sends an empty tool list. Provider output validation remains
in place as a separate response boundary.

### 3.5 Runtime capability snapshot

Runtime builds a `CapabilitySnapshot` from the registry's pinned descriptors.
It includes a stable snapshot ID/tool signature and a bounded safe summary of
name, capability ID, description, availability, mutation class, scheduling
constraint, and input schema. It excludes secrets, credentials, local command
lines, and MCP transport details.

The snapshot is immutable for an Engine instance/run. `RuntimeIdentity` gains
an optional backward-compatible `capability_snapshot_id`; older saved identity
without it remains readable, while a persisted ID must match on resume.
Initial and replacement `PlanRevision` values pin the snapshot ID. Planner gets
the same bounded snapshot summary through a separate context value rather than
a hand-maintained prompt list.

This wave does not implement live capability refresh. A changed registry
creates a different Engine/runtime identity; it is not silently substituted
inside an active run.

### 3.6 Atomic MCP catalog registration

Each enabled server may be connected and queried using the existing stdio or
legacy SSE transport. Discovered tools are accumulated, normalized, validated,
and then committed to the registry as one batch. Any invalid schema, duplicate
alias, duplicate capability ID, or collision with an existing tool leaves the
registry unchanged.

Remote annotations remain untrusted. MCP tools stay conservative
(`destructive = true`, `parallel_safe = false`) in this wave.

## 4. Compatibility

- Existing `Tool` implementations keep the same trait.
- Existing trusted `ToolRegistry::register` call sites remain source
  compatible.
- `RuntimeIdentity.capability_snapshot_id` is additive with a serde default and
  is omitted when absent in legacy fixtures.
- Existing `PlanRevision` fixtures already tolerate an absent snapshot ID.
- No provider-specific payload receives operational safety metadata.
- No dependency crate or npm package is required.

## 5. Implementation Order

1. Add model-schema validation types, limits, and tests in `rove-models`.
2. Add pinned registry entries, deterministic iteration, typed registration
   errors, atomic batch registration, and Core tests.
3. Move provider/tool preflight before model stream creation and prove invalid
   input performs zero dispatches.
4. Add capability IDs to built-ins and normalized MCP descriptors; convert MCP
   discovery to atomic registration.
5. Add Runtime capability snapshot construction, runtime identity binding,
   planner context, and plan-revision binding.
6. Update current runtime documentation only for behavior that is implemented.
7. Run focused tests, formatting, workspace Clippy, and the full Rust workspace
   suite. Web checks are not required unless a public API/Web contract changes.

## 6. Acceptance Criteria

- invalid root, unsupported keywords, malformed bounds, excessive size/depth,
  and excessive catalog size are rejected before registration/model I/O;
- execution never re-reads a mutable/dynamic schema from a tool object;
- descriptors and model schemas are deterministic across insertion order;
- duplicate tool names and capability IDs cannot overwrite a registered tool;
- a failed batch, including an MCP catalog, changes registry length by zero;
- provider incompatibility and invalid schemas dispatch zero model streams;
- built-in and MCP capability IDs are stable and do not grant permission;
- runtime identity contains the registry-derived snapshot ID;
- every new plan revision contains the same run-pinned snapshot ID;
- planner requests contain a bounded, redacted summary of that snapshot;
- legacy runtime identity and plan artifacts remain readable;
- `cargo fmt --all --check`, workspace Clippy with warnings denied, and
  `cargo test --workspace` pass.

## 7. Explicitly Deferred

- Coding Tool V2 and its read/edit/apply-patch/search/shell UX;
- the shared Agent/Runtime kernel cutover;
- model-based PlanEvaluator, Replanner separation, and independent Finalizer;
- AgentDefinition, `AGENTS.md` runtime discovery, procedures, and Skills;
- dynamic capability refresh and persisted full snapshot artifacts;
- MCP Streamable HTTP, negotiated sessions, rich result envelopes, output
  schemas, and Tool Artifact persistence;
- Desktop/Tauri work.

Coding Tool V2 should start from `main` only after this branch is reviewed,
merged, and its final verification is green.

## 8. Completion Evidence

Implemented on `feature/kernel-tool-schema-wave2` from base `559bc1e`.
The final deterministic gates passed:

- `cargo fmt --all --check`;
- `cargo test -p rove-models` (140 passed);
- `cargo test -p rove-core` (14 passed);
- `cargo test -p rove-runtime --lib` (134 passed);
- `cargo test -p rove-runtime --test mcp_contract` (1 passed);
- `cargo test -p rove-integration-tests --test e2e` (100 passed);
- `cargo test -p rove-integration-tests --test mcp` (8 passed);
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace`.

No external-provider, real-MCP, or browser gate was run because this wave does
not claim external interoperability or change a Web contract. Coding Tool V2,
the shared kernel cutover, Finalizer/evaluator work, live capability refresh,
and MCP Streamable HTTP remain deferred exactly as listed above.
