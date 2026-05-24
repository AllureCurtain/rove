# rove

`rove` is a local-first, stateful agent runtime written in Rust. It provides a CLI, an HTTP API, a standalone web workbench, tool execution, resumable run state, layered memory, provider routing, and optional RAG indexing.

The runtime is designed around a small core engine:

```text
CLI / API / Web
    -> Engine
        -> Context + checkpoints
        -> Provider routing
        -> Tool orchestration
        -> Memory layers
        -> State store
```

## Quick Start

Run a local fake-model task without network credentials:

```bash
cargo run -- --model fake "echo hello from rove"
```

Inspect effective configuration:

```bash
cargo run -- dump-config
```

List resumable local task states:

```bash
cargo run -- sessions
```

Start the local API server:

```bash
cargo run --bin rove-api
```

Start the web workbench in another shell:

```bash
cd web-ui
npm ci
npm run dev
```

By default the API binds to `127.0.0.1:8787`, and the web workbench proxies `/api/*` to that local API.

## Main Entry Points

| Area | Path | Purpose |
|---|---|---|
| CLI | `src/main.rs` | One-shot task runs, config dump, sessions, and RAG indexing command dispatch. |
| API | `src/bin/rove-api.rs`, `src/interfaces/api/` | HTTP job lifecycle, SSE event streaming, approvals, inputs, and cancellation. |
| Web | `web-ui/` | Next.js workbench that consumes the API and SSE job stream. |
| Core runtime | `src/core/` | Engine loop, context building, planner, parser, executor, IDs, and workspace detection. |
| State | `src/state/` | File artifacts under `.rove/runs/` plus SQLite indexing in `.rove/state.sqlite`. |
| Models | `src/models/` | OpenAI-compatible, Anthropic, Ollama, fake providers, and routing fallback. |
| Tools | `src/tools/` | Filesystem, shell, memory, request input, MCP proxy, and optional RAG tools. |
| Memory | `src/memory/` | Session summaries and bounded durable memory recall. |
| Docs | `docs/runtime/` | Current architecture, subsystem boundaries, and implementation status. |

## Configuration

Configuration is layered as:

```text
defaults < .rove/config.toml < environment < CLI overrides
```

Common environment variables:

| Variable | Purpose |
|---|---|
| `ROVE_MODEL` | Primary model override. Use `fake` for local deterministic smoke runs. |
| `ROVE_PROVIDER` | Provider name: `openai`, `openai-compatible`, `anthropic`, `ollama`, or `fake`. |
| `OPENAI_API_KEY` | OpenAI-compatible API key. |
| `OPENAI_API_BASE` | OpenAI-compatible API base URL. |
| `ANTHROPIC_API_KEY` | Anthropic API key. |
| `ROVE_API_BIND_ADDR` | API bind address override. Defaults to `127.0.0.1:8787`. |
| `ROVE_FALLBACK_MODELS` | Comma-separated fallback model list using the primary provider. |

`dump-config` prints the effective config, source summary, resolved paths, and secret-redacted provider fields.

## State Layout

Runtime state is written under `.rove/` by default:

```text
.rove/
  state.sqlite
  runs/<run_id>/trace.jsonl
  runs/<run_id>/task_state.json
  runs/<run_id>/report.json
  memory/MEMORY.md
  memory/topics/*.md
  memory/sessions/<session_id>.md
```

Files are the readable artifacts. SQLite is the index used for listing, replay, and restart-aware API job state.

## RAG

The RAG subsystem is optional and compiled behind the `rag` feature:

```bash
cargo test --features rag
cargo check --features rag --bin rove-index
cargo test --features rag --test cli_index deterministic_index_run_writes_manifest -- --exact
```

Index a workspace with deterministic local embeddings:

```bash
cargo run --features rag --bin rove-index -- --deterministic -C .
```

## Verification

Default Rust and web checks:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

cd web-ui
npm ci
npm test
npm run typecheck
npm run build
```

RAG feature checks:

```bash
cargo clippy --all-targets --features rag -- -D warnings
cargo test --features rag
```

## Runtime Docs

Start here:

- [Runtime Architecture](docs/runtime/architecture.md)
- [Subsystem Design](docs/runtime/subsystems.md)
- [Implementation Status](docs/runtime/implementation-status.md)

Older design notes remain in `docs/` and `docs/superpowers/specs/`; the `docs/runtime/` files describe the current implementation surface.
