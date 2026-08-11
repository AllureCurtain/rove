# P0 - Prompt And Agent Intelligence

> Status: **Current-state correction and audit evidence / Not an implementation plan.**
> The 2026-08-10 program selectively incorporates findings into Workstream B:
> [`2026-08-10-post-full-delivery-productization.md`](2026-08-10-post-full-delivery-productization.md).
> The observations below describe the 2026-08-09 `main` snapshot. Sections named
> `Proposed Design` or `Acceptance Criteria` are retained analysis and do not
> create future work. Only requirements restated in the 2026-08-10 program are
> authorized for implementation.

## 1. Current State

Measured on `main` at 2026-08-09.

| Asset | Size | Content |
|---|---|---|
| `prompts/system.md` | **13 lines** | Identity line, JSON tool-call instruction, two examples, one tool-preference sentence, final-answer rule, "be concise" |
| `prompts/planner.md` | **8 lines** | JSON shape for `{goal, steps[]}` only |

Loading in the 2026-08-09 snapshot was in `apps/bootstrap/src/config.rs:539`:

```rust
pub fn load_system_prompt(&self) -> String {
    let path = self.resolve_path(&self.runtime.system_prompt_path);
    std::fs::read_to_string(path).unwrap_or_else(|_| {
        "You are rove, a helpful assistant that can use tools to accomplish tasks.".to_string()
    })
}
```

The prompt is a single opaque string. `ContextManager` inserts it as one
`Message::system` at position 0 (`runtime/src/context/manager.rs:183`). There is
no section model, no environment injection, and no assembly step.

Both prompt paths were governed by Project Trust
(`apps/bootstrap/src/project_trust.rs:772-773`), so a repository cannot silently
change agent behavior. That property must be preserved by any change here.

### 1.1 The problem is category, not length

The instinct on reading `prompts/system.md` is that it is too short. Length is the
symptom. The actual defect is that it is not a system prompt at all - it is an
**output-format specification** wearing a system prompt's filename.

Count what the 13 lines allocate:

| Purpose | Lines |
|---|---|
| How to serialize a tool call | 6 (instruction + 2 examples + final-answer rule) |
| Which tool to prefer | 1 |
| Identity | 1 |
| Style | 1 |

Nine of thirteen lines describe wire formatting. Under a native tool-use provider
the provider already owns that concern, so those nine lines are not just wasted -
section 4.3.1 shows they actively steer the model onto the unsafe path.

A system prompt's job is to establish **who the agent is, where it is, what it can
do, how it should decide, and how it should verify.** Measured against those five,
the current file answers one and a half: a bare identity line, and a single
sentence of tool preference. There is no *where* at all - the model is not told its
working directory, platform, workspace kind, git state, or which tools are
actually registered.

This reframing matters for scoping. If the problem were length, the fix would be
"write more prose." Because the problem is category, the fix is a structured
assembly step with named sections whose content is derived from real runtime state
(section 4.1), and the JSON formatting text mostly gets deleted rather than
expanded.

## 1.2 Post-full-delivery rebaseline

The full-delivery baseline is assumed to provide versioned AgentDefinition
packages, immutable runtime profiles, bounded `AGENTS.md` discovery, typed
procedures, profile provenance, and runtime identity pinning. Therefore the open
question is no longer "does rove have a prompt assembly step?" It is:

> Does the final assembled profile give the model the smallest trustworthy set of
> environment, tool, procedure, budget, and verification facts needed to finish
> real repository work?

Before implementing any new section abstraction, inspect the final profile and
prompt-slot assembly. Extend that authority in place. Keep system prompt,
planner, evaluator, finalizer, workspace instructions, memory, procedures, and
tool permissions separately typed and separately hashed where the runtime
contract requires it.

The remaining work is split into four independently measurable concerns:

| Concern | Required result |
|---|---|
| Native tool use | Native-capable providers never need JSON-text tool instructions |
| Recovery | Invalid calls produce bounded, deterministic, retry-positive tool results |
| Planning | Steps describe verifiable outcomes and fit the actual budgets |
| Grounding | Environment and capability facts are bounded data, never permissions |

## 2. What Is Actually Missing

The current prompt teaches exactly one thing: emit JSON to call a tool. Under a
native tool-use provider that instruction is not merely redundant, it is
**actively harmful** - it pushes capable models onto the compatibility text path.
The evidence run in the program document shows a model doing precisely that.

Missing capability, in priority order:

1. **Environment grounding.** The model is not told its working directory,
   platform, workspace kind, git state, or which tools are actually registered.
   It cannot reason about where it is.
2. **Tool selection policy.** One sentence prefers `search_code` over
   `run_shell`. Nothing explains ranged `read_file`, `glob_paths` versus
   `list_directory`, when `workspace_checkpoint` is worth taking, or that
   `edit_file` requires a prior exact observation.
3. **Engineering conventions.** Nothing says read surrounding code before
   writing, match existing style, or prefer the smallest correct change.
4. **Verification loop.** Nothing says run the project's tests after a mutation,
   or how to find the right command.
5. **Failure recovery guidance.** Nothing tells the model that a validation
   error is recoverable within the same step and should be corrected rather than
   retried identically.
6. **Planner decomposition guidance.** Eight lines specify a JSON shape and
   nothing about what makes a good step. The evidence run produced three steps
   for a two-file read, then exhausted a four-turn budget on step one.
7. **Budget awareness.** The model does not know `max_model_turns_per_step`
   exists, so it cannot economize.

## 3. Reference Designs

### Claude Code (from `learn-claude-code/s10_system_prompt`)

The most directly applicable model. Key properties:

- **Sectioned, assembled at runtime, not one hardcoded blob.**
  `getSystemPrompt(tools, model, additionalWorkingDirs?, mcpClients?)` returns
  `string[]` where each element is a section.
- **Static versus dynamic split**, separated by a
  `SYSTEM_PROMPT_DYNAMIC_BOUNDARY`. Static sections: identity, system,
  doing_tasks, actions, using_tools, tone_style, output_efficiency. Dynamic
  sections: session_guidance, memory, env_info_simple, language, output_style,
  mcp_instructions, token_budget, and others.
- **The boundary exists for prompt caching.** Static sections merge into one
  cacheable block; dynamic sections are not globally cached.
- **One volatile section.** `mcp_instructions` is created through an explicitly
  named uncached path because MCP servers can connect and disconnect between
  turns.
- **Two context channels with different injection points.** System context
  (git status) is appended to the system prompt array. User context (project
  instructions, current date) is prepended as a `<system-reminder>` user message.
- **Scale:** roughly 20-30KB in standard interactive mode. A simple mode exists
  at about 150 characters, which shows the section model supports radical
  reduction without a second code path.

### PI (`pi/packages/coding-agent`)

Tool descriptions carry the operational contract, not just a name. The `find`
tool description states the truncation limits inline:

> "Respects .gitignore. Output is truncated to {DEFAULT_LIMIT} results or
> {DEFAULT_MAX_BYTES / 1024}KB (whichever is hit first)."

`read` exposes `offset` and `limit` so paging is a first-class model-visible
affordance rather than a hidden runtime behavior.

Lesson for rove: some guidance belongs in the tool schema description, where it
is adjacent to the decision, not in a distant prompt section.

### Convergent lesson

Both projects treat the prompt as **assembled data derived from real runtime
state**, never a static file. The audited 2026-08-09 rove baseline had only the
static file; the completed AgentDefinition/profile assembly must be measured
before deciding what remains.

## 4. Proposed Design

### 4.1 Profile-aware prompt assembly

Extend the completed AgentDefinition/AgentRuntimeProfile assembly in place. Do
not introduce an independent `PromptSection` authority that can disagree with
profile slots, workspace instructions, procedures, or resume identity. A section
representation is acceptable only as an internal projection of the already
selected profile and must remain bounded and typed.

```text
PromptSection {
    id: &'static str,
    kind: Static | Dynamic,
    content: String,
}
```

The final assembly produces an ordered, bounded projection for the existing
ContextManager. Static profile content, dynamic runtime facts, trusted
workspace-instruction projections, and selected procedures must retain their
existing authority and hash semantics. Dynamic content must not churn the stable
prefix when its inputs did not change.

Proposed initial sections:

| Section | Kind | Source |
|---|---|---|
| Profile guidance | Profile slot | Selected AgentDefinition/profile |
| Workspace instructions | Dynamic, scoped | Runtime discovery and authority policy |
| Environment | Dynamic | `Workspace` + `RuntimeIdentity` |
| Capabilities | Dynamic | `CapabilitySnapshot` (bounded summary) |
| Budget | Dynamic | `ExecutionPolicy` and public budget snapshot |
| Procedures | Dynamic, selected | Typed catalog and context priority |
| MCP facts | Dynamic, volatile | Validated catalog, when trust permits |

