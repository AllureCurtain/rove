# Rust style — rove

New and changed Rust follows the
[Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html),
filtered by this file. The full guide text lives beside it at
[`docs/rust-style-google.md`](rust-style-google.md) (fetched 2026-11-07).

rove is a shipped binary workspace — a single-user local runtime powering a
CLI/TUI, an HTTP API, a desktop host and benchmarks — not a library published
for out-of-tree consumers. So the guide's public-API chapters are strong
recommendations rather than gates. What this file says is binding.

Two sources of truth:

- **This file** — every rule a reviewer can enforce, plus each Google-guide
  item the project either adopts verbatim or deliberately varies.
- **`docs/development.md`** — commands, test attributes, CI checks.

When a rule below disagrees with the Google guide, this file wins; the entry
names what the guide asks and why the project chooses differently.

## Binding rules, project-specific

These encode a decision rove made, not a Google item restated.

- **`expect` over `unwrap`, and the message names the invariant.** Where a
  panic is possible, `.expect("the <thing> lock is poisoned")` is the required
  form; the message names the violated invariant, not just "failed".
  `.unwrap()` is test-only. Production code currently has a small number of
  legacy `unwrap`s — each change that touches one downgrades it to `expect`
  with a real invariant; we do not run a repo-wide unwrap purge.

- **Lock-poison handling.** `std::sync::Mutex`/`RwLock` locks use
  `.expect("the <field> lock is poisoned")`; rove never reaches inside a
  poisoned lock to salvage the value (the guide's "Handling poison" is adopted
  verbatim). Tokio `Mutex`/`RwLock` do not poison and need no handling.

- **`StreamEvent` is the only lifecycle contract.** New lifecycle or progress
  kinds extend `StreamEvent`; we never introduce a parallel event set for one
  surface (persistence, SSE, Web, tests all share it). This restates an
  `AGENTS.md` hard rule because it constrains how review/tool/event code is
  shaped.

- **`#[allow(dead_code)]` names its consumer.** An unused item must carry a
  comment saying who keeps it alive. A bare `#[allow(dead_code)]` hides code
  nobody calls.

- **`#[allow(clippy::...)]` is per-item and justified.** The project already
  allows a small set (`double_must_use` on intentional `&self`-borrow methods,
  `too_many_arguments` on kernel/plan boundaries). Adding one needs a comment
  or a `docs/decisions.md` entry; widening to module/crate level does not.

## Code organization

- **A crate is a unit of compilation, not a folder of code.** rove splits
  along crate boundaries where the dependency graph already runs one way
  (`rove-protocol` → `rove-models` → `rove-core` → `rove-runtime` →
  `rove-app-bootstrap` → `rove-cli`/`rove-api`/`rove-bench`;
  `rove-product-store` is the side store `rove-api` owns). We do **not**
  adopt the Chromium "one directory per crate" layout — that `[chrome]` rule
  targets a non-Cargo build graph; rove uses idiomatic Cargo workspaces with
  `src/` trees.

- **Non-leaf modules hold declarations and re-exports only.** A `mod.rs` /
  `lib.rs` that declares submodules carries `mod` lines, `pub use`, and at
  most the small glue that ties children together. Implementation belongs in
  a leaf module. Existing violations are cleaned when a change already
  touches that directory, never in a repo-wide reflow. Known offenders as of
  2026-11: `runtime/src/review/mod.rs` (~2060 lines), `runtime/src/tools/hooks/mod.rs`,
  `apps/product-store/src/store/mod.rs`.

- **Name leaf modules after their primary item.** `index.rs` holds
  `StateIndex`, `boundary.rs` holds `Boundary`/`WorkspaceKind`, etc. A module
  that is genuinely a grab-bag may take a topical name, but
  one-type-one-file is the default.

- **Use statements import items from one module each.** A `use` whose brace
  list pulls from two or more different modules is a violation
  (`use tokio::{io::AsyncReadExt, net::TcpListener}` is two modules). Brace
  lists that import several items from the *same* module
  (`use a::{C, D}`) are fine. Current count ~14 violations, mostly `std::{…}`
  and `tokio::{…}` combined lists — cleaned per directory, not in one pass.

