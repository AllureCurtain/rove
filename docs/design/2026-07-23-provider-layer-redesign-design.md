# Rove Provider Layer Redesign

> Status: **Implemented baseline; later cleanup decisions supersede transitional compatibility details**
>
> Date: 2026-07-23
>
> Progress ledger:
> [`../plans/2026-07-23-provider-layer-redesign.md`](../plans/2026-07-23-provider-layer-redesign.md)

Implemented: Stages 1-9 established protocol IDs, registry, framing, shared
transport, all four native protocol strategies, named profiles, registry-driven
bootstrap assembly through `ProviderClient`, the opt-in `external-adapter-v1`
process boundary, current-runtime documentation, and release-gate evidence.
Legacy native HTTP client modules remain in-tree only as parity-test references;
production assembly does not construct them.

The later
[`2026-07-24-cleanup-and-naming-decisions.md`](2026-07-24-cleanup-and-naming-decisions.md)
removed public legacy config migration and request-side `wire_protocol`
selection for the unreleased product. Where this document's transitional
compatibility story disagrees with that decision or current code, the cleanup
decision and [`docs/runtime/`](../runtime/README.md) win.

## 1. Scope and current truth

This document records the Provider architecture that was implemented through
Stages 1–9. It is design history, not a substitute for current API/config
documentation.

The current runtime has OpenAI Chat Completions, OpenAI Responses, Anthropic
Messages, Ollama, and Fake strategies behind the provider registry and
`ProviderClient`; older direct client modules are parity-test references.
Product selection and fallback assembly live in `apps/bootstrap/src/factory.rs`.
Provider-specific API inventory behavior lives in `apps/api/src/provider.rs`.
All clients already normalize output into `ModelEvent`, and routing operates on
`Box<dyn ModelClient>`.

The target keeps those successful boundaries. It changes how endpoint data,
wire behavior, transport, authentication, and model discovery are composed.

## 2. Required outcomes

The redesign must support two different extension promises explicitly:

1. **Profile-only endpoint onboarding.** Any endpoint that implements a known
   wire protocol can be configured without source changes. This includes
   official services, self-hosted servers, relays, gateways, and compatible
   vendors.
2. **Core-independent protocol onboarding.** A new native wire implementation
   can be registered without adding provider-name switches to Rove core.
3. **No-rebuild custom protocol onboarding.** An end user can eventually use a
   separately installed process adapter that speaks Rove's stable normalized
   adapter protocol. This is a distinct later phase and must not be implied by
   the in-process registry alone.

"Any endpoint" does not mean that arbitrary bytes can be interpreted
automatically. An endpoint must either implement a built-in wire protocol or be
paired with an explicit adapter. Unknown protocol identifiers fail closed.

## 3. Non-goals

- A second model event lifecycle for CLI, API, or Web.
- Provider-name or URL-substring inference as an authority boundary.
- Runtime downloading or executing untrusted provider packages.
- A stable Rust dynamic-library ABI.
- Sending raw provider credentials through browser-visible API fields.
- Rewriting routing, retry, health, or the Engine loop as part of this work.

## 4. Invariants

- `ModelClient`, `ModelEvent`, `Message`, `ToolSchema`, `Usage`, and
  `ModelError` remain the engine-facing contract.
- Provider-specific request and response shapes remain inside `rove-models` or
  an explicit external adapter.
- Routing target identity continues to include protocol, endpoint, and model.
- A profile selects behavior with an open protocol ID. It does not grant code
  execution, filesystem access, or permission.
- Secrets are redacted by construction and never appear in normal errors,
  traces, reports, config dumps, API payloads, or fixtures.
- Existing configurations remain usable through a bounded compatibility
  conversion while the new profile format is introduced.
- Each migration phase keeps deterministic fake-provider execution available.

## 5. Target architecture

```text
Engine / RoutingModelClient
          |
          v
ProviderClient implements ModelClient
  |       |            |             |
  |       |            |             +-- resolved auth and headers
  |       |            +---------------- ProviderProfile (data)
  |       +----------------------------- Transport (HTTP + framing)
  +------------------------------------- WireProtocol (behavior)
                                              |
                                              v
                                      StreamDecoder state machine
                                              |
                                              v
                                           ModelEvent
```

