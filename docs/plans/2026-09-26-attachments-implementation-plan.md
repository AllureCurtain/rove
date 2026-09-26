# Attachments And Image Upload Implementation Plan (R8)

> Status: **Proposed / Not Implemented**. No product code exists for this plan. Nothing
> in `docs/runtime/implementation-status.md` or `docs/runtime/acceptance-matrix.md` is
> claimed by this document.
> Date: 2026-09-26 (inventory refreshed 2026-09-27 against `main`).
> Baseline: `main @ 31629d5` (`Merge pull request #74 from AllureCurtain/feature/runtime-align-r4`,
> 2026-09-27). Branch `feature/runtime-align-r8-plan` is at that commit with no local
> divergence. Every "current" claim below was read in this worktree at that commit.
> Inputs:
>
> - Design contract: [runtime contract alignment design](../design/2026-09-26-runtime-contract-alignment-design.md) §0
>   (invariant table), §0.1 (R8 row), §8 (§8.1–§8.4).
> - Security companion: [attachments threat model](../design/2026-09-26-attachments-threat-model.md)
>   (same branch; the design's §8.1 gate requires both documents before work starts).
> - Format and rigour precedent: [P5b local HTML preview threat model](../design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md).
> - Current-state docs this change would have to update when implemented:
>   [implementation guide](../runtime/implementation-guide.md) §5, §15, §19,
>   [subsystems](../runtime/subsystems.md) §"API And Security", §"Web",
>   [state layout and migration](../../STATE_LAYOUT_AND_MIGRATION.md).

The design's §8.1 is explicit: §8 freezes only the contract skeleton and the security
baseline, and implementation may not start before this plan (including the provider-layer
injection design) has been reviewed. This document is that plan. It deliberately answers
the open point in §8.2 rather than carrying it forward.

## 1. Scope and non-goals

### 1.1 In scope (first batch)

1. A session-scoped, API-controlled attachment root outside every workspace, with
   server-generated identifiers.
2. `POST /product/sessions/{id}/attachments` (single attachment, raw body), with
   extension allow-list + magic-byte sniffing + MIME allow-list + secret warning.
3. `GET /product/sessions/{id}/attachments/{aid}` (inline raster or attachment download),
   with server-fixed `Content-Disposition` and a `Content-Type` taken from the durable
   record.
4. An additive `attachments: Vec<ProductMessageAttachmentRef>` field on `ProductMessage`
   and an additive `attachments` request field on the message-send DTO, backed by
   ProductStore migration **020**.
5. Provider-layer injection for raster images (image content parts through all four
   protocol adapters) plus a typed degradation path, and text-like attachments injected
   as bounded, labelled, user-visible content.
6. A bounded TTL cleanup job for unreferenced and expired attachments.
7. The composer affordance, upload/error states, transcript rendering, and copy strings
   required to make the flow usable and honestly reported.

### 1.2 Non-goals (deliberately deferred, with the reason)

| Deferred | Why |
|---|---|
| Archives (`zip`, `tar`, `gz`, `7z`) | Design §8.3 refuses them outright in batch one. `sniff_mime` already recognises `PK\x03\x04` as `application/zip` (`apps/api/src/product/files.rs:747-748`), so refusing is a positive classification, not an unknown. |
| SVG and HTML attachments | They are active content. `rove_core::mime_type_is_active_content` refuses to preview them inline (`core/src/tool_result.rs:279-300`) and the existing artifact path tests assert it (`apps/api/src/product/artifacts.rs:1110-1131`). Accepting them would create a new active-content render path. |
| Inline PDF rendering | The design MIME allow-list includes `pdf`, but `application/pdf` is classified as active content by the same `mime_type_is_active_content` (`core/src/tool_result.rs:298`). Batch one stores and downloads PDFs; it never renders them inline. |
| Audio, video, Office/OOXML, executables | Not in the design's allow-list. |
| Tauri Desktop drag-and-drop | Section 8.4 of this plan. The desktop has no drag-and-drop handler today, so this is new trusted surface, not existing surface to reuse. |
| Local content extraction (PDF text, OCR, image thumbnails) | Would add a parser/rendering dependency and a decompression-bomb class the plan cannot bound with the existing stack. |
| Copying attachments into the workspace | Mutating the user's project is not an attachment feature. It would also collide with git status, ignore rules, and the coding tools' observed-mutation ledger. |
| Attachment editing, versioning, renaming, deduplication by content | Each needs its own contract and quota story. |
| Attachment bytes in `trace.jsonl`, `report.json`, SSE events, or the evidence export | Would violate the "secrets must not appear in trace/report/API responses" invariant and bloat the canonical event stream. |
| Cross-session or workspace-level attachment library | Design §8.2 fixes session scope. |

## 2. Verified current state

Each row is something the plan depends on, read in this worktree.

| Fact | Evidence |
|---|---|
| `ProductMessage` has no attachment field; its wire shape is `id`, `product_session_id`, `content`, `requested_delivery`, `actual_delivery`, `status`, `seq`, `run_id`, `successor_run_id`, `created_at`, `applied_at`, `queue_order`, `reason` | `apps/api/src/product/contracts.rs:326-354` |
| The message-send DTO is `#[serde(deny_unknown_fields)]` with only `content` + `idempotency_key` | `apps/api/src/product/contracts.rs:356-362` |
| Message content is capped at 32 KiB in the shared runtime contract | `runtime/src/conversation.rs:16`, enforced at `runtime/src/conversation.rs:206` |
| Send persists through the shared message lifecycle; the adapter builds `CreateProductMessageRequest { content, idempotency_key }` | `apps/api/src/product/message_adapter.rs:57-79` |
| `create_message` trims content, derives its request digest from content alone (`stable_hash(content)`), and treats a repeated `idempotency_key` with different content as a typed conflict | `apps/api/src/product/store/repository.rs:2484-2536` (digest at `:2494`, conflict at `:2510-2515`) |
| Archived sessions refuse new messages; `error`/`needs_attention` sessions accept them as `abandoned` | `apps/api/src/product/store/repository.rs:2564-2573` |
| ProductStore schema version is **17**; migrations are applied in order, each inside an `IMMEDIATE` transaction guarded by `migration_is_applied`, and each records `(version, name, applied_at)` in `product_schema_migrations` | `apps/api/src/product/store/schema.rs:9`, `:619-684`, `:705-715`, `:745-771` |
| A schema newer than this build is refused, not ignored | `apps/api/src/product/store/schema.rs:686-703` |
| Migration 016 added `last_outcome` columns; migration 017 added nullable `queue_order` | `apps/api/src/product/store/schema.rs:339-353`, `:1184-1240` |
| `join_safe` rejects absolute paths, `..`, and secret-shaped path components, and re-checks the canonicalised path against the root | `apps/api/src/product/files.rs:635-670` |
| `is_secret_filename` matches `.env*`, `*.pem`, `*.key`, `id_rsa*`, `id_ed25519*`, `.npmrc`, `.netrc`, `.dockercfg`, `credentials.json`, `*.p12`, `*.pfx` | `apps/api/src/product/files.rs:672-685` |
| Existing byte caps: text content 1 MiB, download range 64 MiB, image 16 MiB, image dimension 16 384, image pixels 40 000 000, sniff header 1 MiB | `apps/api/src/product/files.rs:27-32` |
| `guess_mime` maps extensions (including `pdf`, `zip`, `svg`, `html`) and `sniff_mime` recognises PNG/JPEG/GIF/WEBP/PDF/ZIP/WASM signatures | `apps/api/src/product/files.rs:702-754` |
| `validate_raster_image` refuses a raster larger than 16 MiB before it parses dimensions, and parses PNG/GIF/JPEG/WEBP headers with a pixel cap | `apps/api/src/product/files.rs:756-800` |
| `serve_file` sets `Content-Type` from the local sniff (falling back to the extension guess), a server-built `Content-Disposition`, `X-Content-Type-Options: nosniff`, `Cache-Control: private, no-store`, and CSP `default-src 'none'; sandbox`; ranged reads return 206 | `apps/api/src/product/files.rs:401-466` |
| `content_disposition` maps every non-`[A-Za-z0-9._-]` character to `_` and truncates to 160 characters — a Unicode attachment name is mangled, never echoed | `apps/api/src/product/files.rs:888-912` |
| Preview sessions already model an API-controlled TTL/concurrency/timeout budget: 8 sessions, 30-minute TTL, 8 MiB per resource, 16 concurrent resource requests, 10-second request timeout | `apps/api/src/product/preview.rs:50-59` |
| Preview returns 429 for its own limit and 504 for its request timeout | `apps/api/src/product/preview.rs:257`, `:368`, `:390-403` |
| The artifact surface already classifies availability as `available`/`cleaned`/`invalid`/`too_large` and preview kind as `text`/`raster_image`/`download_only`/`unavailable` | `apps/api/src/product/artifacts.rs:57-73` |
| The artifact surface refuses inline preview for active content and offers only a raster whitelist (`image/png`, `image/jpeg`, `image/gif`, `image/webp`) for inline rendering | `apps/api/src/product/artifacts.rs:627-641` |
| Durable Tool Artifacts derive their id locally from the content hash, hash while writing, enforce quotas before and during the write, remove a partial payload on rejection, write metadata via temp+rename, and keep an append-only ledger that records rejections | `runtime/src/state/tool_artifacts.rs:17-28`, quotas at `:41-51` |
| `ToolArtifactRef` carries a locally generated `storage_ref`, the verified `sha256`, `Sensitivity`, `trust`, and `validation` | `core/src/tool_result.rs:222-255` |
| A text-level secret-pattern detector exists and already knows `Authorization: Bearer`, `sk-ant-`, `sk-proj-`, `sk-`, `ghp_`, `gho_`, `github_pat_`, `xoxb-`, `xoxp-`, `AIza`, `Bearer`, and `password=`/`token=`/`api_key=`/`secret=` shapes | `apps/api/src/product/export.rs:677-715` |
| `ApiError` has 400/404/409/429/500/503/504 constructors but no 413 and no 410 | `apps/api/src/lib.rs:4915-4945`, `:4979-5018` |
| `From<ProductStoreError> for ApiError` matches `ProductErrorCode` exhaustively, so a new code is a compile error until it is mapped | `apps/api/src/lib.rs:5021-5060` |
| `ProductErrorCode` has preview and review codes but no attachment code | `apps/api/src/product/contracts.rs:1939-1976` |
| Only the M1 migration route raises the body limit (to 64 MiB); the rest of the router keeps the axum default | `apps/api/src/lib.rs:249-251`, `:325` |
| The API already has bearer auth, a CORS allowlist, and a per-process 60-second rate-limit window | `apps/api/src/security.rs:69-93`, `:95-129`; `docs/runtime/implementation-guide.md:1768-1787` |
| `models::Message.content` is a `String`; the only rich content type is `ContentBlock::{Text, RichReference}`, and `Message` carries `content_blocks: Vec<ContentBlock>` | `models/src/protocol.rs:23-37`, `:472-491` |
| `ProviderCapabilities` declares only `streaming`, `tool_calls`, `parallel_tool_calls` | `models/src/traits.rs:8-14` |
| **No provider protocol adapter reads `content_blocks`.** All four project `message.content` as plain text only | `models/src/provider/protocols/anthropic.rs:123-162`, `ollama.rs:122-137`, `openai_completions.rs:152-170`, `openai_responses.rs:212-241`; a search for `content_blocks` under `models/src/provider` returns nothing |
| `ProviderCapabilities` is built with a struct literal in 9 places (including the fake provider and the wire protocol) | `models/src/fake.rs:164-170`, `models/src/provider/wire.rs:70`, `models/src/provider/client.rs:211`, `models/src/provider/external_adapter.rs:249`, the four protocols, `tests/e2e.rs:345` |
| An idle follow-up run's goal text is `claim.control.content` | `apps/api/src/lib.rs:1602-1618` |
| An in-flight steer becomes `SteerMessage::for_message(id, control.content)` | `apps/api/src/lib.rs:3371-3373` |
| The run loop turns a drained steer into `Message::user(steer.content)` — one plain-text user message | `runtime/src/engine/run_loop.rs:580-593` (`Message::user` at `:588`) |
| The run recorder turns the run goal into `Message::user(goal)` | `runtime/src/state/artifacts.rs:87-97` |
| Every workspace read is resolved by `resolve_workspace_read_path`; escaping the workspace is `EnvironmentError` "workspace path is outside the execution boundary" | `runtime/src/environment.rs:64`, `:2225-2230`, used at `:564`, `:684`, `:707`, `:730` |
| The web composer's only "attachment" today is a text paste chip capped at 10 chips / 256 KiB, folded into the outgoing string by `composeMessageWithAttachments` | `apps/web/chat/composer-paste.ts:10-12`, `:23-38`, `:50-62`; call site `apps/web/chat/Composer.tsx:184-192`, paste handler `:208-227` |
| The web send path keys its one-shot idempotency map on session id + trimmed content and posts `{ content, idempotency_key }` | `apps/web/state/use-session-continuity.ts:916-947` |
| `ProductMessage`/`CreateProductMessageRequest` in the web client mirror the server DTO, and the latter is an alias of the control DTO | `apps/web/product/product-api-types.ts:1074-1094` |
| Composer copy goes through the dictionary layer; paste strings already exist in both locales | `apps/web/copy/zh-CN.ts:254-258` |
| The desktop has path-validation helpers (`canonical_existing_directory` at `:420`, `canonical_existing_path` at `:433`, `is_safe_path` at `:451`, `show_in_folder` at `:464`) but **no drag-and-drop handler** | `apps/desktop/src/commands.rs:420-480`; a search for a drop event handler under `apps/desktop/src` returns nothing |
| The state layout document owns the `<data_root>` contract (`product.sqlite`, `workspaces/<storage_key>/…`, `.migration/`) | `STATE_LAYOUT_AND_MIGRATION.md:12-31`, `:35-49` |
| Legacy migration classifies files relative to a **workspace** root, with an explicit `unknown` class | `apps/bootstrap/src/state_migration.rs:95`, `:614-650` |

## 3. Storage layout

### 3.1 Root

```
<data_root>/
  product.sqlite                     # unchanged (STATE_LAYOUT_AND_MIGRATION.md:37)
  attachments/
    <product_session_id>/            # ULID, server-generated session id
      <attachment_id>                # ULID, server-generated; payload bytes only, no extension
      <attachment_id>.part           # transient; only during a write
  workspaces/<storage_key>/…         # unchanged
```

- The root is `<data_root>/attachments`, i.e. a new sibling of `product.sqlite`, resolved
  from the same pinned user data root that already produces `config.product_sqlite_path()`
  (`apps/api/src/lib.rs:577`). It is deliberately **not** inside any workspace.
- Per-session directories mirror the session ownership boundary already used everywhere
  else: a request must resolve its session through ProductStore before any path is built,
  exactly like `resolve_workspace_file` resolves the workspace first
  (`apps/api/src/product/files.rs:309-322`).
- Directory creation is `create_dir_all` + canonicalise on first write, unix `0700`,
  matching the existing `ensure_workspace_layout` discipline
  (`STATE_LAYOUT_AND_MIGRATION.md:140`). Payload and `.part` files are created unix `0600`
  (no existing precedent is claimed for the file mode; it is proposed here).

### 3.2 Server-generated identity

- `attachment_id` is a fresh ULID generated server-side (`ulid` is already a direct
  dependency of `rove-api`, `apps/api/Cargo.toml:35`). It is never derived from a
  client name, path, or content.
- The stored filename is exactly the `attachment_id`. **No extension is appended**, so no
  code path can ever recover a MIME type from a filesystem name. `guess_mime` exists
  (`apps/api/src/product/files.rs:702`) and must stay out of this path.
- The client's original filename is never a path and never stored as one. It is bounded
  and carried only as display metadata on the message reference (section 6.3).

### 3.3 Durable record versus filesystem

SQLite is authoritative for metadata; the filesystem holds only payload bytes. This is the
same split as the durable Tool Artifact store (`runtime/src/state/tool_artifacts.rs:8-15`),
and it is deliberate that the bytes are not content-addressed here.

| In SQLite (migration 020) | On the filesystem |
|---|---|
| `attachment_id` (primary key) | the bytes at `<data_root>/attachments/<session_id>/<attachment_id>` |
| `product_session_id` (FK, `ON DELETE CASCADE`) | — |
| `content_type` (locally verified) | — |
| `byte_length`, `sha256` (computed while writing) | — |
| `status` (`staged` / `referenced` / `expired`) | — |
| `created_at`, `referenced_at`, `expires_at` | — |
| `scan_flags` (JSON array of warning codes, secret-free) | — |

Why not content-addressed like Tool Artifacts: content addressing would
(a) make two sessions share bytes and therefore share a deletion, which breaks the
per-session quota and the per-session trust boundary; and (b) let a client probe whether a
given byte sequence is already stored in another session by comparing returned ids.
The store is *referenced by an opaque id* like `ToolArtifactRef` (`core/src/tool_result.rs:222-255`),
but not *deduplicated* like it.

### 3.4 Write protocol and how a missing or dangling file is reported

Write order is **file first, row second**, in the spirit of the artifact store's
partial-payload discipline (`runtime/src/state/tool_artifacts.rs:25-26`, `:263-266`):

1. Stream the request body into `<attachment_id>.part` while hashing and counting bytes.
   Abort as soon as the count exceeds the cap; remove the partial file.
2. `sync_all()` the partial file, then `rename` it to `<attachment_id>`. A rename is the
   only step that publishes bytes.
3. Insert the ProductStore row in one transaction. Any failure here removes the file.

The consequence is that the only possible crash residue is a payload with no row. That is
an **orphan**, not a dangling reference, and it is safe by construction: nothing can
resolve it because no row names it and the id is unguessable. The cleanup job (section 9)
reclaims orphans by directory scan.

The reverse case — a row whose payload is gone — can only appear through operator or
filesystem loss. It is reported, never hidden:

- A read verifies that the payload exists, is a regular file, and matches the recorded
  `byte_length` and `sha256`. Any mismatch resolves to the typed availability
  `missing` or `corrupt`.
- `GET .../attachments/{aid}` answers `410 Gone` with code
  `product_attachment_unavailable` for `missing`/`corrupt`, and `409` with code
  `product_attachment_conflict` for `expired` (an attachment the cleanup job deliberately
  reclaimed — a different situation from one that was lost, and section 4.2 keeps the two
  distinguishable). It never streams a partial or truncated body as success.
- A message reference to an unavailable attachment is still rendered in the transcript,
  marked unavailable, and the reference is **not** passed to the provider. The turn
  degrades visibly instead of silently sending less than the user attached.

This mirrors the existing `ProductArtifactAvailability` vocabulary
(`apps/api/src/product/artifacts.rs:57-64`) rather than inventing a second one. The plan
proposes the attachment-specific spelling `available | missing | corrupt | expired`
because `cleaned` and `too_large` have artifact-specific meanings; a maintainer may prefer
reusing the artifact enum verbatim (open question Q6).

### 3.5 Interaction with the state-dir layout

- `attachments/` is a new entry in the `<data_root>` contract, so
  `STATE_LAYOUT_AND_MIGRATION.md` §1 and §2 need one row and one tree line in the same
  implementation PR. That document is current-state, and `AGENTS.md` §10 requires
  current-state documents to move with the contract.
- `rove state migrate` is unaffected: `classify_relative_path` classifies paths **relative
  to a workspace root** (`apps/bootstrap/src/state_migration.rs:614-650`), and
  `<data_root>/attachments` is outside every workspace. The global ProductStore already
  has its own single-file migration handling, and the attachment directory is its
  companion, not a per-workspace artifact.
- `attachments/` must not be moved into a workspace by `--prune-legacy`: the prune path
  only touches legacy sources under a workspace (STATE_LAYOUT_AND_MIGRATION.md:128-131).
  The plan asks for one negative test asserting that no `attachments/` path is ever
  classified or pruned (verification gate V9).
- `ROVE_DATA_ROOT` moves the root, so attachments follow the data root and never the
  workspace. The plan requires a test asserting that changing the workspace does not
  relocate or shadow a session's attachments.

## 4. Endpoints

All three surfaces sit behind the existing bearer-auth/CORS/rate-limit middleware
(`apps/api/src/lib.rs:344-346`). None of them is reachable without it. The routes are
registered in the same `OpenApiRouter` as the other product routes
(`apps/api/src/lib.rs:252-325`), so OpenAPI and the API contract tests move with them.

Request and response bodies are **new DTOs in `apps/api/src/product/contracts.rs`**,
following the existing naming convention (`ProductAttachment*`).

### 4.1 `POST /product/sessions/{session_id}/attachments`

Why raw body rather than multipart: axum is declared with no features in the workspace
manifest (`Cargo.toml:37`, `axum = "0.8"`), `axum::extract::Multipart` is gated behind the
`multipart` feature, and `multer` — the crate that feature pulls in — appears neither in
`Cargo.lock` nor in axum's resolved dependency list. Enabling it would be a dependency and
lockfile change. A raw-body endpoint needs no new dependency and has one fewer parser to
bound, which the threat model prefers. The trade-off is that the display name travels in
the query string rather than in a part.

Request:

```
POST /product/sessions/{session_id}/attachments?name=<percent-encoded display name>
Content-Type: application/octet-stream
Content-Length: <n>
Authorization: Bearer <token>

<raw bytes>
```

- `name` is optional, at most 255 bytes decoded, and rejected (400) if it contains a
  control character, a path separator, or a `..` component. It is a **display hint only**.
  The `Content-Type` request header is read as a *claim* for validation comparison and is
  never stored, echoed, or used to build a path. Any other header is ignored.
- Body cap: enforced by reading with `axum::body::to_bytes(body, cap + 1)` rather than
  relying on the extractor default. The route also installs
  `DefaultBodyLimit::max(cap + 1)` so an oversized `Content-Length` fails before the read.
  The pattern of a route-scoped `DefaultBodyLimit` already exists
  (`apps/api/src/lib.rs:249-251`).

Response `201 Created`:

```json
{
  "attachment_id": "01J8Z0M6Q3W9F2V7B4K1N5T8XR",
  "product_session_id": "01J8Z0M6Q3W9F2V7B4K1N5T8XA",
  "content_type": "image/png",
  "size": 20480,
  "sha256": "9f2c…",
  "name": "screenshot.png",
  "status": "staged",
  "expires_at": "2026-09-27T12:00:00Z",
  "warnings": []
}
```

Status codes and typed codes:

| Status | Code | When |
|---|---|---|
| 201 | — | stored |
| 400 | `product_attachment_invalid_input` | empty body; name invalid; extension not allow-listed; sniffed type not allow-listed; extension/sniff mismatch; refused archive signature |
| 400 | `product_attachment_secret_warning` **is not an error** | see 4.1.1 — a secret-shaped name or detected content secret is a 201 with a warning |
| 404 | `product_not_found` | session id unknown (existing code, `apps/api/src/product/contracts.rs:1981`) |
| 409 | `product_attachment_conflict` | session is `archived` |
| 409 | `product_attachment_quota` | per-session count or total-byte quota would be exceeded |
| 413 | `product_attachment_too_large` | body exceeds the cap |
| 429 | `product_attachment_busy` | the upload concurrency semaphore is exhausted |
| 429 | — | the existing per-process rate limit (`apps/api/src/security.rs:95-129`) |
| 503 | `product_store_unavailable` | ProductStore closed or the data root is not writable |
| 504 | `product_attachment_timeout` | the upload exceeded its write deadline |

413, 410, and the busy/timeout codes require small `ApiError` additions: today the type has
`bad_request`, `bad_request_with_code`, `not_found_with_code`, `conflict_with_code`,
`too_many_requests_with_code`, `service_unavailable_with_code`, `gateway_timeout_with_code`,
and `internal` (`apps/api/src/lib.rs:4915-5018`) but no `payload_too_large_with_code` and
no `gone_with_code`. Adding those two is part of PR-1, not a workaround.

#### 4.1.1 Concurrency, quotas, and timeouts

| Bound | Value | Rationale |
|---|---|---|
| Single attachment bytes | 20 MiB for `pdf`/`txt`/`md`; 16 MiB for raster | 20 MiB is the design ceiling (§8.2). Raster is narrowed to the existing 16 MiB image cap (`apps/api/src/product/files.rs:29`, enforced at `:760-764`), because a 16–20 MiB raster would be accepted and then be unrenderable, which is worse than a clear 413. See Q1. |
| Concurrent uploads, process-wide | 4 | Below the preview precedent's 16 (`apps/api/src/product/preview.rs:57`) because each upload holds up to 20 MiB of memory and a file handle. |
| Upload deadline | 60 s from first byte to rename | Generous for a local loopback client at 20 MiB; bounded so a stalled client cannot pin a slot. |
| Referenced attachments per session | 32 | Well above a realistic message; enforced as a store invariant, not a UI hint. |
| Total referenced bytes per session | 128 MiB | 4× the per-session count at the largest single size. |
| Unreferenced (staged) attachments per session | 32 | A staged attachment consumes the same count quota, so a client cannot stage unbounded data. |
| Total staged bytes per session | 64 MiB | Separate from the referenced total so a staged attachment cannot starve a referenced one. |
| Unreferenced TTL | 24 h | See section 9. |
| Referenced retention | none (until the session is deleted) | A referenced attachment is part of the session's durable content. |

Quota checks run inside the same transaction that publishes the row, so two concurrent
uploads cannot both pass the last slot. The bytes are already on disk at that point; a
quota refusal removes the file it just wrote.

### 4.2 `GET /product/sessions/{session_id}/attachments/{attachment_id}`

Request: optional `Range` header, only for the attachment disposition.

Response headers are fixed by the server, reusing the audited `serve_file` discipline
(`apps/api/src/product/files.rs:401-466`):

- `Content-Type`: the verified `content_type` from the durable row. Not the sniff result
  of the moment, and never a client header. (If the row and the bytes ever disagree, the
  `sha256`/length check in 3.4 has already failed the request.)
- `Content-Disposition`: `inline` for `image/png|jpeg|gif|webp`, otherwise `attachment`,
  with a filename derived from the sanitized display name through the existing
  `content_disposition` rules (`apps/api/src/product/files.rs:888-912`): ASCII-only,
  `[A-Za-z0-9._-]`, truncated to 160 characters, empty becomes `download`. No
  `filename*=UTF-8''` variant in batch one, which means a CJK display name downloads as
  `____.png`. That is a cosmetic regression against a nicer header and the honest option
  while `Content-Disposition` encoding is not a tested surface here (Q4).
- `X-Content-Type-Options: nosniff`, `Cache-Control: private, no-store`, CSP
  `default-src 'none'; sandbox`, `Accept-Ranges: bytes`.
- 206 with `Content-Range` for a satisfied `Range`; 400 for an unsatisfiable range,
  matching `parse_range` (`apps/api/src/product/files.rs:577-583`).
- 409 `product_attachment_conflict` for an attachment whose row exists but whose payload
  was expired by TTL — an expired attachment is a visible conflict, not a 404 that hides
  why it vanished.
- 410 `product_attachment_unavailable` for `missing`/`corrupt`.
- 404 `product_attachment_not_found` when the id is unknown or belongs to another session.
  Cross-session probing must be indistinguishable from "does not exist", so the
  session/attachment pair is always checked together.

No `GET .../attachments` list endpoint in batch one: the transcript already carries the
references, and a list endpoint would be a second authority for the same fact.

### 4.3 Cleanup job

There is no public route. Cleanup is an in-process periodic task owned by the API state
alongside the existing background sweeps (`apps/api/src/lib.rs:612-614` spawns the product
ownership recovery task), registered at startup and stopped by the shutdown token.

Job contract:

- Period: 15 minutes, jittered; first run one minute after startup.
- Per run, bound the work: at most 256 candidate rows and at most 256 orphan directory
  entries per session, and at most 64 sessions per run. Overflow is deferred to the next
  run rather than unbounded.
- Each deletion is a store transaction that flips `status` to `expired` and clears
  `expires_at`, followed by best-effort file removal. A file that cannot be removed leaves
  an expired row, which the next run retries; the recovery is the row, not the file.
- Every deletion writes one bounded, secret-free `tracing::info!` line (attachment id,
  session id, reason, bytes). It does not write a canonical event: attachments are not part
  of the run lifecycle, and inventing an event would breach the "one lifecycle contract"
  invariant (AGENTS §4).

## 5. Validation

Validation happens in this order, and the first failure wins, so the error a client sees is
the earliest, cheapest, most specific reason.

1. **Session and authorization.** Resolve the session through ProductStore; `404` for
   unknown. Archived sessions `409`. This is the existing pattern
   (`apps/api/src/product/store/repository.rs:2570-2572`).
2. **Body size.** Length-checked before reading where possible, then enforced during the
   read (section 4.1.1).
3. **Display name.** Bound, control-character, and path-separator checks. Failure is 400.
4. **Extension allow-list.** The client's `name` extension, lowercased, must be one of
   `png`, `jpg`, `jpeg`, `webp`, `gif`, `pdf`, `txt`, `md`. A missing or unknown extension
   is 400. This is the first of the two required checks per design §8.2.
5. **Magic-byte sniffing.** `sniff_mime` (`apps/api/src/product/files.rs:736-754`) already
   recognises PNG, JPEG, GIF, WEBP, PDF, ZIP, and WASM from the leading bytes, and
   `validate_raster_image` (`:756-800`) additionally parses raster dimensions and applies
   the 40 000 000-pixel cap.
   **No new crate is needed.** A hand-rolled sniffer for exactly these formats already
   exists in the workspace, is already exercised by the file API, and the image crate is
   not required because the plan does not decode pixels. This is the honest position: if a
   later batch wants true raster decoding or PDF structure inspection, that is a new
   dependency with its own review, not a silent addition here.
6. **MIME allow-list and mismatch.** `png`/`jpeg`/`webp`/`gif` must sniff as their own
   raster type and pass `validate_raster_image`; `pdf` must sniff `%PDF-`; `txt`/`md` have
   no signature and are accepted only when the bytes are valid UTF-8 without a NUL byte —
   the same test the file API already applies (`apps/api/src/product/files.rs:353-356`).
   Any extension/sniff disagreement, and any sniffed `application/zip` or
   `application/wasm` (both recognised, both refused), is 400
   `product_attachment_invalid_input`. Archives are refused as a positive classification,
   per design §8.3.
7. **Raster pixel bound.** Reuse the 40 000 000-pixel and 16 384-dimension caps
   (`apps/api/src/product/files.rs:30-31`). This is the only decompression-bomb control
   the plan can offer, and it protects the raster path only.
8. **Secret check — warning, not silent rejection.** Two independent checks, both
   warn-only, per design §8.3:
   - the display-name check reuses `is_secret_filename` (`apps/api/src/product/files.rs:672-685`);
   - the content check reuses the text-level pattern list in `redact_secret_patterns`
     (`apps/api/src/product/export.rs:677-715`) as a **detector**. That function currently
     lives in `export.rs` and redacts; PR-2 extracts the pattern table into a shared,
     bounds-tested predicate so a second caller cannot drift from it.
   The response carries `warnings: ["secret_shaped_name"]` and/or
   `warnings: ["possible_secret_content"]`. The bytes are stored **unmodified**: the user
   asked to send their own file, and silently rewriting their data would be worse than
   warning them. The warning must be rendered in the composer before send (section 8).
   This is the design's explicit "user warning rather than silent rejection".
9. **Record publication.** Only after 1–8 pass is the row written, in the same transaction
   as the quota check.

`content_type` in the stored row is always the **locally verified** type, never the
client's claim. A mismatch between the client's `Content-Type` claim and the verified type
is itself a warning (`content_type_claim_mismatch`), recorded in `scan_flags`.

## 6. Message contract

### 6.1 Migration number claimed: **020**

- The current schema version is **17** (`apps/api/src/product/store/schema.rs:9`), which is
  R4's migration 017 (`docs/design/2026-09-26-runtime-contract-alignment-design.md:46`,
  `docs/runtime/implementation-guide.md:1574-1578`). R3's `last_outcome` is 016 and R4's
  `queue_order` is 017; both are `Implemented` in the design's §0.1 table
  (`:45-46`).