- **No glob imports in production code.** `pub use foo::*` re-exports in a
  `mod.rs` are allowed (they define the module's surface); `use foo::*` inside
  implementation files is not. `use super::*` is allowed only inside a
  `#[cfg(test)]` module, matching the guide's own carve-out.

- **Top-level functions are called qualified by module.** `use` imports
  types and traits unqualified; a free function keeps its module prefix at
  the call site (`memory::durable::load(...)`, not `load(...)` when `load`
  was imported directly).

- **Type names are imported and used unqualified** — `HashMap`, `Arc`,
  `Duration`, `PathBuf`. The guide's common-name exception applies:
  `serde_json::Value`, `std::fmt::Formatter`, `std::io::Error`,
  `anyhow::Result`, and UTOIPA `utoipa::ToSchema` keep their qualifier.

## Formatting

- **`cargo fmt --all --check` and clippy are the gate.** The workspace keeps
  rustfmt defaults plus `max_width = 100` (the rustfmt default, recorded
  explicitly in `rustfmt.toml`). We do **not** pin a nightly toolchain for
  formatting — `imports_granularity`/`format_macro_matchers` are nightly-only
  and unstable, and the project values a stable-only toolchain more than the
  extra normalizations (this differs from oxsum's pinned-nightly choice; see
  `docs/decisions.md`).

- **`#[rustfmt::skip]` only where the guide allows** — alignment-sensitive
  tables or generated code, with a comment.

## Ownership, functions, types

- **`Rc`/`Arc` where sharing is real.** The guide's "avoid `Rc`/`Arc`" targets
  gratuitous shared ownership in application code; rove's `Arc` uses are the
  runtime's shared state (`Engine`, `StateIndex`, `ToolRegistry`,
  `Arc<dyn Tool>`) and tokio primitives — these are the design, not a smell.
  `Rc` stays at zero; anything single-threaded that wants sharing reaches for
  `Arc` + `Mutex` anyway because the runtime is multi-threaded.

- **Borrows over deep copies**; `&[T]`/`&str`/`Option<&T>` parameters. The
  two known `&Option<T>` params (`engine/facade.rs` `trace_writer`,
  `models/error.rs` `retry_after_ms`) get fixed when their file is next
  touched.

- **Errors are `Result` + typed error enums** (`thiserror` at module edges,
  `anyhow` only inside the engine's private plumbing). Panics are reserved for
  invariant violations, always via `expect`.

- **`#[non_exhaustive]` on public wire/state types.** `rove-protocol` is the
  lowest shared contract and `rove-product-store`'s `contracts.rs` is the
  persisted product contract — their public enums and public-field structs
  carry `#[non_exhaustive]` so additive changes don't break callers or old
  state files. Internal application types don't bother.

- **`#[must_use]` on fallible handles and pure queries.** Handles like
  `RunHandle`/`SessionGuard` and pure query methods return `#[must_use]` so a
  dropped result is a compile error, not a silent leak. Currently ~0 — added
  incrementally where a dropped result has bitten us.

- **`new`/`try_new`/`from_*` naming.** A `new` that can fail is `try_new`;
  `From` impls get a paired inherent `from_*` constructor when the type is
  user-facing.

## Concurrency

- **Adopt the guide's poison handling verbatim** (see Binding rules above).
- **`async` is the runtime's model.** The Chromium "don't use `async`" rule is
  inverted here: the engine, tools and MCP transport are `async` on tokio, and
  that's the architecture, not a style choice.
- **`std::thread` is allowed** for the few places that need an OS thread
  outside the runtime (input pumps, blocking subprocess waits); inside the
  engine, work goes through tokio tasks.
- **Fuchsia's `fuchsia_sync::Mutex`** — not applicable; target is
  macOS/Linux/Windows, `std`/`tokio` sync primitives.

## Documentation

- **Every public item on a published-boundary crate is documented** —
  `rove-protocol`, `rove-product-store` contracts, `rove-models` provider
  surface. Internal crates don't run `missing_docs` but do document every
  module with `//!`.
- **`//!` module docs on every non-test source file.** ~176 files currently
  lack one; added when a file is touched, not in a sweep.

## Macros

- `macro_rules!` only where a function or derive cannot express the pattern;
  today a handful exist (serde validation, test helpers). Procedural macros
  are not introduced without a `docs/decisions.md` entry.

## Deliberate variations from the Google guide

Each of these is a place the project looked at the guide and chose otherwise:

- **`[chrome]` small-crates →** adapted. We split along real dependency
  boundaries (done: `rove-product-store` out of `rove-api`, test-binary merge
  27→3); we do not pursue one-type-per-crate, and the `rove-runtime`
  multi-module split was explicitly deferred after measurement — see
  `docs/refactor-plan.md` item 6.

- **`[chrome]` flat-file layout →** not adopted. Nested `src/` trees under one
  crate (`runtime/src/tools/mcp/`, `apps/api/src/product/transcript/`) are
  normal Cargo and fine.

- **Returning `impl Iterator` everywhere →** application code may return
  `Vec<T>`/`BTreeMap<K,V>` when callers would collect; the guide's own
  "application code" carve-out is adopted.

- **`impl Into<T>` parameters →** allowed at operator-facing boundaries
  (config, CLI, API types) where call-site ergonomics dominate; hot inner
  paths take concrete types so the signature tells the truth about storage.

- **Chromium "no `std::thread`", "no `async`" →** inverted, see Concurrency.

## Legacy cleanup rule

Clean per directory when a change already touches it; never run a repo-wide
import or `unwrap` reflow. Historical commits keep the style they landed
with. The standing list of known deviations lives in this file under the
relevant rule, so it stays accurate as code changes — update it when you fix
one.
