# Subsystem Design

## Configuration

Configuration is typed in `src/config.rs` and grouped by runtime, provider, tool, memory, state, API, web, and routing.

Merge order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

Validation currently covers provider names, model values, fallback providers, routing thresholds, token budgets, SQLite timeout, memory recall limit, API remote-bind safety, and workspace-relative paths. `rove dump-config` prints effective config with secrets redacted and resolved path fields.

## State, Job, And Run

`StateStore` coordinates file artifacts and the SQLite `StateIndex`.

Files:

- `trace.jsonl` records append-only runtime events.
- `task_state.json` stores resumable task state and prompt checkpoint.
- `report.json` stores final status, output, and identity metadata.

SQLite:

- stores sessions, jobs, runs, events, reports, task state metadata, pending approval/input tables, and replay offsets;
- uses schema migrations, foreign keys, WAL, `synchronous=NORMAL`, and a bounded busy timeout;
- exposes async helpers through `spawn_blocking` where API handlers need indexed reads.

## Context And Compaction

`ContextManager` supports token-aware prompt construction with soft, hard, and reserved budgets. Prompt order is:

```text
system -> durable memory -> session memory -> compact summary -> recent history tail -> current user message
```

`TaskState` can include a `PromptCheckpoint` with summary, preserved tail, plan pointer, memory pointers, last step, optional last event seq, token estimate, and compacted message count. Resume prefers this checkpoint over replaying full audit history.

Current compaction is deterministic and artifact-based. It writes checkpoint summaries and preserved tails; it does not yet call a model to synthesize richer summaries or implement a multi-attempt compaction failure circuit.

## Provider And Routing

The model boundary is `ModelClient`, which streams normalized `ModelEvent` values. Native providers are peers:

- OpenAI-compatible
- Anthropic
- Ollama
- Fake

Fallback can be configured as:

- `provider.fallback_models`: model names using the primary provider;
- `provider.fallback_providers`: explicit provider/model/base/key records.

`RoutingModelClient` can fall back before user-visible content or committed tool-use begins. It tracks provider health with a failure threshold and cooldown.

## Tool Orchestration

Tools are registered in `ToolRegistry` and executed through `Executor`. Tool schemas include `destructive` and `parallel_safe` flags.

Batch execution rules:

- multiple non-destructive, parallel-safe calls may run concurrently;
- destructive, unknown, shell, write, request-input, and memory-write style calls serialize through the approval and execution boundary;
- conversation history and trace events are written back in model call order after a batch completes.

Approval policy is `ask`, `auto`, or `never`. The CLI uses stdin for approvals; the API exposes pending approvals through `/jobs/{job_id}/approvals/{call_id}`.

## Memory

The memory model has three layers:

- working memory: in-run prompt messages built by the engine;
- session memory: `.rove/memory/sessions/<session_id>.md`, written by a post-run hook and used on resume;
- durable memory: `.rove/memory/MEMORY.md` plus `.rove/memory/topics/*.md`, managed through memory tools.

Durable recall is bounded by `memory.recall_limit` and query relevance. The `save_memory` tool rejects unsafe topic names, obvious secrets, and transient one-off content before writing durable files.

## API And Security

The API routes are:

- `POST /jobs`
- `GET /jobs/{job_id}/state`
- `GET /jobs/{job_id}/events`
- `POST /jobs/{job_id}/cancel`
- `POST /jobs/{job_id}/approvals/{call_id}`
- `POST /jobs/{job_id}/inputs/{input_id}`

The API default is local-only binding. Config already has slots for token auth, CORS origins, rate limits, and an explicit unsafe remote-without-auth override. Token auth, CORS enforcement, and rate limiting are config surfaces rather than complete middleware in the current implementation.

## RAG

The RAG implementation is behind `--features rag` and lives under `src/tools/rag/`. It includes:

- deterministic and OpenAI-compatible embedders;
- staged ingestion with logging;
- markdown-aware and fixed chunking;
- LanceDB storage plus manifest fallback;
- vector, lexical, and path-scoped retrieval channels;
- postprocessing for dedupe and score normalization;
- pure retrieval eval reports.

Default builds expose stub `retrieve_code` and `retrieve_docs` tools that explain the feature requirement.

## Web

`web-ui/` is a standalone Next.js app. It talks to the Rust API through a local rewrite from `/api/*` to `ROVE_API_BASE` or `http://127.0.0.1:8787`.

The web verification surface is:

```bash
npm test
npm run typecheck
npm run build
```

## CI

CI is split by dependency weight:

- `.github/workflows/ci.yml`: Rust default fmt/clippy/test and web test/typecheck/build.
- `.github/workflows/rag-ci.yml`: RAG feature clippy, full `--features rag` tests, and `rove-index` feature/smoke coverage.

RAG remains separate so DataFusion/LanceDB dependencies do not slow every default feedback loop.