- The design reserves **018** for R5 (product catalogue SSE, §0.1 `:47`) and **019** for R7
  (FTS5, §7.2 `:652`). Neither is on this baseline: the version is still 17.
- The design's §8.2 already anticipates this: "迁移 019/020 视 R7 是否同批". Because R7 is
  `Proposed` and takes 019, **R8 claims 020**.
- Why claiming 020 is safe even if R5 or R7 never ships: `product_schema_is_current`
  compares `MAX(version)` against `CURRENT_SCHEMA_VERSION`
  (`apps/api/src/product/store/schema.rs:686-703`), and each `apply_migration_NNN` is a
  no-op when `migration_is_applied` is already true (`:705-715`). A gap in the sequence is
  therefore harmless; reusing 018 or 019 would not be, because a database that already
  applied a different migration under that number would skip the new one.
- R8 must **not** claim 018 or 019, and R5/R7 must not later claim 020.

### 6.2 Additive `ProductMessage` field

```rust
/// Additive. Absent/empty for every message written before migration 020, and
/// for every message a client sends without attachments.
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub attachments: Vec<ProductMessageAttachmentRef>,
```

Placement: appended after `reason` in `ProductMessage`
(`apps/api/src/product/contracts.rs:326-354`), with the same
`#[serde(default, skip_serializing_if = …)]` discipline as `queue_order` (`:350-351`), so a
pre-migration payload parses unchanged and an attachment-free payload serialises
byte-identically to today. That last property is the one the contract test must pin.

