# Cleanup and Naming Decisions — 2026-07-24

> Status: **Implemented — W1/W2a/W2b/W3 complete on `main`**
>
> Scope: post–provider-redesign cleanup, naming vocabulary, Core/Runtime boundary,
> tools, config shape, and phased delivery (W1 / W2a / W2b / W3).
>
> Execution plan:
> [`../plans/2026-07-24-cleanup-w1-w2-w3.md`](../plans/2026-07-24-cleanup-w1-w2-w3.md)
>
> Product has **not** shipped. There is **no** legacy compatibility window and
> **no** migration layer for old config or old code paths.

Implementation evidence: W1 `8ffb291`, W2a `d13646a`, W2b `9847fdd`, and W3
`a3f2681`. The remainder of this document records the decisions that produced
those changes; current runtime truth remains [`docs/runtime/`](../runtime/README.md).

## 1. Product stance

rove is a **local-first coding / agent runtime** (closest neighbors: PI structure,
coding-agent tool path). It is **not**:

- an OnCall-style RAG/AIOps platform as the default path;
- an OpenClaw-style multi-channel personal-assistant gateway as the near-term goal.

Workspace context today:

```text
tools (read/write/shell) + layered file memory
```

Built-in LanceDB vector RAG was removed (`f1de256`). Optional external semantic
retrieval is **future only** (W3 documents first-class search; vector RAG is
out of this cleanup series).

## 2. Vocabulary (must stay consistent)

| Layer | Field / name | Values / examples | Who sets it |
|---|---|---|---|
| UI label | **Type** | OpenAI, OpenAI Responses, Anthropic, Ollama, Fake | User |
| Product field | **`provider_type`** | `openai`, `openai-responses`, `anthropic`, `ollama`, `fake` | User / config / API request |
| Wire protocol field | **`wire_protocol`** | `openai-completions`, `openai-responses`, `anthropic-messages`, `ollama`, `fake` | **System only** (mapped from `provider_type`) |
| User copy | **OpenAI** | Never “OpenAI-compatible” as a product type | Docs / UI |

### 2.1 Mapping table (authoritative)

| `provider_type` | UI | `wire_protocol` |
|---|---|---|
| `openai` | OpenAI | `openai-completions` |
| `openai-responses` | OpenAI Responses | `openai-responses` |
| `anthropic` | Anthropic | `anthropic-messages` |
| `ollama` | Ollama | `ollama` |
| `fake` | Fake | `fake` |

Rules:

- Requests and config **must not** accept a user-supplied `wire_protocol` override.
- Responses **may** include read-only `wire_protocol` for debugging.
- Chat Completions internal id is **`openai-completions`** (aligned with PI/OpenClaw),
  not bare `openai` (avoids colliding with product type) and not `openai-chat`.
- Implementation modules/files should follow wire ids
  (e.g. `openai_completions.rs`, not a single ambiguous `openai.rs` for Chat).

### 2.2 Why not bare `openai` for Chat wire id?

OpenAI exposes at least two HTTP shapes. Product type `openai` means “Chat Completions
family endpoints (official or gateway)”; product type `openai-responses` means Responses.
Internal wire ids must distinguish those shapes. PI/OpenClaw use
`openai-completions` / `openai-responses`; Hermes uses `api_mode` values such as
`chat_completions`.

### 2.3 CC Switch note

CC Switch Codex configs use **`wire_api`** (e.g. `wire_api = "responses"`) for protocol
shape inside provider blocks. That is the **wire** layer, not the user Type field.
rove keeps the name **`wire_protocol`** for that layer and **`provider_type`** for Type.

## 3. Delete-all legacy policy

Because the product is unreleased:

| Remove | Do not keep |
|---|---|
| Legacy native clients used only as dual production paths (`models/src/openai.rs` et al. once parity is folded into the new stack) | “Compatibility window” as a standing product feature |
| `LegacyProviderKind` / `legacy_targets` / flat `[provider] name/api_base/api_key` assembly | Silent ignore of old keys |
| `channel` as the public product field | Dual public names for the same API |
| `OpenAiCompatible` product/code naming | “OpenAI-compatible” as UI type |
| Compat plan events (`plan_step_*` dual-fire) | Forever dual event streams |
| Dual assembly entrypoints | `build_product_engine` ≡ `build_interface_engine` forever |
| Local garbage | `.rove/rag.lancedb`, `.rove/rag_eval` as meaningful product state |