### 4.2 Environment injection

`RuntimeIdentity` already carries everything needed and is already redacted:
`cwd`, `workspace_kind`, `model_id`, `approval_policy`, `max_steps`,
`plan_enabled`, `execution_capabilities`. The dynamic `environment` and
`capabilities` sections render from those existing values.

Constraint: the environment section must state capability facts as **data**, in
the same spirit as the existing Planner capability summary. It must not read as
permission. `process_pty: false` must render as unavailable, not as something to
work around.

### 4.3 Native tool-use first

The JSON tool-call instruction moves out of the always-on prompt. It is emitted
only when the resolved `ModelClient` does not advertise native tool-call support.
`ProviderCapabilities` already exists on the `WireProtocol` trait, so this is a
lookup, not a new mechanism.

Two independent statements are being conflated today and must be separated:

- **Wire contract**: a tool call travels in the provider's structured channel
  (`tool_calls` for openai-completions, `tool_use` blocks for anthropic-messages).
  This is what the prompt should assert.
- **Compatibility parse**: rove can also recover a call from plain text. This is
  an implementation fallback and must never be described to the model as the
  normal path.

The prompt therefore states the contract positively - request tools through the
structured tool-call channel, and reserve plain text for the final answer - and
says nothing about JSON shapes when the provider is native-capable.

### 4.3.1 The compatibility parse is bidirectionally unsafe

`core/src/model_turn.rs:266` dispatches on native call count, and count zero
falls through to `parse_action`:

```rust
fn build_action_from_model_output(calls: Vec<ToolCallAction>, full_response: &str) -> Action {
    match calls.len() {
        0 => parse_action(full_response),
        ...
```

`core/src/parser.rs:4` then produces a wrong answer in both directions:

- **False positive.** Any assistant text that happens to parse as JSON with
  `tool` and `args` keys becomes a real tool invocation. A model explaining a
  tool call is indistinguishable from making one.
- **False negative.** Parse failure returns `Action::Final { text: raw }`, so a
  malformed tool call is silently promoted to "task complete". A truncated or
  slightly-off JSON emission ends the turn with the broken JSON as the answer.

The second is the more damaging of the two, because it converts a recoverable
formatting error into a terminal success. Native-capable providers should not
reach this code at all; the ordering guard at `core/src/model_turn.rs:167-184`
already ensures native wins, and is covered by
`native_tool_use_wins_over_text_fallback`.

Note that the fallback's original justification no longer holds:
`models/src/fake.rs:9-22` defines `FakeTurn::{Text, ToolUse, ToolBatch}`, so the
in-process fake provider emits native calls and does not need the text path
either.

### 4.3.2 Prior art on malformed tool calls

No sibling agent treats text parsing as a tool-call path.

| Project | Mechanism | Malformed-call handling |
|---|---|---|
| PI | `toolcall_delta` events carrying `contentIndex` + partial assistant message (`packages/ai/src/types.ts:525`) | No text-parsing path exists |
| Hermes | `json.loads(tool_call.function.arguments)` on the structured field (`tool_dispatch_helpers.py:176`, `tool_executor.py:144`) | No text-parsing path exists |
| OpenCode | Structured calls, plus `experimental_repairToolCall` (`packages/opencode/src/session/llm.ts:296`) | Repairs, then routes to a dedicated tool |

OpenCode's approach is the one worth borrowing. It attempts a cheap repair
first - lowercasing a tool name that only differs by case - and if that fails it
rewrites the call to a real registered tool named `invalid`
(`packages/opencode/src/tool/invalid.ts`), whose entire body is:

```ts
execute: (params: { tool: string; error: string }) =>
  Effect.succeed({
    title: "Invalid Tool",
    output: `The arguments provided to the tool are invalid: ${params.error}`,
    metadata: {},
  }),
```

It is registered at `registry.ts:205` but filtered out of `activeTools`, so the
model can never select it - it can only be routed into it. The payoff is that a
malformed call comes back as an ordinary tool result the model can read and
retry, instead of terminating the turn. That is the structural inverse of our
`Action::Final` default: OpenCode's worst case is a wasted turn, ours is a wrong
answer marked complete.

Adopting this is a runtime change, not a prompt change, and is tracked here only
as the reason the prompt must not advertise the text path.

### 4.4 Planner prompt

Extend `prompts/planner.md` beyond a JSON shape:

- Prefer the fewest steps that make progress verifiable.
- Do not create a step whose only content is reading one file.
- A step must be completable within the step-local model-turn budget.
- Name the observable outcome, not the activity.

### 4.5 Tool error messages must carry fix guidance

Prompt guidance about recovering from errors is useless if the error itself does
not say what to do. The evidence run shows the failure end to end:

| Trace position | Content |
|---|---|
| `checkpoint.session.entries[6]` | call `search_code` with `{"query": "[workspace.dependencies]", "regex": "true"}` |
| `history[7]` (tool) | `Error: Invalid arguments: Argument regex must be boolean` |
| `history[8]` (assistant) | empty - the model abandoned `search_code` entirely |
| `history[9]` (tool) | re-read of `Cargo.toml`, byte-identical to `history[3]` |

The model had one wrong character. It responded by discarding the tool and
repeating work it had already done, which is what exhausted the four-turn step
budget. The message named the constraint but not the correction, and gave no
signal that a retry was worthwhile.

Required message shape - state the field, the expected type, what was received,
and the corrected call:

```
field 'regex' must be a JSON boolean (true / false), got string "true".
Retry with {"regex": true}.
```

Three properties are load-bearing:

- **Deterministic.** Generated from the schema and the received value, never
  model-authored, so it stays reproducible across replays.
- **Retry-positive.** The message must make it explicit that the same tool with
  corrected arguments is the expected next action.
- **Non-authoritative.** A fix hint is guidance, never a permission grant. This
  is a validation message, so it sits before the approval gate and cannot widen
  it. The fail-closed rule is unchanged: tool descriptions, MCP annotations,
  prompts, and model requests cannot grant permission.

This is a schema-validation change in the Executor rather than a prompt change,
but it is scoped into this document because the prompt's failure-recovery
guidance cannot be validated without it.

### 4.6 Trust and hashing

Unchanged and non-negotiable:

- Section source files stay under the existing trust-governed prompt paths.
- `system_prompt_hash` and `planner_prompt_hash` must cover the assembled result,
  so a prompt change still invalidates a resume identity match.
- Adding a section must not make `stable_prefix_hash` unstable across turns when
  nothing actually changed. Dynamic sections that vary per turn belong outside
  the stable prefix.

## 5. Risks

| Risk | Mitigation |
|---|---|
| Prompt growth silently eats the token budget | Emit assembled section byte counts into `PromptBuildMetadata`; assert a ceiling in tests |
| A dynamic section leaks a raw path or secret | Render only from already-redacted `RuntimeIdentity` fields; reuse existing redaction tests |
| Environment text is read as permission grant | Keep capability rendering declarative; retain approval gating tests unchanged |
| `stable_prefix_hash` churn breaks prompt caching | Static/dynamic split is load-bearing, not cosmetic; test that an unchanged workspace yields an unchanged stable prefix across turns |
| Over-modeling into a 50-hook prompt plugin surface | Ship a fixed section list first. No extension point in this pass |

## 6. Acceptance Criteria

1. The evidence task (read two files, summarize in three bullets) completes
   inside the default step budget on the fake provider and has a separately
   classified result on at least one native tool-use provider.
2. A native-tool-use provider run contains **zero** JSON-text tool actions, while
   a compatibility-only client retains the minimum required fallback behavior.
3. A schema-validation failure names the field, expected type, received type,
   and a bounded corrected call; the recorded `regex`-as-string regression is
   covered explicitly.
4. A malformed compatibility call cannot be promoted to a successful final
   answer and instead produces a recoverable typed result.
5. Prompt/profile metadata reports bounded component sizes and total bytes; the
   assembled result remains within the configured ceiling.
6. An unchanged workspace, profile, capability snapshot, and tool catalog
   produce stable identity/prefix hashes across consecutive turns.
7. A restricted workspace produces no unauthorized project-sourced instruction
   or procedure section, verified by a negative test.
8. `agent-smoke` results are recorded before and after in the same format under
   `.rove/bench`, and real-provider results are never represented by fake-gate
   evidence.
9. `docs/runtime/subsystems.md` and `docs/runtime/acceptance-matrix.md` describe
   only the assembled behavior that has actually landed.

## 7. Explicit Non-Goals

- No output-style or persona system.
- No user-authored prompt plugin API.
- No per-model prompt variants beyond the native/compat tool-call switch.
- No `<system-reminder>` style second injection channel in this pass. rove's
  layered memory already occupies that role.