Wire shape (server-verified values only — see 6.4):

```json
{
  "attachments": [
    {
      "attachment_id": "01J8Z0M6Q3W9F2V7B4K1N5T8XR",
      "content_type": "image/png",
      "size": 20480,
      "sha256": "9f2c…",
      "name": "screenshot.png",
      "availability": "available"
    }
  ]
}
```

`attachment_id`, `content_type`, `size` are required by the design's §8.2 reference shape.
`sha256` and `availability` are additions this plan proposes:

- `sha256` lets the injection path (section 7) verify that the bytes it reads are the bytes
  that were referenced, which closes the read-after-reference TOCTOU window.
- `availability` is what makes "a referenced attachment whose file is gone" visible in the
  transcript instead of an absent array entry (section 3.4). Without it, the UI has no way
  to distinguish "the user attached nothing here" from "the payload is gone".

Neither addition weakens the §8.2 skeleton; both are inside its stated intent. A maintainer
who wants the skeleton literally minimal may drop `sha256` (Q5) at the cost of the TOCTOU
check.

### 6.3 Request field

```rust
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductMessageAttachmentRequest {
    pub attachment_id: ProductAttachmentId,
    /// Display-only. Bounded, sanitized for storage, sanitized again on render.
    #[serde(default)]
    pub name: Option<String>,
}

// added to CreateProductMessageRequest (contracts.rs:356-362)
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub attachments: Vec<ProductMessageAttachmentRequest>,
```