The composition boundary has five parts:

| Part | Responsibility |
|---|---|
| `ProviderProfile` | Endpoint URL, model, protocol ID, auth reference, headers, options, compatibility flags, and optional inventory configuration. |
| `WireProtocol` | Build a native request, select framing, create a per-stream decoder, and classify native HTTP errors. |
| `Transport` | Send bounded HTTP requests, inject resolved auth, frame streaming bytes, and drive a decoder. |
| `WireProtocolRegistry` | Resolve an open protocol ID to behavior; reject unknown IDs and duplicate registrations. |
| `ProviderClient` | Assemble profile, protocol, transport, and auth behind the existing `ModelClient` contract. |

## 6. Endpoint and protocol identity

`WireProtocolId` is an open string newtype. Built-ins expose stable constants:

- `openai-chat`
- `openai-responses`
- `anthropic-messages`
- `ollama-chat`
- `fake`
- later, `external-adapter-v1`

The set is intentionally not a closed provider enum. Finite mechanics such as
framing and authentication style remain enums because Rove owns those sets and
benefits from exhaustive matching.

Provider display names and profile IDs are not wire protocol IDs. For example,
profiles named `openrouter`, `vllm-local`, and `team-gateway` may all select
`openai-chat` while retaining distinct endpoint and model identities.

## 7. Protocol strategy and stream state

The protocol interface is side-effect free for request construction:

```rust
pub trait WireProtocol: Send + Sync {
    fn id(&self) -> &WireProtocolId;
    fn build_request(
        &self,
        input: &WireRequestInput<'_>,
    ) -> Result<WireRequest, ModelError>;
    fn framing(&self) -> Framing;
    fn decoder(&self) -> Box<dyn StreamDecoder>;
    fn classify_error(
        &self,
        status: StatusCode,
        headers: &HeaderMap,
        body: &str,
    ) -> ModelError;
    fn default_auth_style(&self) -> AuthStyle;
}
```

`WireRequest` is data: method, relative endpoint path, protocol headers, and a
JSON body. It cannot send a request. Transport owns all network effects.

Each response stream receives its own `StreamDecoder`. This makes fragmented
tool calls explicit state machines and prevents state from leaking between
requests:

```rust
pub trait StreamDecoder: Send {
    fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError>;
}
```

OpenAI Chat, OpenAI Responses, Anthropic, and Ollama retain their native
request and decoding rules. The migration moves existing tested logic; it does
not translate every provider through OpenAI format.

## 8. Shared framing and transport

Transport supports bounded framing mechanics shared by protocols:

- Server-Sent Events, including CRLF, comments, chunk boundaries, and multiple
  `data:` lines per event;
- JSON Lines, including partial network chunks and a final line without LF.

The byte framer must preserve UTF-8 characters split across network chunks. A
malformed complete frame is a typed stream error, not silently discarded.

Transport requirements:

- only `http` and `https` endpoint schemes;
- no credentials embedded in endpoint URLs;
- bounded error-body capture;
- configured request/connect/idle timeouts;
- bounded response buffering;
- explicit redirect policy;
- typed invalid-header and invalid-endpoint failures;
- cancellation by dropping the returned stream;
- auth injection after protocol request construction;
- no secret values in error strings or debug output.

## 9. Profiles, authentication, and compatibility

The target configuration is a map of named profiles:

```toml
[provider]
active = "team-gateway"

[provider.profiles.team-gateway]
wire_protocol = "openai-chat"
base_url = "https://gateway.example.test/v1"
model = "team/model"
auth = { style = "bearer", secret = { env = "TEAM_GATEWAY_KEY" } }

[provider.profiles.claude]
wire_protocol = "anthropic-messages"
base_url = "https://api.anthropic.com"
model = "claude-sonnet"
auth = { style = "header", header = "x-api-key", secret = { env = "ANTHROPIC_API_KEY" } }
```

