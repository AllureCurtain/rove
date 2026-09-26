# Attachment And Image Upload Threat Model (R8)

> Status: **Proposed / Not Implemented**. This is a threat model and implementation-boundary
> record for a protocol that does not exist yet. No attachment code, route, table, or column
> exists in this build, so **no mitigation below is claimed as present for attachments**; the
> "existing mitigation" column names controls that already exist elsewhere and that the plan
> reuses.
> Date: 2026-09-26 (inventory refreshed 2026-09-27 against `main @ 31629d5`).
> Related:
>
> - Design contract: [runtime contract alignment design](../design/2026-09-26-runtime-contract-alignment-design.md)
>   §0 invariant table, §0.1 R8 row, §8.1–§8.4.
> - Implementation plan: [attachments implementation plan](../plans/2026-09-26-attachments-implementation-plan.md)
>   (same branch). Design §8.1 requires a plan **and** a threat model before implementation;
>   this document is the second half of that gate.
> - Format and rigour precedent: [P5b local HTML preview threat model](./2026-09-17-pi-desktop-workbench-p5b-threat-model.md).
> - Security checklist: `AGENTS.md` §11; review policy `CONTRIBUTING.md` §6 (this change is
>   security-sensitive and needs independent blind review, not self-review).

## 1. Threat object

The feature lets a user hand the product a file — a screenshot, a PDF, a note — and have it
become part of a conversation with a model.

The object of this model is therefore not "a file". It is **an untrusted byte sequence with
an untrusted name, stored on the operator's machine by an API the operator already trusts,
and then transmitted off that machine.**

That has three consequences, and they are the three axes of everything below:

1. The bytes are untrusted input to *storage*. They must be bounded, classified, and made
   impossible to confuse with a path or a type the server chose.
2. The bytes are untrusted input to *rendering*. The product origin already holds the
   product token; anything that executes there is a full product compromise. This is the
   same conclusion the P5b model reached for previewed HTML
   (`docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md:30`, `:36-37`),
   but the answer is different here because the accepted types are not executable and
   because PDF changes that answer if it is ever rendered inline (T15).
3. The bytes are untrusted input to *egress*. Attaching a file is the user's own act, so
   the threat is not unauthorized disclosure but **unnoticed** disclosure: a secret the user
   forgot was in the file, a location the camera embedded, or an image silently dropped
   because the selected model cannot read it.

Attachments are also a new *durable* surface: unlike a pasted chip held in browser memory
(`apps/web/chat/composer-paste.ts:6-8`), an uploaded file persists on disk and in SQLite
across restarts. A mistake here is not a bad page render; it is a file on the operator's
disk.

## 2. Assets and trust boundaries

| Asset | Where it is today | May an attachment path reach it? |
|---|---|---|
| Product API bearer token | `api.token_auth`; Desktop injects it into its own webview; the browser deployment keeps it server-side in the Next.js proxy (`docs/runtime/implementation-guide.md:1774`, `:1783-1784`) | **No.** An attachment is never served from an origin that holds the token, and never as executable content. |
| ProductStore | `<data_root>/product.sqlite` (`STATE_LAYOUT_AND_MIGRATION.md:19`, `apps/api/src/lib.rs:577`) | **No.** Attachment metadata is a new table in the same database; no attachment response exposes a row, a schema detail, or a path. |
| Other sessions' attachments | Same data root | **No.** Resolution always goes through session ownership (T18). |
| Workspace files | `<data_root>/workspaces/<storage_key>/…`, plus the user's actual project tree | **No.** The attachment root is deliberately outside every workspace; and no attachment path is ever handed to the execution environment (T1, T5 in §5). |
| Host files outside the data root | Operator's filesystem | **No.** The root is API-created and bounded by the resolved data root. |
| Runtime `StateStore`, traces, reports | Per-workspace | **No.** No attachment fact enters `trace.jsonl`, `task_state.json`, `report.json`, the SSE stream, or `/product/sessions/{id}/export`. |
| Provider credentials | Provider profiles store an env-var reference only | **No.** Attachments never travel near a credential. |
| The attachment bytes themselves | New: `<data_root>/attachments/<session_id>/<attachment_id>` | This is the new asset. It is readable by the API process and by anything that can read the data root, and it is transmitted to the selected provider. |

Trust boundaries crossed, in order:

1. **Browser → API.** Untrusted bytes, an untrusted name, and an untrusted `Content-Type`
   claim cross into the API process. The API is the only place that decides type, size, and
   identity.
2. **API → filesystem.** The API turns an id it generated into a path it owns. No client
   value participates in that path.