The request carries **only** the id and the display name. `content_type`, `size`, and
`sha256` are not accepted from the client; the server resolves them from the durable row
and overwrites any client claim. Sending them is a 400 because
`CreateProductMessageRequest` keeps `deny_unknown_fields` (`:357`), which is the correct
fail-closed behaviour: an honest client sends nothing extra, and a client that thinks it
can assert a MIME type is refused rather than silently corrected.

Idempotency: today the digest is `stable_hash(content)` and a repeated key with different
content is a conflict (`apps/api/src/product/store/repository.rs:2494`, `:2510-2515`).
Attachments must join that digest, in a canonical order (sorted by `attachment_id`), so a
retry with the same key and the same attachment set replays, and a retry with the same key
and a *different* attachment set is the same typed conflict as different text. This is a
required behaviour change in `create_message`, not an optional refinement.

### 6.4 Backward-compatibility rule

Stated as a rule rather than an assumption:

| Combination | Behaviour | Why |
|---|---|---|
| Old row, new server | `attachments` is `[]` (missing → `default`) | additive field with a default; migration 020 does not backfill |
| Old client, new server | Sends no `attachments`; the message is created with an empty set | `#[serde(default)]` on the request field |
| New client, old server | The unknown field is refused with 400 `product_invalid_input` by the existing `deny_unknown_fields` path (`apps/api/src/product/routes.rs:60-67`) | fail-closed. Degrading a new client to silently dropping attachments would send the user's message without their file, which is worse than an error |
| Old server, new row | Not possible: migration 020 refuses a database whose recorded version is newer than the build (`apps/api/src/product/store/schema.rs:696-701`) | the schema check is the guard, so a downgrade cannot silently misread attachment state |
| Referenced attachment expired or missing | The reference remains in the message with `availability: "expired"`/`"missing"`, and the injection path sends a labelled placeholder | the user's message text and history stay intact and honest |

