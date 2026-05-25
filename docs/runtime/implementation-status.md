# Current Implementation Status

This matrix compares the runtime hardening target with the current implementation.

| Area | Current status | Remaining gap |
|---|---|---|
| Local-first default | API defaults to `127.0.0.1:8787`; CLI and state default to the workspace. Remote bind requires token auth unless explicitly marked unsafe. | None for the current local-first target. |
| Configuration | Typed config with defaults, `.rove/config.toml`, env, CLI/API overrides, validation, source summary, redacted dump output. | More CLI fields could be exposed as explicit overrides over time. |
| State layer | Files under `.rove/runs/` plus SQLite index for sessions/jobs/runs/events/reports/task states. WAL, foreign keys, migrations, busy timeout, lazy task-state import, TTL cleanup, and explicit `rove state repair/cleanup` commands exist. | Pending approval/input reconstruction remains intentionally out of scope. |
| API jobs | Live registry stores active handles; SQLite stores durable job/run/event state. Restart marks stale running jobs `interrupted`; historical state and SSE replay read from SQLite. | Pending approvals/inputs are schema slots but are not reconstructed after restart. |
| Status semantics | `init`, `running`, `done`, `error`, `cancelled`, and `interrupted` are represented. | None for the current lifecycle target. |
| Context budgets | Token-estimated context builder with soft, hard, and reserved budgets. | Token counting is approximate, not provider tokenizer based. |
| Prompt checkpoints | `PromptCheckpoint` stores summary, preserved tail, plan, memory pointers, last step, last event seq, token estimate, compacted count, and compaction metadata. Resume prefers checkpoint tail/summary. | Model-generated richer summary text is not implemented. |
| Tool orchestration | Tool batches can run parallel when every call is non-destructive and `parallel_safe`; results are emitted in deterministic call order. Destructive tools go through approval. | No separate batch hook layer yet. |
| Provider abstraction | OpenAI-compatible, Anthropic, Ollama, and Fake are peer providers behind `ModelClient`. Stream events are normalized to `ModelEvent`. | Provider-specific advanced features remain intentionally thin. |
| Routing and fallback | Fallback models and native fallback providers are supported. Fallback happens before committed visible output/tool-use and uses health threshold/cooldown. | More detailed retry/backoff policy could be added. |
| Memory layers | Working prompt memory, session summary files, durable topic files, bounded durable recall, and guarded durable promotion through `save_memory`. | Durable recall is lightweight lexical relevance, not a full knowledge system. |
| RAG | Feature-gated staged RAG pipeline with LanceDB, manifest fallback, deterministic embeddings, retrieval channels, postprocessing, eval reports, RAG prompt formatting, and lightweight code-aware chunking. | Full production embedding/provider management remains optional. |
| API security | Config includes bind address, token auth, CORS origins, rate limit, and unsafe remote override. Middleware enforces bearer token auth, CORS allowlists, and per-process request limits. | Multi-user identity and distributed rate limiting are out of scope. |
| Web | Standalone Next.js workbench with tests, typecheck, and build in CI. | Browser-level end-to-end tests are not part of default CI. |
| CI | Default Rust/Web workflow and separate RAG workflow are split. | Optional nightly/full workflow is not present. |
| Docs | Root README and runtime docs explain quick start, architecture, subsystems, and current-vs-target status. Older `04/05/06` docs are marked historical. | Keep future architecture updates centered in `docs/runtime/`. |

## Acceptance Criteria Mapping

| Criterion | Status |
|---|---|
| Default running remains local-first. | Met |
| Config has multi-source priority and secret redaction. | Met |
| State uses file artifacts plus SQLite index. | Met |
| Resume prefers checkpoint reconstruction. | Met |
| Context is token-budget and segmented prompt aware. | Met, with approximate token estimates |
| Compaction can automatically trigger with degradation/circuit semantics. | Met for deterministic checkpoints; model-generated summaries remain future work |
| Tool calls support batch parallelism with stable writeback order. | Met |
| Provider layer is unified with native peers. | Met |
| Memory is working/session/durable with controlled promotion. | Met |
| API job/state is durable and live handles are active-only. | Met |
| CI covers Rust/Web/RAG in separate layers. | Met |
| Root README explains the project mainline. | Met |