3. **Filesystem → browser.** Bytes travel back as a bounded, typed, server-headered
   response on the product origin.
4. **Runtime → provider.** Bytes (or a labelled placeholder) leave the machine inside a
   model request. This is the only boundary where the content becomes visible to a third
   party.
5. **Host → webview (Desktop only, deferred).** A drag-and-drop event would carry a host
   path across the Tauri IPC boundary. Deferred precisely because it is new trusted surface
   (T13).

Boundary conclusion, stated as the one non-negotiable rule this model adds to the P5b one:
**a client-supplied string may never become a filesystem path component, and a stored MIME
type may never be derived from a client-supplied string.** Everything else below is a
degree choice.

## 3. Threats

`Existing mitigation` cites the code that is already there and would be reused. `Plan
mitigation` cites the implementation plan by section. `Residual` states what is left after
the plan, and `Verification` names the test that would prove the control. Where a residual
is accepted rather than closed, §5 says why.

| # | Threat | Vector | Impact | Existing mitigation (code) | Plan-required mitigation | Residual risk | Verification |
|---|---|---|---|---|---|---|---|
| T1 | Path escape / traversal | A crafted `attachment_id`, session id, or file name containing `..`, an absolute path, a drive/UNC prefix, a NUL, or percent-encoded traversal is used to build a filesystem path | Read, overwrite, or delete files outside the attachment root | `join_safe` rejects absolute paths, `RootDir`/`Prefix`, `..`, and secret-shaped components, then re-checks the canonicalised path against the root (`apps/api/src/product/files.rs:635-670`); the directory listing applies the same filter (`:491-500`) | The path is `<root>/<session_id>/<attachment_id>` where **both components are server-generated ULIDs**; the client name never becomes a path component; the two components are validated as ULIDs before `join_safe` is called, so the same audited function is the second line of defence, not the first | An operator-created symlink inside the root (covered by T2) | Plan §11 V9: absolute id, `..`, `%2e%2e`, UNC prefix, drive letter, NUL, over-long id → 400/404 with no filesystem effect |
| T2 | Symlink tricks | A symlink or Windows reparse point is planted at the target id path, or `<id>.part` is raced, so a write or read is redirected outside the root | Arbitrary file write or read as the API process user | `join_safe` canonicalises and refuses an escape (`apps/api/src/product/files.rs:662-666`); `require_regular_file` refuses a non-regular file (`:604-612`); the preview service reuses this exact discipline by design (`apps/api/src/product/preview.rs:10-12`) | Write to `<id>.part` then `rename` (a rename replaces rather than follows), verify the published path with `symlink_metadata` as a regular non-reparse file, and re-check the canonicalised read path is inside the root | A local attacker who can already write inside `<data_root>/attachments` is outside the client trust boundary and already has the API user's filesystem rights | Plan §11 V9: plant a symlink/reparse point at the target and at `.part`; assert refusal and that the outside file is byte-unchanged |
| T3 | Hardlink tricks | A hardlink to a host file is pre-created at the target path, so the API serves or overwrites the linked file | Disclosure of a host file, or corruption of one | None specific to attachments. `is_secret_filename` is name-based only (`apps/api/src/product/files.rs:672-685`) | The root and per-session directory are API-created with restrictive permissions; the id is fresh, so the publish step uses create-new semantics and an existing target is a typed conflict, never an overwrite; the session directory is required to be a real directory | A local attacker with write access to the root (same boundary as T2) | Plan §11 V9: pre-create a hardlink at the target path and assert the upload fails without modifying the linked file |
| T4 | MIME spoofing / sniff disagreement | `payload.zip` renamed `photo.png`; a PNG header prepended to non-image bytes; a polyglot that is both a valid raster and a valid container; a client `Content-Type` that contradicts the bytes | The browser or the model layer is handed a type the server did not verify; a later consumer acts on the wrong type | Two independent classifiers already exist — extension via `guess_mime` (`apps/api/src/product/files.rs:702-734`) and bytes via `sniff_mime` (`:736-754`) — plus `validate_raster_image`, which parses real raster headers and rejects a bad one (`:756-800`); the file API already reports "file extension and raster image signature do not match" (`:336-340`); response `Content-Type` is built from the local sniff and never from a client header (`:437-441`), always with `X-Content-Type-Options: nosniff` (`:449`) | Require extension ∈ allow-list **and** sniff ∈ allow-list **and** the two agree; `txt`/`md` additionally require valid UTF-8 with no NUL byte (mirroring `:353-356`); store the verified type and never recompute it at read time; record a `content_type_claim_mismatch` warning when the client claims otherwise | A polyglot that sniffs as a raster is stored as that raster. Accepted: the only consumer of the image path is a bounded image projection, and the download path is header-fixed | Plan §11 V5: `PK\x03\x04`+`.png`, `%PDF-`+`.txt`, `\0asm`, JPEG magic+`.pdf`, PNG magic+`image/jpeg` claim, and the archive/executable refusals |
| T5 | Single-request size exhaustion | A 10 GiB body; a lie in `Content-Length`; a chunked body with no declared length | Memory or disk exhaustion; the API process is destabilised | The M1 migration route already raises the body limit for one route only (`apps/api/src/lib.rs:249-251`); ranged downloads are capped at 64 MiB (`apps/api/src/product/files.rs:28`, `:577-583`) | A route-scoped `DefaultBodyLimit::max(cap + 1)` **and** an explicit bounded `axum::body::to_bytes(body, cap + 1)` read, so the cap holds for a chunked body and `Content-Length` stays a hint; abort and unlink `.part` the moment the byte counter exceeds the cap | A client can still hold one concurrency slot until the upload deadline | Plan §11 V5: 413 at cap+1 for declared and undeclared lengths; assert no `.part` survives |
| T6 | Concurrency exhaustion | Many parallel uploads or downloads | File-descriptor/memory exhaustion; unrelated endpoints starve | The preview service models exactly this: `MAX_PREVIEW_CONCURRENT_REQUESTS = 16`, `MAX_PREVIEW_SESSIONS = 8`, `PREVIEW_REQUEST_TIMEOUT = 10s` (`apps/api/src/product/preview.rs:50-59`), returning 429 (`:368`) and 504 (`:403`) | A separate bounded upload semaphore and download semaphore; a 60 s upload deadline and a 10 s download deadline; typed 429 `product_attachment_busy` and 504 `product_attachment_timeout` | Limits are process-local. The API has a per-process rate limit, but it is inactive unless `api.rate_limit_per_minute` is configured (`apps/api/src/security.rs:96`) | Plan §11 V5: exhaust the semaphore, assert 429, assert other endpoints stay responsive; assert 504 on a stalled read |
| T7 | Quota exhaustion | Many small uploads in one session; a session reused indefinitely | Disk fill; the session becomes unservable | The store already enforces a per-session count cap as a store invariant, not a UI hint (`MAX_PENDING_STEERS_PER_SESSION`, `apps/api/src/product/store/repository.rs:2549-2554`) | Staged and referenced count/byte quotas per session (plan §4.1.1), checked inside the transaction that publishes the row; bounded per-run cleanup work | Per-session caps do **not** bound the whole data root: N sessions × 128 MiB is the real ceiling. Accepted for a local single-user API; recorded as a known gap in §5 | Plan §11 V8: quota boundary at N and N+1; two concurrent uploads racing the last slot, exactly one wins |
| T8 | Archives and decompression bombs | A `.zip`/`.gz`/`.7z`/`.rar`/`.tar` is uploaded under an allowed extension, with nesting designed to explode if anything ever extracts it | Storage of a bomb now; an expansion bomb in any future extraction feature | `sniff_mime` positively recognises `PK\x03\x04` as `application/zip` and `\0asm` as WASM (`apps/api/src/product/files.rs:747-750`); `guess_mime` maps `zip` (`:730`); `is_raster_mime` is already a whitelist (`:695-700`) | Refuse archives and executables outright in batch one, as a **positive classification** rather than an unknown (design §8.3); add no extraction code | None in this batch, because no extraction path exists. The risk returns the moment a future batch adds extraction, at which point that batch must carry its own bounded expansion budget | Plan §11 V5: `PK\x03\x04`, `\x1f\x8b`, `7z\xbc\xaf\x27\x1c`, `Rar!`, `ustar`, `\0asm` with allowed extensions → 400; plus a check that no extraction API is called |
| T9 | Raster decoding bomb | A tiny PNG/JPEG/GIF/WEBP declaring enormous dimensions | Memory exhaustion when the bytes are decoded by a consumer | `MAX_IMAGE_DIMENSION = 16_384`, `MAX_IMAGE_PIXELS = 40_000_000`, `MAX_IMAGE_BYTES = 16 MiB`, and header parsing that refuses a zero dimension or an unparsable JPEG/WebP header (`apps/api/src/product/files.rs:29-32`, `:796-800`) | Reuse `validate_raster_image` at upload; the API never decodes pixels; the image payload sent to a provider is bounded by `MAX_CONTENT_BYTES` (`models/src/protocol.rs:13`) and by the upload cap; PDF is download-only, so no PDF parser exists to bomb | A provider may decode an image the API never decoded; rove's only control there is the pre-decode dimension/pixel cap. Accepted (§5) | Plan §11 V5: one dimension above the cap; width×height above the pixel cap; a truncated header for each of the four raster types |
| T10 | Secret scanning: warning versus rejection | The user attaches `.env`, `id_rsa`, `credentials.json`, or a file whose text contains an API key | A credential is persisted durably and transmitted to a provider | `is_secret_filename` blocks secret-shaped **path components** for workspace reads (`apps/api/src/product/files.rs:648`, `:672-685`) — a hard refusal there, because that path means "read a file for me". A text-level pattern detector already exists for the export path and covers `Authorization: Bearer`, `sk-ant-`, `sk-proj-`, `sk-`, `ghp_`, `gho_`, `github_pat_`, `xoxb-`, `xoxp-`, `AIza`, `Bearer`, and `password=`/`token=`/`api_key=`/`secret=` (`apps/api/src/product/export.rs:677-715`) | Warn, do not reject, and do not rewrite: reuse the name check and an extracted, bounds-tested version of the text pattern predicate (plan §5 item 8); store bytes unmodified; surface an unmissable composer confirmation before send; record the warning code on the row | A user who ignores the warning sends the secret, and it is persisted on disk and transmitted. Accepted deliberately: design §8.3 asks for a warning, and rewriting the user's file would be a data-integrity hazard. The residual is transferred to user judgement, which is why the notice must be blocking-styled rather than a dismissible toast | Plan §11 V6: `.env`/`.pem`/`id_rsa` names and one content pattern per family produce the warning; stored bytes are byte-identical to the upload; no rejection is returned; fixtures contain no real credential |
| T11 | Cleanup race (reference versus TTL) | A message referencing an attachment commits while the cleanup job is reclaiming it; or a read races the unlink | A durable message points at a deleted payload; a run proceeds with content the user attached and the model never sees | No attachment cleanup exists. The general precedent is CAS-plus-typed-conflict (`apps/api/src/product/store/repository.rs:2510-2515`, `transition_control` at `:2985`) | Plan §9.3: cleanup marks candidates expired with a **compare-and-set on `status = 'staged'`**; the reference transition happens in the **same transaction** as the message insert; a reference to an `expired` row is a typed 409, never a message with a dead reference; `expired` is terminal; file removal happens only after the row transition commits, and is idempotent | An in-flight `GET` can lose the race with file removal and receive 410 instead of bytes. Accepted and visible: the status is typed and the transcript marks the reference expired | Plan §11 V4 plus a deterministic interleaving test that pauses between the row CAS and the unlink |
| T12 | API responses leaking absolute paths or host state | A JSON field, an error message that stringifies an OS error, or a header embedding `<data_root>`, the per-session directory, or a host path | Host reconnaissance; a username in a screenshot; a path useful for a chained attack | Product-route responses carry workspace-relative paths and `safe_name`s, not absolute paths (`apps/api/src/product/files.rs:86-101`, `apps/api/src/product/artifacts.rs:75-92`); the export path redacts home-style prefixes and absolute paths and counts the redactions (`apps/api/src/product/export.rs:646-675`, asserted at `tests/api.rs:1128-1130`) | Attachments are addressed by id only; no attachment response, log line, or event contains the root, the session directory, or a payload path; attachment errors are mapped to typed codes so an OS error string never becomes the response message | A pre-existing property of the API is that some `internal_error` bodies stringify OS errors; the export-time redaction is not applied to live responses. Accepted, with the plan requiring typed attachment errors to avoid adding a new instance | Plan §11 V1/V2 plus a scan of every attachment response body and every log line emitted across a full upload/download/reference/cleanup cycle for the data root and the session directory name |
| T13 | Tauri Desktop drag-and-drop trust boundary | A drop event delivers a host path to the webview and the page posts it, or a page requests an arbitrary path | The client reads or forwards any file on the host, bypassing the user's actual intent | Path validators already exist (`canonical_existing_path` at `apps/desktop/src/commands.rs:433`, `is_safe_path` at `:451`, consumed by `show_in_folder` at `:464`) but there is **no drag-and-drop handler** in this build | Deferred entirely (plan §8.5). Batch one uses a webview file input, which yields a browser `File` rather than a host path, so no new trust is introduced. Any later batch must add a `stage_attachment(path)` command that re-validates with the existing helpers and never returns the path to the webview | No risk in batch one. The live risk is a future implementer who uses the common Tauri/Electron idiom of reading a path off the drop event and posting it unvalidated | A Desktop-shell test asserting that no attachment command accepts a host path in batch one; later, traversal/outside-workspace/non-regular-file negatives on the new command |
| T14 | Provider-side egress of attachment bytes | The user attaches a file; the run sends it (or its text) to a remote provider | The file leaves the machine and may be retained remotely; a secret inside it leaves with it | The provider boundary already carries prompt text off-machine. `Sensitivity` and `ArtifactTrust::Untrusted` exist as classifications (`core/src/tool_result.rs:166-183`) but govern tool artifacts, not user messages | The composer and the transcript state, per attachment, that it will be sent to the selected provider; an unavailable or expired attachment is never sent; a provider that cannot accept images receives a labelled placeholder, so bytes are neither silently sent nor silently dropped; no attachment byte enters trace, report, SSE, or export | **The first-party trust decision.** A local-first product that sends user files to a third-party model is an inherent egress; the plan's answer is visibility plus per-provider capability, not prevention. Choosing a local provider keeps bytes on the machine, and that is stated in the copy rather than enforced. Accepted (§5) | Plan §11 V10: a fake-provider test asserting exactly which bytes reached the client; a test asserting no attachment byte appears in `trace.jsonl`, `report.json`, the SSE stream, or the session export |
| T15 | Inline rendering of active content and `Content-Type` confusion | An SVG/HTML/PDF — or a file that sniffs as one — is served inline and executes, or is served with a type that makes the browser treat it as a document | Product-token theft and arbitrary product actions — the P5b A1/A2 threats, which that model concluded could only be answered by origin isolation (`docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md:36-37`) | `serve_file` always sets `nosniff`, `Cache-Control: private, no-store`, and CSP `default-src 'none'; sandbox` (`apps/api/src/product/files.rs:449-454`); the artifact path refuses inline preview for active content and offers only a four-type raster whitelist (`apps/api/src/product/artifacts.rs:627-641`, with the explicit assertion at `:1117`) | `inline` disposition is granted **only** for `image/png`, `image/jpeg`, `image/gif`, `image/webp`, decided from the stored verified type; `image/svg+xml` and `text/html` are refused at upload and cannot be stored at all; PDF is stored but only ever `attachment`; CSP and `nosniff` are reused unchanged | Attachments are served from the **product origin**, unlike the P5b preview which required an isolated origin. Accepted only because the accepted types are non-executable raster, and PDF is download-only. If a future batch accepts SVG/HTML or inlines PDF, origin isolation from the P5b model becomes mandatory again | Plan §11 V5/V2: assert the disposition per type; assert SVG/HTML are refused at upload; assert a stored raster can never be served as `text/html`; assert the headers on every response |
| T16 | Filename and Unicode tricks | An RTL override, a homoglyph, a control character, a 300-character name, a name containing `/` or `\`, a Windows device name (`CON`, `NUL`), or a trailing dot/space | A deceptive UI row, a broken download, or — if a name ever reached the filesystem — a path or device-name attack | `content_disposition` maps every character outside `[A-Za-z0-9._-]` to `_` and truncates to 160 characters (`apps/api/src/product/files.rs:888-912`), so a name is mangled rather than echoed; `is_secret_filename` lowercases before matching (`:673`) | The name never reaches the filesystem (the stored file is the ULID); the name is bounded to 255 bytes; control characters and path separators are refused at upload; device-name-shaped names are refused; the name is sanitized again on render in the Web client, so a stored name cannot become stored XSS | A homoglyph or RTL name can still deceive a reader inside the transcript. Accepted: it is the same class as any user-supplied string, and the UI shows the verified type and size beside it | Plan §11 V5/V6: `..`, `/`, `\`, NUL, `\u202e`, an over-cap 4-byte emoji name, `CON`, trailing dot/space → 400; a valid CJK name is accepted, renders correctly, and downloads as ASCII |
| T17 | Metadata leakage (EXIF/GPS) | A photo carries GPS coordinates, a device serial, or an original timestamp | The user's location or device identity leaves the machine with the image | None. No metadata-stripping code exists in this repository | **Not mitigated.** The plan requires the composer to state that the original file, including embedded metadata, will be sent; it does not strip | Full. Accepted explicitly (§5): stripping needs an image-codec dependency and a re-encode, which is itself a new attack surface and a quality loss, and the user chose the file | Plan §11 V6: a test asserting the stored bytes are byte-identical to the upload — which is also the honest proof that nothing is stripped or rewritten — plus a copy test that the metadata notice exists in both locales |
| T18 | Cross-session probing (IDOR) | Session A's id paired with session B's attachment id; or guessing a well-formed id | Reading another session's attachment | Every existing file and artifact route resolves the owning workspace or session before touching a path (`apps/api/src/product/files.rs:309-322`; the artifact manifest is session-scoped, `apps/api/src/product/artifacts.rs:124-139`) | The `(session_id, attachment_id)` pair is always resolved through ProductStore first; a wrong-session id answers 404 with a body indistinguishable from "unknown", never 403, so it is not an oracle | Ids are ULIDs, so they are guessable in principle (timestamp plus randomness). Accepted: resolution is ownership-checked, so guessing gains nothing | Plan §11 V1/V2: cross-session pairs → 404; a well-formed unknown id → 404 with an identical body |
| T19 | Read-after-reference TOCTOU | The bytes change between the message reference and the model read, or a payload is replaced in place | The model receives content the user did not attach; a hash-mismatched payload is treated as valid | The durable artifact store hashes while writing and records the digest (`runtime/src/state/tool_artifacts.rs:20-21`), and `ToolArtifactRef` carries `sha256` and `byte_length` (`core/src/tool_result.rs:230-232`) | The attachment row carries `sha256` and `byte_length`; the read path verifies both before returning bytes and fails as `corrupt` otherwise; the message reference carries `sha256` so the injection path verifies again; payloads are never modified after publication (no in-place writes, no rename-over) | An attacker with local write access to the root can replace a payload with different bytes of the same length; the hash check catches it as `corrupt`. Accepted, with the hash check as the control | Plan §11 V10: replace one payload byte and assert the read is `corrupt`; assert the model path refuses rather than sending mismatched bytes |
| T20 | Attachment content as instructions (prompt injection) | An attached file contains text shaped like a system instruction, a tool-approval grant, or an `AGENTS.md` workspace-instruction block | The model treats file content as authority and acts on it | The repository's stated invariant is that retrieved or generated text is not automatically a trusted instruction (`AGENTS.md` §4), and the agent-material injection path already bounds and labels injected content (`runtime/src/agents/activation.rs:38` for the budget, `:209-213` for the labelled push) | Attachment content arrives inside a fixed ASCII delimited block in a user message (plan §7.4), and a negative test asserts that no attachment content can change a capability, an approval, or a permission | A prompt-injection *influence* on model behaviour cannot be removed by delimiting; the model may still act on the text. Accepted: the boundary that matters — a file cannot grant permission — is enforced by the tool registry and the approval path, not by the delimiter | Plan §11 V10: an attachment containing an approval-granting sentence and an `AGENTS.md`-shaped block changes no capability snapshot and no approval state |
| T21 | Disk exhaustion by orphan files | Uploads that crash between the file write and the row insert; or a client that uploads and never references the file | The data root fills | None. No attachment reclamation exists | File-first/row-second ordering makes a crash produce only orphans, never a dangling reference (plan §3.4); the cleanup job scans and reclaims orphans; staged quotas plus the TTL bound growth between runs | Orphans are reclaimed only on the next run (≤15 min plus jitter), and each run is bounded, so a determined client can stay slightly ahead of reclamation within the staged quota. Accepted: the staged quota is the hard bound | Plan §11 V8: orphan reclamation, bounded per-run work, staged-quota refusal |
| T22 | Log, trace, or evidence leakage of names, paths, or content | A log line or trace event records the display name, the payload path, or the bytes | A sensitive filename or host path reaches logs, trace, report, or an exported evidence bundle | The preview token is deliberately kept out of `Debug` and out of logs, and the P5b model makes that a gate (`apps/api/src/product/preview.rs:65-67`; `docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md:77`, `:96`) | Attachment logging carries the ids, the verified type, the size, the hash prefix, and the reason — never the display name, never content, never a path. No attachment fact becomes a canonical event, so nothing enters `trace.jsonl`, `report.json`, the SSE stream, or the export | The display name is user-supplied text and does appear in API responses by design, so it lives in client memory and in any screenshot. It does not enter durable runtime evidence | Plan §11 V10: scan all log, trace, report, and export output for the display name and the root path across V1–V4 |

## 4. Where the controls live, and why

Candidate landing spots for attachment storage and validation: `apps/api` (alongside the
existing file, artifact, and preview services), `runtime` (alongside the Tool Artifact
store), or `apps/desktop` (as a host-side staging service).

**Chosen: `apps/api`**, for the same reasons P5b chose it for preview, plus one specific to
attachments:

- The reusable controls already live there and are already tested: `join_safe`,
  `is_secret_filename`, `require_regular_file`, `sniff_mime`, `guess_mime`,
  `validate_raster_image`, `serve_file`, and `read_bounded_file_content`
  (`apps/api/src/product/files.rs:309-322`, `:401-466`, `:604-612`, `:635-685`, `:702-756`).
  T1, T2, T4, T9, T12, and T15's mitigations are therefore **reuse**, and reuse is the point:
  a second implementation is where the two would drift.
- The API owns the user data root (`apps/api/src/lib.rs:577`), so it can create and bound
  `<data_root>/attachments` without asking any other component for a path.
- The bytes must be injected back into a run through an API-implemented port precisely
  because the runtime's read boundary is the workspace
  (`runtime/src/environment.rs:64`, `:2225-2230`). If the runtime owned the payload root,
  either the boundary would have to widen or the runtime would need a second root concept.
- Desktop and browser deployments then behave identically, avoiding a "Desktop safe, browser
  unsafe" split.

**Cost:** the API process holds the payloads, so a compromise of the API process reaches
them. That is already true of `product.sqlite`, and the plan's answer is content
classification plus hash verification, not process isolation.

Deliberately **not** chosen:

- **`runtime/` as the payload authority.** It would require widening or duplicating the
  workspace read boundary (T1/§5), and durable Tool Artifacts are keyed to a *run*, whereas
  an attachment outlives and precedes any run.
- **A standalone staging process.** A new IPC channel plus a second path validator is more
  attack surface, not less — the same conclusion P5b reached
  (`docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md:56-58`).
- **`apps/desktop` as the authority.** Desktop DnD is deferred (T13), and a host-side
  authority would make the browser deployment a second-class, differently-behaved path.

## 5. Explicitly not mitigated, and why that is accepted

1. **Embedded metadata (EXIF/GPS/device identity), T17.** Not stripped. Stripping requires an
   image-codec dependency and a re-encode, which adds a decoder the API currently does not
   have (the API parses headers only, `apps/api/src/product/files.rs:756-800`) and a
   quality/data-loss decision that is not an attachment feature. The user chose the file.
   Mitigation is a visible notice, and the test asserts byte-identity so the absence of
   stripping can never be mistaken for an oversight.
2. **Provider-side retention and training use, T14.** Not preventable from inside rove.
   Once bytes are in a request to a remote provider, their fate is the provider's. The
   mitigation is per-attachment visibility that the file will be sent, plus a placeholder
   path for providers that cannot accept images. A local provider is the only way to keep
   bytes on the machine, and the plan surfaces that as a choice rather than enforcing it.
3. **Prompt-injection influence on model behaviour, T20.** Delimiting and labelling remove
   the *authority* claim, not the *influence*. The invariant that a file cannot grant a
   permission is enforced by the tool registry and approval path; a model that is persuaded
   to request a destructive tool still needs the user to approve it.
4. **A local attacker who can write inside `<data_root>/attachments`.** Outside the client
   trust boundary. Such an attacker already has the API user's filesystem rights, and no
   in-process control can change that. Hash verification (T19) makes silent payload
   substitution detectable, which is the honest limit.
5. **Per-session quotas do not bound the whole data root (T7).** 32 sessions × 128 MiB is
   the real ceiling, and nothing prunes an abandoned session's referenced attachments except
   deleting the session. Accepted for a local single-user API. A global byte budget with an
   eviction policy is a future change, and it needs a policy decision (evict oldest
   referenced attachment — which silently changes a durable message — is not obviously
   better than filling the disk).
6. **No malware or content-based scanning.** The allow-list is a type allow-list, not a
   safety scanner. A `.pdf` or `.png` can still contain a malicious payload for whatever
   eventually opens it. Accepted: rove does not execute attachments, and adding a scanner is
   a dependency and an update channel, not a validation rule.
7. **No multi-user identity or per-attachment authorization.** The API has no multi-user
   identity model, no browser session, and no distributed rate limiting
   (`docs/runtime/implementation-guide.md:1779-1784`). Attachment access is protected by the
   same bearer token as everything else, and the per-session ownership check (T18) is
   authorization *within* that trust. Accepted as the existing deployment scope, not as a
   new gap.
8. **Attachments are served from the product origin (T15).** Unlike P5b's preview, there is
   no isolated origin. Accepted **only** because the inline-capable types are the four
   non-executable raster types and PDF is download-only. This acceptance is conditional: the
   moment SVG, HTML, or inline PDF is wanted, the P5b origin-isolation requirement applies
   again and this section must be rewritten rather than extended.
9. **The 32 KiB message cap interaction (plan §7.5).** `MAX_MESSAGE_BYTES` is 32 KiB
   (`runtime/src/conversation.rs:16`). An inlined text attachment can therefore push the
   combined prompt content past what a small model will accept, and the plan deliberately
   does not raise that cap: raising it would change the shared message contract for all
   producers, not just attachments. The plan requires an oversized attachment to become a
   labelled omission instead of a silent truncation or a rejected turn.
10. **No `.part`-file race protection beyond atomic rename (T2/T3).** The publish step is an
    atomic rename with create-new semantics; there is no directory-level lock. Accepted
    because the ids are fresh and unguessable, so a race requires local write access to the
    root — case 4 again.

## 6. Gates that must be satisfied before implementation

These are the conditions a reviewer should require before PR-1 of the plan is authorized.
They restate design §8.4's acceptance line for the *design record* as conditions on the
*code*.

1. The plan and this threat model are both approved, and the plan explicitly records the two
   deliberate deviations from §8.2 — the id-addressed bounded read instead of a path handed
   to a tool, and per-type size caps instead of one 20 MiB cap (plan §13).
2. The path rule is enforced structurally: no client-supplied string can become a path
   component, and no stored MIME type is derived from a client string (T1, T4). A test must
   fail if a client name reaches the filesystem.
3. `join_safe`, `is_secret_filename`, `sniff_mime`, `validate_raster_image`, and `serve_file`
   are reused rather than reimplemented, with negative cases for absolute paths, `..`,
   symlinks, hardlinks, secret names, and every extension/sniff disagreement (T1–T4, T9).
4. Every bound is present and tested: 20 MiB/16 MiB per attachment, upload and download
   concurrency, upload and download deadlines, staged and referenced count/byte quotas, and
   the bounded per-run cleanup work (T5–T7, T21).
5. Archives are refused as a positive classification, and no extraction code exists (T8).
6. The secret decision is warn-only and visible, the stored bytes are byte-identical to the
   upload, and the warning is unmissable in the composer rather than a dismissible toast
   (T10, T17).
7. The cleanup/reference ordering is implemented as CAS plus a single reference transaction,
   with interleaving tests for the three invariants in plan §9.3 (T11).
8. `inline` disposition is impossible for anything outside the four raster types, and SVG and
   HTML cannot be stored at all (T15).
9. No attachment fact enters `trace.jsonl`, `report.json`, the SSE stream, the export, or a
   log line containing a display name or a path (T12, T22).
10. The image capability is negotiated, and a provider that cannot accept images produces a
    visible, labelled degradation rather than a silent drop (T14, and plan §7.3).
11. Desktop drag-and-drop is either absent (batch one) or implemented through a validated
    host-side staging command that never returns a path to the webview (T13).
12. `docs/runtime/implementation-guide.md` §5/§15/§19, `docs/runtime/subsystems.md`
    §"API And Security", and `STATE_LAYOUT_AND_MIGRATION.md` §1/§2 are updated in the same
    PR, and no `implementation-status.md`/`acceptance-matrix.md` line is claimed before the
    four integration cases pass.

## 7. Known limitations of this model

- This is a **static** analysis of code read at `main @ 31629d5`. No attack payload has been
  executed against a running API, because the feature does not exist. The verification
  column names the test that would prove each control; until those tests exist and fail
  appropriately when the control is removed, every mitigation here is a design intention.
- No fuzzing of the sniffer, the name validator, the range parser, or the `Content-Disposition`
  builder is proposed. The P5b model has the same gap and says so
  (`docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md:84`).
- Windows-specific filesystem behaviour (reparse points, hardlinks across volumes, device
  names, MAX_PATH, `rename` semantics over an existing file) is reasoned about from the
  existing Windows-only helpers (`apps/api/src/product/files.rs:635-670`,
  `apps/desktop/src/commands.rs:420-480`) and from the repository's Windows-first evidence
  base. It is **not** verified on macOS or Linux.
- The provider-projection claims in T14 depend on the four adapters emitting what each
  vendor documents. The plan claims only that the emitted shape matches the documented
  shape; a real credentialed round trip is an opt-in gate that this plan does not run, and
  the repository already treats provider interoperability that way (`AGENTS.md` §9).
- The model does not assess cost or token accounting for attachments. Sending a 20 MiB image
  to a provider has a real cost and a real context-window effect, and the plan does not
  bound either. If a maintainer wants a per-attachment token/cost bound, that is a new open
  question, and this model records it as out of scope rather than silently ignored.