`docs/runtime/implementation-guide.md` §15 (ProductStore schema) and §5 (route table) must
be updated in the same PR as migration 020, and `docs/runtime/subsystems.md` §"API And
Security" must gain the attachment boundary. No `docs/runtime/implementation-status.md`
or `acceptance-matrix.md` line is claimed until the four integration cases in section 11
pass.

## 7. Runtime injection — the design's open point, answered

Design §8.2 leaves "how a referenced attachment becomes model input" open and requires the
plan to answer it. This section is the answer.

### 7.1 The seam

Today a product message becomes a model input as **one plain-text user message**, through
exactly two paths:

- idle follow-up: `claim.control.content` → `CreateJobRequest.message`
  (`apps/api/src/lib.rs:1602-1618`) → `RunArtifactRecorder::new` → `Message::user(goal)`
  (`runtime/src/state/artifacts.rs:87-97`);
- in-flight steer: `control.content` → `SteerMessage::for_message`/`with_id`
  (`apps/api/src/lib.rs:3371-3373`) → `Message::user(steer.content)`
  (`runtime/src/engine/run_loop.rs:580-593`).

Both currently construct a text-only `Message`. The plan introduces one new runtime-neutral
value that both paths carry:

```rust
// runtime/src/conversation.rs — extension of the existing command vocabulary
pub struct MessageAttachment {
    pub attachment_id: String,
    pub content_type: String,
    pub byte_length: u64,
    pub sha256: String,
    pub display_name: Option<String>,
    /// Bytes are resolved through this port, never read by the runtime from a
    /// path it was handed. `None` when the attachment is unavailable.
    pub source: Option<Arc<dyn AttachmentSource>>,
}

#[async_trait]
pub trait AttachmentSource: Send + Sync {
    /// Bounded read of the verified payload. The implementation owns path
    /// resolution and the workspace-independent root.
    async fn read_bounded(&self, max_bytes: u64) -> Result<Vec<u8>, AttachmentError>;
}
```

- `SendMessageCommand` (`runtime/src/conversation.rs:61`) gains
  `attachments: Vec<MessageAttachment>`.
- `SteerMessage` (`runtime/src/engine/control.rs:41-45`) gains the same field.
- `RunArtifactRecorder::new` (`runtime/src/state/artifacts.rs:69-97`) gains an
  attachment parameter and builds the initial user message through one shared constructor.

**The runtime never receives an absolute filesystem path.** The API passes an
`AttachmentSource` it implements over the API-controlled root; the runtime receives bytes
or a typed error. This is the only shape that respects the workspace-boundary invariant
(AGENTS §4) while the bytes live outside the workspace.

Why the design's "text-like attachments by path reference for tools" cannot be implemented
literally, with evidence: every workspace read in the runtime resolves through
`resolve_workspace_read_path` and fails with "workspace path is outside the execution
boundary" (`runtime/src/environment.rs:64`, `:2225-2230`, used at `:564`, `:684`, `:707`,
`:730`). Handing the `fs`/read tool a path under `<data_root>/attachments/...` would be
refused by the environment, and widening the environment root to include the attachment
root would dissolve the workspace boundary for every other tool. So the plan keeps the
design's *intent* — text-like content reaches the model through a bounded, explicit,
tool-shaped route rather than by being pasted into a prompt — and changes the *mechanism*
from "a path" to "an id-addressed bounded read". This is the single most important
deviation from §8.2 and it is called out again in section 12.

### 7.2 Images: provider content parts through the adapters

The plan adds one variant to the model protocol:

```rust
// models/src/protocol.rs — ContentBlock (currently Text | RichReference, :25-37)
Image {
    /// Locally verified MIME type.
    mime_type: String,
    /// Base64 payload, bounded by MAX_CONTENT_BYTES (:13).
    data: String,
}
```

and teaches each of the four protocol adapters to project it:

| Adapter | Native shape to emit | Evidence of where the projection happens today |
|---|---|---|
| `openai_completions` | `content: [{ "type": "text", … }, { "type": "image_url", "image_url": { "url": "data:<mime>;base64,…" } }]` | text-only projection at `models/src/provider/protocols/openai_completions.rs:152-170` |
| `openai_responses` | `input` items with `type: "input_image"` and a `data:` URL | text-only projection at `openai_responses.rs:212-241` |
| `anthropic` | `content` blocks with `{ "type": "image", "source": { "type": "base64", "media_type", "data" } }` | text-only projection at `anthropic.rs:123-162` |
| `ollama` | `images: [<base64>]` alongside the text `content` | text-only projection at `ollama.rs:122-137` |

This is real, non-trivial work in a shared boundary and it is why the plan puts it in its
own PR (PR-4). The important verified fact is that **`content_blocks` is currently dropped
on the floor by every adapter**: `Message` carries the field
(`models/src/protocol.rs:489-490`) and validates it (`:451-470`), and a search for
`content_blocks` under `models/src/provider` returns nothing. So the field is a
serialisation-only surface today, and "images already work through content parts" would be
false.

### 7.3 Capability negotiation and typed degradation

`ProviderCapabilities` currently declares only `streaming`, `tool_calls`,
`parallel_tool_calls` (`models/src/traits.rs:8-14`). The plan adds:

```rust
pub images: bool,
```

- Default `false`, so any provider that has not been taught to project images declares
  itself incapable and degrades (fail-closed), rather than emitting a payload the provider
  will reject.