**No migration writer.** Old shapes are invalid; loaders fail with a clear message
or code is simply deleted so old shapes cannot run.

## 4. Config shape

**Profiles + env/CLI override for the current run (option C).**

Illustrative:

```toml
[provider]
active = "relay"

[provider.profiles.relay]
provider_type = "openai"
base_url = "https://example.invalid/v1"
model = "gpt-4.1-mini"
# auth via env reference only this phase

[provider.profiles.claude]
provider_type = "anthropic"
base_url = "https://api.anthropic.com"
model = "claude-..."
```

- Secrets this phase: **`api_key_env` only** (no UI/API literal keys; file/exec later if needed).
- Per-request API/Web profile: `provider_type` + `api_base` + `api_key_env` + `model` (+ optional display name).
- No request field for writable `wire_protocol`.

## 5. Tools

### 5.1 Naming style

**Scheme A — verb_object**, applied with **direct rename (no aliases)**.

### 5.2 Default model-visible tool names (authoritative)

| Current | New | Notes |
|---|---|---|
| `fs_read` | `read_file` | |
| `fs_write` | `write_file` | |
| `shell` | `run_shell` | |
| `save_memory` | `save_memory` | Writes topic; already triggers internal reindex |
| `update_memory_index` | **`reindex_memory`** | Rebuilds `MEMORY.md` from `topics/*.md` — **not** list |
| `read_memory_topic` | **`read_memory`** | Read one topic by name |
| `request_input` | `request_input` | |
| `echo` | not in default model registry | tests only |
| — | `search_code` | First-class structured workspace search (W3); not vector RAG |

Code basis for memory names: `runtime/src/tools/memory.rs`
(`UpdateMemoryIndexTool` description: “Rebuild the durable memory index…”).

## 6. Events and type pairs

### 6.1 Three event layers (keep all three)

```text
ModelEvent   →  models   (LLM stream)
AgentEvent   →  core     (embeddable loop)
StreamEvent  →  runtime  (durable / SSE / Web / TUI)
```

Conversions only at explicit boundaries. No casual re-exports that blur layers.

### 6.2 Tool description vs model schema

| Concept | Name |
|---|---|
| Runtime tool metadata (risk flags, etc.) | `ToolDescriptor` |
| JSON schema sent to the model | **`ModelToolSchema`** |

Prefer `model_schema()` / `model_schemas()` naming so `schemas()` does not return descriptors.

### 6.3 Plan lifecycle

Keep only the new facts (e.g. `step_result`, `plan_decision`, `plan_revised`).
Remove dual-fire compatibility plan-step events.

### 6.4 Execution config

**Single source of truth: `ExecutionPolicy`** (and necessary constructors).
`max_steps` / `plan_enabled` are not a second parallel model; if CLI/API keep
`max_steps`, it is sugar that writes into `ExecutionPolicy`.

## 7. Public assembly API

Single short names (no dual aliases):

```text
build_engine
tool_registry
```

Related helpers share the same prefix style; delete
`build_product_engine` / `build_interface_engine` /
`product_tool_registry` / `default_tool_registry` dual exports.

## 8. Core vs Runtime (PI-inspired, structure cleaned)

| Layer | Role | Default consumer |
|---|---|---|
| `rove-models` | Wire protocols, `ModelClient`, routing | everyone below |
| `rove-core` | Embeddable `Agent`, tool contracts | libraries / tests |
| `rove-runtime` | Durable `Engine`, plan, state, memory, tool impls | **CLI / API / Web / Bench** |
| `apps/*` | Product shells | end users |

**Product default entry: `runtime::Engine`.**  
**`core::Agent` is embed-only.**

Cleanup depth for this series: **option C**

- **W2a:** docs + public API / dual-name removal / entry narrative  
- **W2b:** physical regroup of `runtime/src` by domain (`engine/`, `planning/`, `tools/`, `state/`, `memory/`, …)

## 9. Phases

