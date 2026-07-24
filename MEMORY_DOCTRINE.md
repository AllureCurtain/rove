# Memory Doctrine

This document records the memory and context rules implemented in this
repository. It is a runtime contract, not a future design note.

## Layers

rove uses three memory layers:

| Layer | Storage | Lifetime | Prompt role |
|---|---|---|---|
| Working memory | In-process `Message` values | Current run | Injected into every context build |
| Session memory | `memory.session_dir/<session_id>.md` | Current session and resume | Loaded as session summary context |
| Durable memory | `memory.durable_dir/MEMORY.md` plus `topics/*.md` | Cross-session | Recalled by relevance and injected as bounded context |

Working memory is not persisted by itself. Session and durable memory are
filesystem artifacts and follow the configured `memory.session_dir` and
`memory.durable_dir` paths.

## Durable Memory

Durable memory is for stable facts, preferences, project conventions, feedback,
and reference material. It is managed by:

- `save_memory`
- `update_memory_index`
- `read_memory_topic`

`save_memory` writes topic files with frontmatter:

- `title`
- `type`: one of `user`, `feedback`, `project`, `reference`
- `scope`: one of `global`, `project`, `session`
- `source`
- `confidence`
- `created_at`
- `updated_at`

The tool rejects unsafe topic names, likely secrets, and transient scratch
content before writing. Durable memory must not become a dump of logs, private
tokens, one-off debug output, or short-lived instructions.

## Recall

Prompt-time durable recall is bounded by `memory.recall_limit`. It reads the
durable index and topic files, then scores entries against the active query.

Recall is lexical but CJK-aware:

- CJK, Japanese kana, and Hangul text produce unigrams and overlapping bigrams.
- Latin-like words are lowercased and split on non-alphanumeric boundaries.
- Numeric tokens are preserved.

Ranking uses smoothed IDF with field weighting:

- title matches have the strongest weight;
- slug, type, source, description, and body can contribute;
- exact title phrase matches receive a bonus;
- `confidence` scales the score;
- recently updated topics can receive a small recency boost.

The prompt path recalls all memory types. Detailed recall APIs can use
`RecallOptions.type_filter` when a caller needs only one durable memory type.

## Session Memory

Session memory is deterministic markdown. At normal run completion, the
post-run hook writes a summary with the goal, final status, output excerpt,
completed plan steps, tools used, and write-set metadata when available.

Before automatic compaction collapses older history, the run and plan loops
extract durable-worthy notes from the messages about to be compacted. Those
notes are appended to the session summary file under `## Flush at <timestamp>`
blocks and a `MemoryFlushed` stream event is emitted. Later final summaries
preserve these flush blocks instead of overwriting them.

## Context

`ContextManager` builds prompts in this order:

```text
system -> durable memory -> session memory -> compact summary -> recent history tail -> current user message
```

History can be limited by message count or token budget. Token estimates are
approximate and provider-neutral.

Prompt metadata includes:

- `prompt_hash`: hash of the exact messages sent to the model;
- `stable_prefix_hash`: hash of the system prompt, working memory, and compact
  summary;
- `prompt_cache_key`: derived later from stable prefix and tool signature.

The stable prefix is not elided from provider requests. Current model clients
are stateless request boundaries, so system prompt, memory, and compact summary
must remain present in every model call unless a future provider contract
explicitly supports stateful prefix reuse.

## Compaction

Automatic compaction is considered when the context builder drops old history
or crosses the configured soft budget. Model-generated compaction is optional
and controlled by `runtime.model_compaction_enabled`.

The active compaction prompt version is `rove.compaction.v2`. It asks for a
structured summary with seven fields:

- goal
- decisions
- open tasks
- read files
- modified files
- tool results
- risks

If the model summary succeeds, the parsed structure is rendered back into a
prompt-friendly compact summary and recorded as `model_generated`. If the model
fails or returns unusable output, rove uses a deterministic structured fallback,
records degraded metadata, and keeps the run moving. Repeated failures open the
compaction circuit according to `runtime.compaction_failure_threshold`.

## Boundaries

Context/compaction and the session/durable memory implementation live under
`runtime/src/`; the root `rove::core::context` and `rove::memory` paths are
temporary compatibility re-exports. Engine coordination, the final session
summary hook, and built-in memory tools have not moved yet.

Durable memory is helpful context, not an authoritative database. Session memory
is a resumability aid, not a full transcript. Vector RAG is not a built-in product path; retrieval is tools + file memory
and should not be mixed into durable memory unless the fact is stable enough to
survive across sessions.

Future provider-specific prompt-cache work must preserve the stateless
`ModelClient` semantics unless the model boundary is changed deliberately.