Initial secret sources are environment variables and bounded files. A command
secret source is not part of the initial contract because it creates a code
execution boundary. If added later, it requires an explicit opt-in policy,
direct executable/argument representation, timeout and output limits, and
negative security tests.

Compatibility flags are explicit profile data. Standard behavior is the
default. URL substring detection may produce a non-authoritative suggestion in
user tooling, but runtime assembly never silently changes protocol or auth
based on a hostname.

Legacy flat provider fields are converted to an in-memory profile for one
documented compatibility window. The conversion is deterministic, warning is
redacted, and routing/fallback identity remains stable.

## 10. Inventory capability

Model inventory is an optional protocol/profile capability, not a switch on a
provider display name. A profile may define:

- no inventory support;
- a protocol default inventory endpoint and decoder;
- an explicit inventory path and response mapping for compatible gateways.

Inventory failure never prevents using an explicitly configured model unless
the caller requested validation. API and Web continue to pass environment
variable names, never secret values.

## 11. External adapter boundary

An in-process Rust registry does not satisfy no-rebuild custom protocols. The
later external adapter phase introduces a versioned subprocess protocol:

```text
Rove neutral request (length-bounded JSONL)
    -> explicitly configured adapter executable and argv
    -> adapter talks arbitrary provider wire protocol
    -> normalized ModelEvent JSONL
    -> Rove validates sequence, sizes, IDs, and terminal state
```

This boundary is opt-in and disabled unless configured. It must include:

- protocol version negotiation;
- direct executable plus argument array, never a shell command string;
- bounded startup, request, idle, and shutdown timeouts;
- bounded stdin/stdout/stderr;
- environment allowlisting and secret injection without logging;
- cancellation and child-process cleanup;
- strict event schema and lifecycle validation;
- no adapter-provided permissions or local paths;
- deterministic fixture adapters and negative tests.

Native built-ins remain preferred for performance and richer provider
features. The sidecar is the escape hatch for arbitrary formats, not the common
path.

## 12. Migration sequence

The authoritative statuses and evidence are maintained in the progress ledger.
The intended order is:

1. Contract and progress ledger.
2. Foundation types, registry, and byte-safe framing.
3. Shared HTTP transport and security bounds.
4. OpenAI Chat migration and parity tests.
5. OpenAI Responses migration and parity tests.
6. Anthropic and Ollama migration and parity tests.
7. Profiles, secrets, legacy conversion, factory, and routing assembly.
8. API inventory and Web endpoint convergence.
9. External adapter v1 for no-rebuild custom wire formats.
10. Compatibility cleanup and release evidence.

Every stage must compile independently. Old clients remain active until their
replacement passes request-shape, stream, tool-call, usage, error, identity,
and routing parity tests.

## 13. Acceptance criteria

The redesign is complete only when all of the following are evidenced:

- A new OpenAI-compatible endpoint is usable through profile data alone.
- OpenAI Chat, OpenAI Responses, Anthropic, and Ollama preserve native tool
  history, stream events, usage, and typed errors.
- Unknown protocol IDs fail with a redacted error listing available IDs.
- Duplicate protocol registration fails deterministically.
- SSE and JSONL framing pass fragmented UTF-8 and boundary tests.
- Routing, retry, health, and fallback contract tests remain unchanged.
- Legacy config fixtures load through the documented compatibility path.
- Config/API output proves secret values are absent.
- API and Web use the same profile contract as CLI/bootstrap.
- A deterministic external adapter fixture proves an otherwise unsupported
  wire format can be used without rebuilding Rove.
- Current `docs/runtime/` files describe only the behavior actually shipped.

## 14. Decisions recorded

- Named profile map plus `active` selection: accepted.
- Explicit protocol and compatibility configuration: accepted.
- Unknown protocol fallback to OpenAI: rejected.
- URL-substring protocol selection: rejected as runtime behavior.
- Environment and file secret sources first: accepted.
- Command secret source: deferred pending a separate security decision.
- Remote catalog service: outside this redesign; endpoint-local inventory only.
- Legacy config conversion: accepted for a bounded transition.
- External process adapter: required for the no-rebuild custom-format promise,
  but implemented only after native protocol convergence.