- The four protocol adapters plus the fake provider set it explicitly; `ProviderCapabilities`
  is constructed with a struct literal in 9 places (`models/src/fake.rs:165`,
  `models/src/provider/wire.rs:70`, `models/src/provider/client.rs:211`,
  `models/src/provider/external_adapter.rs:249`, the four protocols, `tests/e2e.rs:345`),
  so the compiler enumerates the decision points for the reviewer.
- The existing validation shape is the precedent: `validate_tools` returns
  `ModelError::InvalidConfiguration` before dispatch when a capability is missing
  (`models/src/traits.rs:26-41`), and there is a test that a capability failure precedes
  transport dispatch (`models/src/provider/client.rs:251`). Images follow it with
  `validate_messages(&self, messages: &[Message]) -> Result<(), ModelError>`.

Degradation is **typed and visible**, never silent. When a message carries an image and the
selected client reports `images: false`:

1. The API resolves the capability before the run starts, from the same product model
   descriptor the session already uses.
2. The run proceeds with the text the user wrote. Each image is replaced by one bounded,
   labelled placeholder line in the user message:

   ```
   [attachment not sent: screenshot.png (image/png, 20.0 KiB) — the selected model does not accept image input]
   ```

   The placeholder contains the display name, the verified MIME type, and the size. It
   contains no absolute path, no bytes, and no provider credential.
3. The API surfaces the degradation to the client through the existing typed-degradation
   vocabulary rather than a new event: the message's attachment entry reports
   `availability: "available"` (the bytes are fine) plus a session-visible degradation
   reason, and the Web transcript renders "this image was not sent to the model". The plan
   proposes attaching the reason to the run's existing degradation reporting rather than
   adding a `StreamEvent` variant, because attachments are not a run lifecycle fact
   (AGENTS §4). PR-4 must confirm which existing surface is the correct carrier and say so
   in its implementation record; if none fits, the honest resolution is a new canonical
   event with the full five-part update ("event five-piece": producer, trace persistence,
   API SSE, OpenAPI, Web, contract test), which is why this is a PR-sized decision.

### 7.4 Text-like attachments

`txt` and `md` attachments are injected as bounded, labelled content, not as a path and not
as an opaque blob:

- Read through the same `AttachmentSource::read_bounded`, capped at the smaller of the
  attachment's own size and a new `MAX_INLINE_ATTACHMENT_TEXT_BYTES` (proposed 64 KiB).