| Phase | Focus |
|---|---|
| **W1** | Delete old provider/config paths; `provider_type` + mapping; current docs; remove local rag junk |
| **W2a** | Tool renames; event/assembly/`ExecutionPolicy` convergence; Core/Runtime narrative |
| **W2b** | Runtime directory regroup |
| **W3** | First-class `search_code` (specified in docs now; implement later) |

## 9.1 Implementation worktrees (mandatory)

All implementation for this cleanup series runs in a **git worktree under the
repo-local** `.worktrees/` directory (gitignored). Do **not** implement on
`main` in the primary checkout, and do **not** create sibling folders next to
the repo (e.g. `../rove-something`) for this work.

Reasons:

- `main` stays clean and pullable;
- failed experiments are discarded with `git worktree remove` without touching
  the primary tree;
- matches existing repo practice (see historical
  `docs/plans/2026-07-18-tui-parallel-worktree-handoff.md`).

### Rules

1. **Commit the documentation baseline first** so the worktree starts from a
   clean index (no unstaged decision/plan files left only on disk).
2. Create the worktree **from that commit** (usually `main` after the docs
   commit, or the cleanup branch tip):

   ```powershell
   # from repo root D:\Study\project\agent\rove
   git status --short          # must be empty before add
   git worktree add .worktrees/cleanup-w1 -b cleanup/w1 main
   git worktree list
   ```

3. One conversation / agent owns one worktree. Do not share a worktree across
   parallel agents. Do not share `CARGO_TARGET_DIR` across worktrees.
4. Suggested layout for this series:

   ```text
   .worktrees/cleanup-w1     branch cleanup/w1      # W1
   .worktrees/cleanup-w2a    branch cleanup/w2a     # after W1 merges (optional)
   .worktrees/cleanup-w2b    branch cleanup/w2b
   .worktrees/cleanup-w3     branch cleanup/w3      # search_code later
   ```

5. When a wave merges to `main`, remove its worktree and delete the local
   branch if finished:

   ```powershell
   git worktree remove .worktrees/cleanup-w1
   git branch -d cleanup/w1
   ```

6. Primary checkout remains on `main` for docs, review, and integration only
   unless explicitly integrating.

Details and acceptance checklists: see the delivery plan.

## 10. Non-goals for this series

- Built-in vector RAG / LanceDB resurrection  
- Hermes multi-channel gateway / skills learning loop as default  
- OpenClaw full plugin marketplace  
- Secret file/exec providers (beyond documenting as later)  
- Keeping dual public names “for safety” after renames  

## 11. Decision log (grill)

Decisions frozen in discussion 2026-07-24, including:

1. Wire Chat id: `openai-completions`  
2. Product field: `provider_type` (not `channel`, not `type`)  
3. Protocol field: `wire_protocol` (request immutable; response optional read-only)  
4. Full delete of legacy (no migration)  
5. Tool scheme A + direct rename  
6. Tool table including `reindex_memory` / `read_memory` after code inspection  
7. Search: W3 + must document  
8. Events: three layers kept  
9. `ToolDescriptor` + `ModelToolSchema`  
10. Assembly: `build_engine` / `tool_registry`  
11. Engine product / Agent embed  
12. Mapping table including `ollama` → `ollama`  
13. No user `wire_protocol` override  
14. Response may echo `wire_protocol`  
15. Config: profiles + env/CLI override  
16. Secrets: `api_key_env` only this phase  
17. Plan events: new only  
18. `ExecutionPolicy` as sole truth  
19. Three delivery waves  
20. Core/Runtime cleanup level C (W2a+W2b)  
21. Docs: design + plan pair  
22. Memory names from real code behavior  

## 12. References

- Provider redesign design: [`2026-07-23-provider-layer-redesign-design.md`](./2026-07-23-provider-layer-redesign-design.md)  
- Provider redesign plan: [`../plans/2026-07-23-provider-layer-redesign.md`](../plans/2026-07-23-provider-layer-redesign.md)  
- Runtime SoT: [`../runtime/README.md`](../runtime/README.md)  
- PI / OpenClaw known API ids: `openai-completions`, `openai-responses`, `anthropic-messages`  
- CC Switch Codex provider blocks: `wire_api` (protocol shape), not product Type  