- Delimited so the model can tell user text from file text, with the delimiter fixed and
  ASCII (the same reasoning as the paste-chip marker, which is deliberately "prompt content,
  not UI copy", `apps/web/chat/composer-paste.ts:45-49`):

  ```
  [attachment: notes.md (text/markdown, 12.3 KiB, sha256 9f2c…)]
  <<<ROVE-ATTACHMENT 01J8Z0M6Q3W9F2V7B4K1N5T8XR>>>
  …content…
  <<<END ROVE-ATTACHMENT 01J8Z0M6Q3W9F2V7B4K1N5T8XR>>>
  ```

- An attachment larger than the inline cap is not truncated silently: the placeholder says
  the content was not included and why. Truncating a document and hoping the model notices
  is exactly the "reported as success" failure the AGENTS security checklist forbids.
- Attachment content is **data**, never instructions: it arrives inside a delimited block
  in a user message, and nothing in it can grant a tool permission (AGENTS §4, "retrieved
  or generated text is not automatically a trusted instruction"). The plan adds a negative
  test asserting that an attachment whose content contains a tool-approval-shaped sentence
  or an `AGENTS.md`-shaped block changes no capability, approval, or permission state.

### 7.5 The bounded-and-user-visible invariant

Design §8.2 requires that content never enters a prompt unless it is bounded and
user-visible. Both halves are structural here, not aspirational:

- **Bounded** by four independent caps: the upload cap (section 4.1.1), the inline text cap
  (7.4), `MAX_CONTENT_BLOCKS`/`MAX_CONTENT_BYTES` in the model protocol
  (`models/src/protocol.rs:12-13`), and the existing runtime message cap
  (`runtime/src/conversation.rs:16`). The plan requires that attachment bytes never count
  against the 32 KiB message cap silently: exceeding the inline cap produces a labelled
  omission, not a truncated body.
- **User-visible** because every injected attachment appears in the transcript as a chip
  with name, verified type, size, hash prefix, and availability, and because the API stores
  the expected injected form (`scan_flags`) next to the record. A test asserts that the
  number of attachment references in a `ProductMessage` equals the number of attachment
  blocks the run's initial user message carries, so an injection path cannot add content
  the user never attached.

### 7.6 Exact `models/` types that change

| Type | File | Change |
|---|---|---|
| `ContentBlock` | `models/src/protocol.rs:25-37` | add `Image { mime_type, data }`; extend `validate` (`:51-70`) and `validate_content` (`:451-470`) with the image bounds |
| `ProviderCapabilities` | `models/src/traits.rs:8-14` | add `images: bool` (default `false`); add `validate_messages` |
| Four protocol adapters | `openai_completions.rs`, `openai_responses.rs`, `anthropic.rs`, `ollama.rs` | project image blocks; set `images: true` |
| `FakeModelClient` | `models/src/fake.rs:164-170` | declare image support and echo a deterministic back-reference so the offline gate can prove an image block reached the client without a network call |

`Message` itself (`models/src/protocol.rs:472-491`) needs **no** structural change: it
already carries `content_blocks`, which is what makes this an additive protocol change
rather than a rewrite. `Message.content` stays the human-readable text.

## 8. Web surface

### 8.1 Composer affordance

The composer already has the right shape for this, which is why the Web work is
substantial but not architectural. Today `attachments` is a local array of text chips
(`apps/web/chat/Composer.tsx:184-192`, `:208-227`) and the send call folds them into the
message string (`composeMessageWithAttachments`,
`apps/web/chat/composer-paste.ts:50-62`).

Changes:

| Module | Change |
|---|---|
| `apps/web/chat/composer-attachments.ts` (new) | Pure state machine for file attachments: add/remove, per-file and total caps mirroring the server, `attachment_id`-keyed status (`queued`/`uploading`/`uploaded`/`failed`), and the mapping from a server warning code to a copy key. Unit-tested without React, matching the existing pure-module convention (`composer-paste.ts`, `transcript-window.ts`). |
| `apps/web/chat/composer-paste.ts` | Keep the text-chip path unchanged. Add one function that produces the *request* attachment list, so the text path and the file path cannot drift. |
| `apps/web/chat/Composer.tsx` | File input (`multiple`, `accept` from the server allow-list), drop-target wiring **inside the webview only**, per-file progress and error rows, the secret warning, and removal before send. `onSend` keeps its `(message: string)` shape and gains a second argument carrying references, so the existing retry/idempotency handling above it is untouched. |
| `apps/web/state/composer-draft-store.ts` | The restore snapshot must capture attachment *references*, not bytes, so a restored draft re-validates against the server instead of replaying a stale upload. An expired reference restores as a visible "re-attach" row. |
| `apps/web/state/use-session-continuity.ts` | Add the attachment references to the send request (`:944-947`) and include them in the `requestKey` (`:916`) so two sends that differ only by attachment are not treated as one idempotent request. |
| `apps/web/product/product-client.ts` | Add `uploadAttachment(sessionId, file, onProgress)` (`:218` region) and extend `sendMessage` (`:970-996`). Upload uses `XMLHttpRequest` or `fetch` with a streamed body for progress; the existing client is `fetch`-based, so the plan states the choice and its reason in PR-5. |
| `apps/web/product/product-api-types.ts` | Extend `ProductMessage`/`ProductMessagesResponse` and the message request type (`:1074-1094`) with the attachment shapes, plus a runtime schema check in the same style as the existing `ProductApiSchemaError` validation. |

### 8.2 Upload progress and error states

- Progress: a determinate bar per file while uploading, a spinner while the server
  validates, and a terminal row for success/failure. Never a fake 100% while the server is
  still validating.
- Errors map one-to-one to the typed codes of section 4.1: too large, unsupported type,
  content mismatch, quota exceeded, busy (with retry), timeout (with retry), session
  archived, store unavailable. Each has its own copy key; no generic "upload failed".
- A `429`/`504` is retried once with a visible "retrying" state, because both are
  idempotent for a *new* upload (a retry creates a new `attachment_id`; the first attempt
  is either absent or becomes an orphan the cleanup job reclaims). A retry must never reuse
  an `attachment_id`, and the plan requires a test for that.
- The send button stays disabled while any attachment is `uploading`, so a message cannot
  be sent referencing an id the server has not yet confirmed.

### 8.3 Transcript rendering

- Raster attachments render as a bounded inline image through
  `GET .../attachments/{aid}` (the disposition is `inline`, so an `<img src>` works without
  a new endpoint).
- `pdf`, `txt`, `md` render as a file row with a download link. No inline PDF viewer, no
  HTML/SVG rendering (section 1.2).
- Every attachment shows name, verified type, size, and a short hash prefix, and shows its
  `availability` when it is not `available`.
- `apps/web/chat/Transcript.tsx` and the transcript entry builders (`chat/transcript-entries.ts`)
  change; the transcript projection stays a pure function over the message payload.

### 8.4 Copy strings

All new strings go into `apps/web/copy/zh-CN.ts` and `apps/web/copy/en-US.ts`, matching the
existing paste strings (`zh-CN.ts:254-258`). The plan requires both locales to be complete
in the same PR: `apps/web/copy/copy.test.ts:29-33` asserts that every locale exposes the
same key tree, so a missing key fails a gate rather than shipping an English string into the
Chinese UI.

### 8.5 Tauri Desktop drag-and-drop: deferred, and why

Verified: the desktop has path-validation helpers (`canonical_existing_path`, `is_safe_path`,
`show_in_folder`, `apps/desktop/src/commands.rs:433-462`) but **no** drag-and-drop handler.
There is therefore no existing trusted path to reuse, and design §8.3 flags the host-supplied
path as its own trust boundary. Adding one is a new Tauri command, a new capability entry,
and a new validation surface, which does not belong in the same PR as the API contract.

What a later batch must do, recorded here so it is not rediscovered:

1. A `stage_attachment(path)` command that re-validates the host path with the existing
   `is_safe_path`/`canonical_existing_path` discipline, refuses a non-regular file, reads
   bounded bytes, and posts them to the API itself.
2. The webview must never receive the absolute path. The command returns only the API
   response (id, type, size, hash).
3. Batch one cannot rely on Desktop DnD, so the file input must work in the Desktop shell
   too (a Tauri webview file input yields a browser `File`, not a host path, so it needs no
   new trust).

## 9. Cleanup and lifecycle

### 9.1 States and transitions

```
staged ──(first message referencing it)──▶ referenced ──(session deleted)──▶ (row + file removed)
   │                                            │
   │ TTL expired                                │ payload lost / hash mismatch
   ▼                                            ▼
expired (409 on read)                     missing | corrupt (410 on read)
```

- `staged`: uploaded, not yet named by any message. `expires_at = created_at + 24 h`.
- `referenced`: named by at least one durable message. `referenced_at` set on the first
  reference; `expires_at` cleared. A referenced attachment is never TTL-collected.
- `expired`: the payload was reclaimed. The row survives so the transcript can say *why*
  the attachment is gone instead of showing an empty slot.
- `missing`/`corrupt` are computed at read time from the file and hash, not stored states:
  a stored "missing" would be a second authority that can go stale.

### 9.2 TTL for unreferenced attachments

24 hours. Long enough that a user who uploads, gets distracted, and sends the next morning
still has the file; short enough that a client that uploads and abandons cannot accumulate
20 MiB objects indefinitely. The staged quota (32 files / 64 MiB per session) is the hard
bound; the TTL is the reclamation.

### 9.3 The cleanup-versus-reference race, and what makes a deletion safe

The race is real: a user uploads, the cleanup job decides the staged attachment is expired,
and the message referencing it commits in between. The plan makes the deletion safe by
ordering rather than by locking:

1. The cleanup job selects candidates in a transaction and marks them expired **with a
   compare-and-set on `status = 'staged'`**, so a row that became `referenced` between
   selection and commit fails the CAS and is skipped.
2. The message-send transaction resolves every referenced attachment and flips
   `staged → referenced` in the **same transaction** that inserts the message row. A
   reference to an already-`expired` row fails the send with 409
   `product_attachment_conflict` instead of creating a message whose file is being deleted.
3. Only after step 1 commits does the job remove files, and file removal is best-effort and
   idempotent. A failure leaves the expired row, and the next run retries.
4. `expired` is a terminal state. Nothing can resurrect an expired attachment, because
   resurrection would need a new upload, which mints a new id.

This gives three invariants a test can assert:

- **I1**: no message ever references an attachment whose payload the job deleted — because
  the CAS and the reference happen in serialised transactions.
- **I2**: a reference to an `expired`/`missing`/`corrupt` attachment is a typed, visible
  refusal or degradation, never a silent drop.
- **I3**: cleanup never removes a payload whose row is `referenced`, regardless of age.

What makes a deletion *safe* in one sentence: a deletion always follows a committed state
transition of the row that owns the file, and the row is the only thing that can name the
file, so the file can never be deleted while something still points at it.

### 9.4 Interaction with session deletion

`product_session_attachments.product_session_id` cascades on session delete
(`ON DELETE CASCADE`, matching `product_sessions`' existing cascade,
`apps/api/src/product/store/schema.rs:38-39`). The cascade removes rows; the payload
directory is removed by the same best-effort sweep, and a residue is reclaimed as orphans
by the next run. `DELETE /product/sessions/{id}` must therefore also remove
`<data_root>/attachments/<session_id>/`, and the plan requires a test for it (V8).

## 10. PR split

Each PR is independently verifiable, and each states its own gate. The sequence follows
design §11's "each item is its own project" and its note that R8 needs a threat model
first (design §11 table `:784`, risk register `:815`).

| PR | Scope | Gate (all must pass to merge) |
|---|---|---|
| PR-1 | Store schema, storage layout, and the upload/read endpoints with full validation. No message or model changes. OpenAPI and the state-layout document updated. | Migration 020 unit tests (fresh, upgrade from a v17 database, idempotent re-run, future-version refusal); upload/download integration cases; the full negative matrix of section 5 (traversal, symlink/hardlink, extension/sniff mismatch, archive refusal, secret warning, size cap, quota, busy, timeout); `cargo fmt`/`clippy -D warnings`/`test --workspace` |
| PR-2 | The shared secret-pattern predicate extracted from `export.rs`, plus the `warnings` contract and its copy. No new endpoint. | The extractor is behaviour-preserving: the existing `export.rs` tests still pass unchanged, plus a new test that the predicate is bounds-safe on adversarial input and that the attachment path stores bytes unmodified while reporting the warning |
| PR-3 | `attachments` on `ProductMessage` and the send DTO, idempotency digest change, reference-transition and cleanup job, TTL, cascade on session delete. | Contract tests: old payload parses, attachment-free payload serialises byte-identically, old client works, new field on an old server is refused, idempotency digest covers the attachment set, I1/I2/I3, session-delete cascade, cleanup bounded per run |
| PR-4 | Provider layer: `ContentBlock::Image`, `ProviderCapabilities::images`, the four adapter projections, fake-provider support, capability negotiation, typed degradation, text-attachment injection, the bounded/user-visible invariant test, and the workspace-boundary negative test. | Offline fake-provider run asserting an image block reached the client and that a non-image provider produced a labelled placeholder; a negative test that attachment content grants no capability; a test that no code path hands a raw attachment path to the environment. Still no network requirement |
| PR-5 | Web: composer affordance, upload UI, transcript rendering, copy in both locales, no Desktop DnD. | `pnpm test`, `pnpm typecheck`, `pnpm build`; new mocked e2e cases for upload progress, each error class, secret warning, degraded-image rendering, and the reference-in-transcript round trip |

Deliberately not in this sequence: Desktop DnD (section 8.5) and any content extraction.

## 11. Verification gates

Design §8.4 sets the acceptance line for the *design record* as "threat model + contract
tests + the four integration cases exist". This plan restates that as the implementation
line and adds the tests the design's own §8.3 checklist implies.

### 11.1 The four integration cases design §8.4 requires

| # | Case | Assertion |
|---|---|---|
| V1 | Upload | A valid PNG under the cap returns 201 with a server-generated id, the verified type, the exact size, and the sha256 of the bytes; the payload exists at the derived path; no response field contains an absolute path or the data root |
| V2 | Download | The same id returns the exact bytes with the verified `Content-Type`, `nosniff`, `no-store`, a server-fixed `Content-Disposition`, and CSP `default-src 'none'; sandbox`; a `Range` request yields 206 with a correct `Content-Range`; a client-supplied `Content-Type`/`Content-Disposition` is never echoed |
| V3 | Reference | Sending a message with the id persists the reference, returns it on the message payload, flips the row to `referenced`, clears the TTL, and makes the attachment inbound to the model (verified offline through the fake provider, including the non-image provider's labelled placeholder) |
| V4 | Cleanup | An unreferenced attachment past its TTL is expired and its payload removed; a referenced one is untouched regardless of age; a reference to an expired attachment is a typed, visible refusal; a cleanup-vs-reference interleaving leaves no message pointing at a deleted payload |

### 11.2 Unit and contract tests

| # | Test |
|---|---|
| V5 | Validation matrix — every extension/sniff combination in section 5, including `PK\x03\x04` and `\0asm` being refused as archives/executables, a raster over 16 MiB, a raster over the pixel cap, and a NUL-containing `txt` |
| V6 | Secret warning — a `.env`-shaped name and each pattern family from `apps/api/src/product/export.rs:677-715` produce the warning, the stored bytes are byte-identical to the upload, and no test fixture contains a real credential |
| V7 | Contract compatibility — old payload parse, attachment-free byte-identical serialise, old client send, new field on an old server → 400, idempotency digest includes the attachment set |
| V8 | Lifecycle — TTL, I1/I2/I3, session-delete cascade removing rows and the payload directory, orphan reclamation, and the bounded per-run work limit |
| V9 | Boundary — `join_safe`-level negatives are reused unchanged for the attachment path; no attachment path is ever classified or pruned by `rove state migrate`; changing the workspace never relocates an attachment |
| V10 | Injection — the reference count equals the injected block count; oversized text produces a labelled omission; attachment content grants no permission; no absolute path appears in a prompt, a trace event, a report, or an API response |

### 11.3 What can run offline

The repository requires local deterministic execution with no provider key and no network
(AGENTS §4, `docs/runtime/implementation-guide.md:1786-1787`). Stated honestly:

- **Runs offline, no key, no network (all of V1–V10):** everything above. V3's model-side
  assertion uses the fake provider (`models/src/fake.rs`), which the plan extends to
  declare `images: true` and to record the received message blocks deterministically. No
  attachment gate depends on a real provider.
- **Runs offline, browser:** the mocked Playwright cases in PR-5. They prove the client
  contract, not a real upload against a real API.
- **Opt-in, not run by this plan:** the real-provider image round trip (does Anthropic /
  OpenAI / Ollama actually accept the payload this build emits), and a browser case
  against a live API. Both need a credential or a local model server. Recording "the
  protocol adapter emits the documented shape" is not the same as "a real provider accepted
  it", and this plan claims only the former. This is the same split the repository already
  uses for provider smoke tests (`docs/runtime/provider-smoke.md`).

## 12. Open questions for the maintainer

Each has a recommendation and the cost of choosing wrong.

**Q1 — Should the raster cap be narrowed from 20 MiB to 16 MiB?**
§8.2 says one 20 MiB cap; the existing image path refuses to render above 16 MiB
(`apps/api/src/product/files.rs:29`, `:760-764`). Recommendation: 16 MiB for raster, 20 MiB
for documents. Cost of choosing wrong: if 20 MiB rasters must work, the image path needs a
new rendering story (resize on upload, or an out-of-band preview route), which is a
dependency and a second bounded pipeline; if 16 MiB is wrong, a user occasionally gets a
413 on a large screenshot instead of a broken preview.

**Q2 — Text attachments: inject bounded content, or add an id-addressed read tool?**
Recommendation: inject bounded content in batch one (section 7.4), because it needs no new
tool, is provably bounded, and is visible in the transcript. Cost of choosing wrong: if the
product wants "the model reads the file only when it decides to", the injection must become
a tool call, which is a new tool schema, a new approval surface, and a new round trip —
a much larger PR, and it interacts with the approval policy.

**Q3 — Which surface carries the image-degradation fact?**
Recommendation: resolve the capability before the run and carry the reason on the existing
degradation/`execution_degraded` reporting rather than a new `StreamEvent`. Cost of choosing
wrong: if no existing surface fits honestly, the correct answer is a new canonical event,
which triggers the full event five-piece update (producer, trace, SSE, OpenAPI, Web,
contract test) and makes PR-4 visibly larger. This must be decided before PR-4, not during.

**Q4 — Unicode filenames in `Content-Disposition`.**
Recommendation: accept the ASCII-only mangling for batch one and record it as a known
cosmetic limitation; the existing helper is ASCII-only
(`apps/api/src/product/files.rs:888-912`) and RFC 5987/6266 encoding is a header-injection
surface that deserves its own tested change. Cost of choosing wrong: users with non-ASCII
filenames get `____.png` on download, which looks like a bug; the alternative is a header
encoding change that must be fuzzed.

**Q5 — Keep `sha256` and `availability` on the message reference?**
Recommendation: keep both (6.2). `sha256` closes a read-after-reference TOCTOU window and
`availability` is what makes a vanished payload honest. Cost of choosing wrong: dropping
`sha256` re-opens a narrow integrity gap with no visible symptom; dropping `availability`
makes "the file is gone" indistinguishable from "nothing was attached", which is exactly
the silent-success failure the AGENTS checklist forbids.

**Q6 — A dedicated attachment availability enum, or reuse the artifact one?**
Recommendation: a dedicated `available | missing | corrupt | expired`
(section 3.4), because `cleaned` and `too_large` are artifact-specific and reusing them
would force attachment semantics into artifact words. Cost of choosing wrong: reusing the
artifact enum means either misnaming states or later changing a shipped enum; a dedicated
enum means one more small type on the Web client.

**Q7 — Does the cleanup job need its own configuration, or fixed constants?**
Recommendation: fixed constants for batch one (24 h TTL, 15 min period, bounded per-run
work), with the values named as `pub(crate) const` next to the existing preview constants
(`apps/api/src/product/preview.rs:50-59`) so they are greppable and testable. Cost of
choosing wrong: an operator with a full disk cannot tune the TTL without a rebuild; adding
settings later is additive and cheap.

**Q8 — Should an attachment be shareable across sessions in the same workspace?**
Recommendation: no, session scope only, per §8.2. Cost of choosing wrong: cross-session
sharing turns the per-session quota and the per-session trust boundary into a
workspace-level problem, and it makes "which session owns this byte" ambiguous at deletion
time.

## 13. Contradictions found between design §8 and the code at `31629d5`

Recorded because AGENTS §1 requires contradictions to be surfaced rather than silently
resolved, and because a reviewer of this plan needs them in one place. Each is either
resolved above or promoted to an open question.

1. **`pdf` is in §8.2's MIME allow-list, but `application/pdf` is classified as active
   content by the code** (`core/src/tool_result.rs:298`), and the existing artifact path
   has a test asserting PDF must never be offered for inline preview
   (`apps/api/src/product/artifacts.rs:1110-1131`). Resolution in this plan: accept and
   store PDFs, never render them inline (section 1.2, section 8.3).
2. **§8.2's "文本类默认以路径引用交给工具读取" cannot work as written**, because every
   runtime read is bounded by the workspace (`runtime/src/environment.rs:64`, `:2225-2230`)
   and the attachment root is deliberately outside every workspace. Resolution: an
   id-addressed bounded read through an `AttachmentSource` port (section 7.1, 7.4). This is
   the largest deviation from §8.2 and the one a reviewer should weigh first.
3. **§8.2's single 20 MiB cap conflicts with the existing 16 MiB image cap**
   (`apps/api/src/product/files.rs:29`, `:760-764`). Resolution: per-type caps (Q1).
4. **§8.2 implies image content parts are a reachable mechanism; today they are not.**
   `content_blocks` exists on `Message` (`models/src/protocol.rs:489-490`) but no provider
   protocol adapter reads it (section 2, section 7.2), and `ProviderCapabilities` has no
   image flag (`models/src/traits.rs:8-14`). The design's "provider 层注入设计" therefore
   covers four adapters and a capability field, not a wire-up.
5. **§0.1 assigns R8 to "存储/端点/消息合同/provider 层" and mentions the migration only in
   §8.2 as "019/020", while §7.2 has R7 on 019.** The version is still 17, so 018 (R5) and
   019 (R7) are unclaimed. Resolution: R8 claims **020** (section 6.1).
6. **§8.3 names the Tauri drag-and-drop boundary, but no drag-and-drop handler exists**
   (`apps/desktop/src/commands.rs` has only `show_in_folder` and its path validators at
   `:433-462`). It is new surface, not an existing boundary to review, and it is deferred
   (section 8.5).
7. **§12's acceptance matrix has no R8 row** (`docs/design/2026-09-26-runtime-contract-alignment-design.md:790-803`),
   although §11 lists R8 in PR-7+. Not fixed by this docs change because §8 is frozen; the
   plan proposes adding the row when PR-1 lands.
8. **§8.2's message-reference shape omits `sha256` and `availability`**, which the
   injection and lifecycle designs in this plan need. Proposed as additive (Q5), not as a
   redefinition of the frozen skeleton.
