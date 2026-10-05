//! Authoritative registry of the values this process already *knows* are
//! credentials.
//!
//! The evidence export has always guessed secrets by shape: a token that looks
//! like `sk-…`, a `password=` assignment, an environment value. That pass stays
//! as a backstop, but it cannot answer two questions that the runtime can:
//!
//! 1. *Is this exact value a credential?* A provider key resolved from a file
//!    or the OS keyring, or an MCP environment value the configuration declared
//!    secret, matches no prefix at all. Guessing by shape cannot remove it.
//! 2. *Is this field name declared secret?* A declared field must be redacted
//!    even when its value looks innocent, and an ordinary string must not be
//!    mangled just because it resembles a key.
//!
//! This module is the answer to both. Every authority that resolves a
//! credential calls `register_value` with the resolved value and
//! `register_field_name` with the name the configuration declared secret. The
//! emitting boundaries (trace, report, SSE, evidence export, API errors,
//! diagnostics) then redact by *knowledge* first and fall back to the existing
//! pattern pass.
//!
//! # Where the values live, and for how long
//!
//! * Values live only in this process's memory, inside the registry's private
//!   state, for the lifetime of the process.
//! * Nothing in this module implements `Serialize` or `Deserialize`, and
//!   `Debug`/`Display` print counts instead of values, so a registry can never
//!   be written to `trace.jsonl`, `report.json`, a snapshot, an API response,
//!   or a log through a derived formatter.
//! * Nothing is persisted. A restart or a resumed run starts with an empty
//!   registry and repopulates it the next time configuration is loaded, so the
//!   registry never becomes a second secret store on disk. A resumed run inside
//!   the same process keeps the values the process already resolved.
//! * Registration is bounded and a value shorter than the documented floor is
//!   refused, because redacting an ordinary short word would corrupt unrelated
//!   text. Every refusal is reported with a secret-free warning and a typed
//!   [`SecretRefusal`]; none of them is silent.
//!
//! # Arming
//!
//! The registry starts *unarmed* and becomes armed the first time a credential
//! value or a secret field name is declared. The naming convention this module
//! knows (`password`, `api_key`, `*_token`, …) participates in the JSON redaction
//! pass only while armed, so a process that has resolved no credential at all
//! writes byte-identical JSON to the boundary that existed before the registry
//! did — a trace line, a report, or an SSE frame carrying `{"secret": 123}` is
//! left exactly as it was. Arming is per-registry and one-way.
//!
//! [`is_declared_secret_field_name`] is the exception, and answers the
//! declaration question rather than the pass's decision: the evidence export has
//! always redacted convention field names, so it keeps doing that whether or not
//! this process is armed.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Replacement for a value the runtime was explicitly told is a credential.
pub const KNOWN_SECRET_MARKER: &str = "[REDACTED:known_secret]";

/// Replacement for the value of a field whose name is declared secret.
///
/// The same marker the evidence export already writes, so a reader does not
/// have to learn a second vocabulary for an already-documented field.
pub const DECLARED_SECRET_FIELD_MARKER: &str = "[REDACTED:secret_field]";

/// Shortest value the registry will hold: 8 bytes.
///
/// Below this floor a value is far more likely to be an ordinary word than a
/// credential, and redacting every occurrence of it would corrupt unrelated
/// text. The same floor the evidence export already applies to environment
/// values.
const MIN_REGISTERED_VALUE_BYTES: usize = 8;

/// Longest value the registry will hold, mirroring the provider secret bound.
const MAX_REGISTERED_VALUE_BYTES: usize = 16 * 1024;

/// Hard cap on remembered values, so a pathological authority cannot grow the
/// registry without bound.
const MAX_REGISTERED_VALUES: usize = 1_024;

/// Hard cap on remembered secret field names.
const MAX_REGISTERED_FIELD_NAMES: usize = 1_024;

/// Why the registry refused a registration.
///
/// Secret-free by construction: a reason and a byte length are enough to alert
/// on and to act on, while quoting the refused credential in a log would be the
/// leak the registry exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRefusalReason {
    /// Shorter than [`MIN_REGISTERED_VALUE_BYTES`]: redacting it would corrupt
    /// unrelated text.
    ValueTooShort,
    /// Longer than [`MAX_REGISTERED_VALUE_BYTES`], the provider secret bound.
    ValueTooLong,
    /// [`MAX_REGISTERED_VALUES`] reached. The refused value is not redacted, so
    /// **every credential resolved after this point is unprotected too**.
    ValueCapReached,
    /// A field name that normalized to nothing (empty, or punctuation only).
    FieldNameEmpty,
    /// [`MAX_REGISTERED_FIELD_NAMES`] reached.
    FieldCapReached,
}

impl SecretRefusalReason {
    /// Stable spelling for a log field or an alert rule.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValueTooShort => "value_too_short",
            Self::ValueTooLong => "value_too_long",
            Self::ValueCapReached => "value_cap_reached",
            Self::FieldNameEmpty => "field_name_empty",
            Self::FieldCapReached => "field_cap_reached",
        }
    }
}

/// One refused registration: why, and how many bytes the registry was shown.
///
/// The value itself is deliberately absent. A caller that wants to react to a
/// refusal reads this or the warning the registry emits; neither carries the
/// credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretRefusal {
    pub reason: SecretRefusalReason,
    pub bytes: usize,
}

/// Whether `value` is a value the registry can hold.
///
/// Exposed so an authority that resolves a credential — MCP environment values,
/// for one — can refuse to *use* a value the registry would refuse to remember,
/// instead of injecting a credential this process cannot redact.
pub fn is_registrable_value(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() >= MIN_REGISTERED_VALUE_BYTES
        && trimmed.len() <= MAX_REGISTERED_VALUE_BYTES
}

/// Field names this repository already treats as credential-carrying.
///
/// These are declarations by long-standing naming convention, kept byte for
/// byte from the evidence export's list. `token_estimate`/`total_tokens` are
/// deliberately absent: a token *count* is not a credential.
const CONVENTION_SECRET_FIELD_NAMES: &[&str] = &[
    "authorization",
    "proxy_authorization",
    "cookie",
    "set_cookie",
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "id_token",
    "client_secret",
    "private_key",
    "credential",
    "credentials",
];

#[derive(Default)]
struct RegistryState {
    /// Known credential values, longest first, so an overlapping pair redacts
    /// wholly rather than leaving a fragment of the longer value behind.
    values: Vec<String>,
    /// Normalized (lowercase, non-alphanumeric collapsed to `_`) field names
    /// that configuration declared secret.
    fields: BTreeSet<String>,
    /// True once configuration has declared something secret — a credential
    /// value or a secret field name.
    ///
    /// The naming convention is a declaration this repository has always
    /// honoured, but honouring it in the *new* JSON pass unconditionally made
    /// that pass unconditional: a process that had resolved no credential at all
    /// still rewrote `{"secret": 123}` into a string. The convention therefore
    /// participates in the pass only once the authority is armed, so an unarmed
    /// process writes byte-identical text to the boundary that existed before
    /// the registry did. The arming is per-registry and irreversible; a process
    /// that resolves a credential later is armed from that moment on.
    armed: bool,
    /// Values refused because they were empty, too short, too long, or over the
    /// cap. Counters only: the refused bytes are never retained.
    refused_values: u64,
    /// Field names refused because the registry was at its cap.
    refused_fields: u64,
    /// The most recent refused value, for a caller that must react to it.
    last_value_refusal: Option<SecretRefusal>,
    /// The most recent refused field name.
    last_field_refusal: Option<SecretRefusal>,
}

impl RegistryState {
    /// True when the JSON pass should treat `normalized` as a declared secret
    /// field name.
    ///
    /// `is_declared_secret_field_name` answers the other question — what the
    /// name *is* — and is deliberately not armed, because the evidence export has
    /// always redacted a `password` or an `api_key` field whether or not this
    /// process happened to resolve a credential. This one is the pass's
    /// decision, and the pass only runs once something has been declared.
    fn declares(&self, normalized: &str) -> bool {
        if normalized.is_empty() {
            return false;
        }
        if self.fields.contains(normalized) {
            return true;
        }
        self.armed && is_convention_secret_field_name(normalized)
    }
}

/// Make a refusal visible.
///
/// A refusal used to be a counter on a `Debug` impl that nothing read: a
/// credential the registry could not hold looked exactly like one it redacted,
/// and the emitting boundaries carried on printing it. The warning names the
/// reason and the byte length and never the value.
fn warn_refusal(subject: &'static str, reason: SecretRefusalReason, bytes: usize) {
    tracing::warn!(
        subject = subject,
        reason = reason.as_str(),
        bytes = bytes,
        "secret registry refused a {subject} registration; it will not be redacted"
    );
}

fn refuse_value(state: &mut RegistryState, reason: SecretRefusalReason, bytes: usize) {
    state.refused_values = state.refused_values.saturating_add(1);
    state.last_value_refusal = Some(SecretRefusal { reason, bytes });
    warn_refusal("value", reason, bytes);
}

fn refuse_field(state: &mut RegistryState, reason: SecretRefusalReason, bytes: usize) {
    state.refused_fields = state.refused_fields.saturating_add(1);
    state.last_field_refusal = Some(SecretRefusal { reason, bytes });
    warn_refusal("field name", reason, bytes);
}

/// Process-wide registry of known credential values and declared secret fields.
///
/// Shared rather than threaded: the authorities that resolve credentials
/// (configuration load, the provider factory, MCP setup) do not all own the
/// writers that emit, and the alternative — a second handle on every
/// `TraceWriter`, `RunStore`, and `ApiState` — would spread credential-shaped
/// plumbing through the runtime for no behavioural gain. The values never leave
/// the process; see the module documentation.
///
/// There is exactly one of these per process ([`registry`]) and no public
/// constructor. A detached registry would silently redact nothing — it would not
/// hold the values the authorities registered with the process-wide one — so the
/// type is not constructible outside this module.
pub struct SecretRegistry {
    state: RwLock<RegistryState>,
}

impl fmt::Debug for SecretRegistry {
    /// Counts only. A derived `Debug` would print the credential values, and a
    /// registry that can leak through `{:?}` is not an authority worth having.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.read();
        formatter
            .debug_struct("SecretRegistry")
            .field("armed", &state.armed)
            .field("known_values", &state.values.len())
            .field("declared_fields", &state.fields.len())
            .field("refused_values", &state.refused_values)
            .field("refused_fields", &state.refused_fields)
            .finish()
    }
}

impl fmt::Display for SecretRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.read();
        write!(
            formatter,
            "SecretRegistry({} known values, {} declared fields)",
            state.values.len(),
            state.fields.len()
        )
    }
}

impl SecretRegistry {
    /// An empty, unarmed registry. Private on purpose: only [`registry`] may
    /// create one, so a component cannot accidentally hold a detached copy that
    /// redacts nothing.
    fn empty() -> Self {
        Self {
            state: RwLock::new(RegistryState::default()),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, RegistryState> {
        // A poisoned lock still holds a consistent set of already-registered
        // credentials, and refusing to redact because another thread panicked
        // would turn a local failure into a disclosure.
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, RegistryState> {
        self.state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Remember one resolved credential value.
    ///
    /// Returns `true` when the value is now registered. Empty, over-short,
    /// over-long, and over-cap values are refused and counted; the value itself
    /// is never retained in those cases.
    ///
    /// A refusal is not silent: it emits a secret-free warning (reason and byte
    /// length only) and is readable through [`Self::last_value_refusal`] and
    /// [`Self::refused_value_count`], so a caller that discards the returned
    /// flag still cannot mistake "refused" for "redacted".
    pub fn register_value(&self, value: &str) -> bool {
        let trimmed = value.trim();
        let mut state = self.write();
        // Declaring *any* credential arms the pass, including a value the
        // registry then refuses: the caller's intent was to declare one, and
        // leaving the pass off would also skip declared field names.
        state.armed = true;
        if trimmed.len() < MIN_REGISTERED_VALUE_BYTES {
            refuse_value(
                &mut state,
                SecretRefusalReason::ValueTooShort,
                trimmed.len(),
            );
            return false;
        }
        if trimmed.len() > MAX_REGISTERED_VALUE_BYTES {
            refuse_value(&mut state, SecretRefusalReason::ValueTooLong, trimmed.len());
            return false;
        }
        if state.values.iter().any(|known| known == trimmed) {
            return true;
        }
        if state.values.len() >= MAX_REGISTERED_VALUES {
            refuse_value(
                &mut state,
                SecretRefusalReason::ValueCapReached,
                trimmed.len(),
            );
            return false;
        }
        state.values.push(trimmed.to_string());
        // Longest first: replacing a longer value first means a value that is a
        // substring of another cannot leave a recognizable fragment behind.
        state
            .values
            .sort_by_key(|known| std::cmp::Reverse(known.len()));
        true
    }

    /// Remember that configuration declared `name` a credential field.
    ///
    /// Provider and MCP configuration name the environment variables that carry
    /// credentials; the same name appearing as a JSON field is that credential.
    pub fn register_field_name(&self, name: &str) {
        let normalized = normalize_field_name(name);
        let mut state = self.write();
        state.armed = true;
        if normalized.is_empty() {
            refuse_field(&mut state, SecretRefusalReason::FieldNameEmpty, name.len());
            return;
        }
        if state.fields.len() >= MAX_REGISTERED_FIELD_NAMES && !state.fields.contains(&normalized) {
            refuse_field(
                &mut state,
                SecretRefusalReason::FieldCapReached,
                normalized.len(),
            );
            return;
        }
        state.fields.insert(normalized);
    }

    /// How many value registrations have been refused.
    pub fn refused_value_count(&self) -> u64 {
        self.read().refused_values
    }

    /// How many field-name registrations have been refused.
    pub fn refused_field_count(&self) -> u64 {
        self.read().refused_fields
    }

    /// The most recent refused value, if any.
    pub fn last_value_refusal(&self) -> Option<SecretRefusal> {
        self.read().last_value_refusal
    }

    /// The most recent refused field name, if any.
    pub fn last_field_refusal(&self) -> Option<SecretRefusal> {
        self.read().last_field_refusal
    }

    /// Number of known credential values currently held.
    pub fn known_value_count(&self) -> usize {
        self.read().values.len()
    }

    /// Number of explicitly declared secret field names currently held.
    pub fn declared_field_count(&self) -> usize {
        self.read().fields.len()
    }

    /// True when `value` is exactly one of the registered credentials.
    ///
    /// Deliberately exact: shape is the backstop's job, and a false positive
    /// here would mangle an ordinary string.
    pub fn is_known_value(&self, value: &str) -> bool {
        self.read().values.iter().any(|known| known == value)
    }

    /// Replace every registered credential value in free text.
    ///
    /// Returns the redacted text and how many distinct credentials were found.
    pub fn redact_text_with_count(&self, value: &str) -> (String, usize) {
        let state = self.read();
        if state.values.is_empty() {
            return (value.to_string(), 0);
        }
        redact_known_values(&state.values, value)
    }

    /// Replace every registered credential value in free text.
    pub fn redact_text(&self, value: &str) -> String {
        self.redact_text_with_count(value).0
    }

    /// Redact compact or pretty JSON by known value and by declared field name.
    ///
    /// This is the one function every JSON-emitting boundary calls, so trace
    /// lines, `report.json`, SSE frames, diagnostics, and the evidence export
    /// cannot drift on either question:
    ///
    /// * (a) a registered credential value is replaced wherever it appears
    ///   inside a JSON string, including its escaped spelling, without ever
    ///   spanning a delimiter — see [`redact_json_strings`];
    /// * (b) a registered credential value that is written as a bare JSON scalar
    ///   rather than a quoted string is replaced whole, again without spanning a
    ///   delimiter — see [`redact_json_scalars`];
    /// * (c) the value of any field whose name is a declared secret field is
    ///   replaced with `[REDACTED:secret_field]`, whatever the value looks
    ///   like.
    ///
    /// The text must be valid JSON; on malformed input the scan stops at the
    /// first structural surprise and the already-redacted prefix is returned,
    /// so a corrupt line degrades to "redacted what could be parsed" rather
    /// than to an error that would drop the line.
    pub fn redact_json_text(&self, json: &str) -> String {
        let state = self.read();
        let (value_pass, _) = redact_json_strings(&state.values, json);
        let (scalar_pass, _) = redact_json_scalars(&state.values, &value_pass);
        redact_declared_fields(&state, &scalar_pass)
    }

    /// Redact a JSON value in place by known value and declared field name.
    ///
    /// For a boundary that already holds a parsed `Value` and must not
    /// re-serialize it through the text pass: resume history reconciliation uses
    /// it to compare the raw snapshot against the redacted trace stream in one
    /// representation. The evidence export walks its own tree instead, because it
    /// counts each field it replaces.
    pub fn redact_json_value(&self, value: &mut serde_json::Value) -> usize {
        let state = self.read();
        let mut replacements = 0;
        redact_json_value_inner(&state, value, &mut replacements);
        replacements
    }
}

/// The process-wide registry the emitting boundaries consult.
pub fn registry() -> &'static SecretRegistry {
    static GLOBAL: OnceLock<SecretRegistry> = OnceLock::new();
    GLOBAL.get_or_init(SecretRegistry::empty)
}

/// Normalize a field name for comparison.
///
/// ASCII alphanumerics are lowercased; every other character (including `-`,
/// `.`, and `:` in a header name) becomes `_`. The same normalization the
/// evidence export has always used, so `X-Api-Key` and `x_api_key` collapse to
/// the same name.
pub fn normalize_field_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// True when `name` is a declared secret field name: a naming convention the
/// repository has always honoured, or a name configuration registered.
///
/// This is the *declaration* question, and it is deliberately not gated on the
/// registry being armed: the evidence export has always redacted a `password` or
/// an `api_key` field, whether or not this process happened to resolve a
/// credential. The JSON pass consults the armed [`RegistryState::declares`]
/// instead.
pub fn is_declared_secret_field_name(name: &str) -> bool {
    let normalized = normalize_field_name(name);
    if normalized.is_empty() {
        return false;
    }
    let state = registry().read();
    state.fields.contains(&normalized) || is_convention_secret_field_name(&normalized)
}

/// The naming convention alone, without consulting the registry.
fn is_convention_secret_field_name(normalized: &str) -> bool {
    CONVENTION_SECRET_FIELD_NAMES.contains(&normalized)
        || (normalized.ends_with("_token") && !normalized.ends_with("_tokens"))
}

/// Trailing name segments that make a header name credential-bearing.
///
/// A header is rarely named exactly `api_key`; the credential word is a suffix
/// (`x-api-key`, `x-goog-api-key`, `x-auth-token`). Matching the trailing
/// segment rather than the whole name is what keeps both the standard spellings
/// and a vendor prefix covered, while `X-Request-Tag` stays out.
const CREDENTIAL_HEADER_SUFFIXES: &[&str] = &[
    "_api_key",
    "_apikey",
    "_api_token",
    "_token",
    "_secret",
    "_credential",
    "_credentials",
    "_password",
];

/// Whether a header name a profile declares as its credential bearer should also
/// be registered as a secret *field* name.
///
/// Field-name registration is global, so registering an unrecognised name would
/// blank every JSON field that happens to share it — a profile whose auth header
/// is `X-Request-Tag` would rewrite unrelated `x_request_tag` evidence in
/// `trace.jsonl`. Only names that carry a credential by construction are
/// registered: the naming convention this repository already honours, plus the
/// trailing credential segments that convention does not spell out (`x-api-key`
/// and `x-goog-api-key` do not normalize to `api_key`).
///
/// This gates the *field name* only. The header's resolved value is registered
/// either way, so a credential under any other header name is still redacted by
/// value, and an unrecognised header name simply cannot corrupt unrelated
/// evidence.
pub fn is_credential_bearing_header_name(name: &str) -> bool {
    let normalized = normalize_field_name(name);
    is_convention_secret_field_name(&normalized)
        || CREDENTIAL_HEADER_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
}

fn redact_known_values(values: &[String], value: &str) -> (String, usize) {
    let mut output = value.to_string();
    let mut count = 0;
    for credential in values {
        if credential.is_empty() {
            continue;
        }
        count += replace_spelling(&mut output, credential, KNOWN_SECRET_MARKER);
        // A credential may contain `"` or `\`, in which case it appears in JSON
        // text in its escaped spelling. Replacing both spellings keeps such a
        // credential from surviving inside a JSON string.
        let escaped = json_escape_body(credential);
        if escaped != *credential && escaped.len() >= MIN_REGISTERED_VALUE_BYTES {
            count += replace_spelling(&mut output, &escaped, KNOWN_SECRET_MARKER);
        }
    }
    (output, count)
}

/// Replace every occurrence of `needle` in `text`, returning how many were
/// replaced.
fn replace_spelling(text: &mut String, needle: &str, replacement: &str) -> usize {
    if needle.is_empty() || !text.contains(needle) {
        return 0;
    }
    let count = text.matches(needle).count();
    *text = text.replace(needle, replacement);
    count
}

/// Redact the credential values inside the JSON *strings* of `json`.
///
/// The free-text pass cannot be used on serialized JSON: a registered value that
/// contains `"` or `\` occurs in the document in its escaped spelling, and the
/// raw spelling of such a value can straddle the closing quote of the string
/// that holds it — `{"note":"abcdefg"}` contains `abcdefg"` for a credential
/// `abcdefg"`, so a raw replace eats the terminator and leaves
/// `{"note":"[REDACTED:known_secret]}`, which is not JSON at all. The next
/// durable read then fails to deserialize the line.
///
/// So the pass is structure-aware instead: every `"`-delimited token is decoded
/// to the string it denotes, the decoded *content* is redacted, and the result is
/// re-encoded as one token. A replacement is therefore always a whole string
/// body and can never span a delimiter, whatever the credential contains. Object
/// keys are redacted the same way — a credential used as a key is still a
/// credential — and every token without a match is copied byte for byte, so the
/// pass is a no-op on text that carries no registered value.
fn redact_json_strings(values: &[String], json: &str) -> (String, usize) {
    if values.is_empty() {
        return (json.to_string(), 0);
    }
    let bytes = json.as_bytes();
    let mut output = String::with_capacity(json.len());
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            // Bulk-copy up to the next quote so the common case stays linear
            // without a per-character push.
            match json[index..].find('"') {
                Some(offset) => {
                    output.push_str(&json[index..index + offset]);
                    index += offset;
                }
                None => {
                    output.push_str(&json[index..]);
                    break;
                }
            }
            continue;
        }
        let Some(end) = json_string_end(json, index) else {
            // Unterminated: nothing after this point is a string, so it cannot
            // be redacted structurally without risking a broken document.
            output.push_str(&json[index..]);
            break;
        };
        let body = &json[index + 1..end - 1];
        match decode_json_string_body(body) {
            Some(decoded) => {
                let (redacted, replaced) = redact_known_values(values, &decoded);
                if replaced == 0 {
                    output.push_str(&json[index..end]);
                } else {
                    output.push('"');
                    output.push_str(&json_escape_body(&redacted));
                    output.push('"');
                }
                count += replaced;
            }
            None => {
                // A body this decoder cannot read is not the shape `serde_json`
                // writes. Redacting within the token's own bounds still cannot
                // span a delimiter, so the credential is removed without
                // inventing an escape the document would not accept.
                let mut inner = body.to_string();
                for credential in values {
                    if credential.is_empty() {
                        continue;
                    }
                    count += replace_spelling(&mut inner, credential, KNOWN_SECRET_MARKER);
                    let escaped = json_escape_body(credential);
                    if escaped != *credential && escaped.len() >= MIN_REGISTERED_VALUE_BYTES {
                        count += replace_spelling(&mut inner, &escaped, KNOWN_SECRET_MARKER);
                    }
                }
                output.push('"');
                output.push_str(&inner);
                output.push('"');
            }
        }
        index = end;
    }
    (output, count)
}

/// Replace a *bare* JSON scalar that is itself a registered credential.
///
/// A credential whose JSON spelling is not a quoted string is invisible to
/// [`redact_json_strings`], which only rewrites string content: a numeric token
/// emitted as a JSON number — a credential that looks like an id, carried inside
/// `ToolCallStarted.arguments` for instance — would otherwise ride out in the
/// clear because no pattern matches it either.
///
/// The scan walks the text outside strings and cuts it into tokens bounded by
/// JSON structure or whitespace, so a token is never able to span a delimiter. A
/// token whose exact text is a registered value is replaced as a whole with the
/// same marker the value pass uses, written as a JSON *string*: the document stays
/// valid JSON, which is the property that matters. (The declared-field pass has
/// always made the same type change for a declared field whose value was a
/// number.) Everything else is copied byte for byte, so this is a no-op on text
/// that carries no registered value.
fn redact_json_scalars(values: &[String], json: &str) -> (String, usize) {
    let non_empty = values
        .iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if non_empty.is_empty() {
        return (json.to_string(), 0);
    }
    let bytes = json.as_bytes();
    let mut output = String::with_capacity(json.len());
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            // A string token is the other pass's business; copy it whole.
            let end = json_string_end(json, index).unwrap_or(bytes.len());
            output.push_str(&json[index..end]);
            index = end;
            continue;
        }
        let start = index;
        while index < bytes.len() && !is_json_token_boundary(bytes[index]) {
            index += 1;
        }
        if index == start {
            // A structural byte or whitespace: copy it and move on, or the scan
            // would stop making progress on the first delimiter.
            output.push_str(&json[index..index + 1]);
            index += 1;
            continue;
        }
        let token = &json[start..index];
        // Registered values are always within the registry's own length bounds,
        // so a token outside them cannot match one and is copied without a scan.
        let in_bounds =
            token.len() >= MIN_REGISTERED_VALUE_BYTES && token.len() <= MAX_REGISTERED_VALUE_BYTES;
        if in_bounds && non_empty.iter().any(|value| value.as_str() == token) {
            output.push('"');
            output.push_str(KNOWN_SECRET_MARKER);
            output.push('"');
            count += 1;
        } else {
            output.push_str(token);
        }
    }
    (output, count)
}

/// True for a byte that ends a bare JSON token.
///
/// Structure, whitespace, and the opening quote of a string: the bytes a minimal
/// JSON writer emits between two scalars.
fn is_json_token_boundary(byte: u8) -> bool {
    matches!(
        byte,
        b'"' | b',' | b'{' | b'}' | b'[' | b']' | b':' | b' ' | b'\t' | b'\n' | b'\r'
    )
}

/// The string a JSON string body denotes, or `None` when it is not the shape
/// [`json_escape_body`] produces.
///
/// `serde_json` escapes only `"`, `\`, the named control escapes, and other
/// control characters as `\u00xx`, so this is the exact inverse for every
/// document this runtime writes.
fn decode_json_string_body(body: &str) -> Option<String> {
    if !body.contains('\\') {
        return Some(body.to_string());
    }
    let mut output = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match characters.next()? {
            '"' => output.push('"'),
            '\\' => output.push('\\'),
            '/' => output.push('/'),
            'b' => output.push('\u{08}'),
            'f' => output.push('\u{0c}'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'u' => {
                let mut unit = 0u32;
                for _ in 0..4 {
                    unit = unit.checked_mul(16)? + characters.next()?.to_digit(16)?;
                }
                output.push(char::from_u32(unit)?);
            }
            _ => return None,
        }
    }
    Some(output)
}

/// The spelling `serde_json` uses for a string's contents, without the
/// surrounding quotes.
///
/// `serde_json` escapes only `"`, `\`, and the C0 control characters, and
/// leaves every other byte (including non-ASCII) as it is, so this is the exact
/// inverse for the values that can carry those characters. The quotes are
/// excluded on purpose: a replacement must stay *inside* the JSON string it
/// redacts, or the document would stop being valid JSON.
fn json_escape_body(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                let mut buffer = [0u16; 2];
                for unit in character.encode_utf16(&mut buffer) {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
            character => output.push(character),
        }
    }
    output
}

fn redact_json_value_inner(
    state: &RegistryState,
    value: &mut serde_json::Value,
    replacements: &mut usize,
) {
    // A bare scalar can *be* a registered credential — the same case
    // `redact_json_scalars` covers in text form — so the parsed pass has to
    // answer it too, or the two representations would disagree and resume
    // reconciliation would see a mismatch that is not there.
    let scalar_is_credential = match value {
        serde_json::Value::Number(number) => state
            .values
            .iter()
            .any(|registered| registered == &number.to_string()),
        _ => false,
    };
    if scalar_is_credential {
        *value = serde_json::Value::String(KNOWN_SECRET_MARKER.to_string());
        *replacements += 1;
        return;
    }
    match value {
        serde_json::Value::String(text) => {
            let (redacted, count) = redact_known_values(&state.values, text);
            if count > 0 {
                *replacements += count;
                *text = redacted;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_value_inner(state, item, replacements);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                if state.declares(&normalize_field_name(key)) {
                    *field = serde_json::Value::String(DECLARED_SECRET_FIELD_MARKER.to_string());
                    *replacements += 1;
                } else {
                    redact_json_value_inner(state, field, replacements);
                }
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// Rewrite the value of every declared secret field in JSON *text*.
///
/// A string followed by `:` can only be an object key in valid JSON, and a `"`
/// inside a string value is always escaped, so scanning for that shape cannot
/// mistake a value that merely mentions a field name for the field itself.
/// The one shape this does not recognize is a key written with JSON `\u`
/// escapes (`"api\u005fkey"`); `serde_json` never emits those for the ASCII
/// names configuration declares, so every document this runtime writes matches.
fn redact_declared_fields(state: &RegistryState, json: &str) -> String {
    let bytes = json.as_bytes();
    let mut output = String::with_capacity(json.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            // Bulk-copy up to the next quote so the common case stays linear
            // without a per-character push.
            match json[index..].find('"') {
                Some(offset) => {
                    output.push_str(&json[index..index + offset]);
                    index += offset;
                }
                None => {
                    output.push_str(&json[index..]);
                    break;
                }
            }
            continue;
        }
        let Some(end) = json_string_end(json, index) else {
            output.push_str(&json[index..]);
            break;
        };
        let key = &json[index + 1..end - 1];
        let after = skip_json_whitespace(bytes, end);
        let is_object_key = after < bytes.len() && bytes[after] == b':';
        if is_object_key && state.declares(&normalize_field_name(key)) {
            let value_start = skip_json_whitespace(bytes, after + 1);
            let value_end = json_value_end(json, value_start).unwrap_or(json.len());
            output.push_str(&json[index..end]);
            output.push(':');
            output.push('"');
            output.push_str(DECLARED_SECRET_FIELD_MARKER);
            output.push('"');
            index = value_end;
            continue;
        }
        output.push_str(&json[index..end]);
        index = end;
    }
    output
}

fn skip_json_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r') {
        index += 1;
    }
    index
}

/// Index just past the closing quote of the string that starts at `start`.
///
/// Byte-wise is safe: UTF-8 continuation bytes are all `>= 0x80` and so can
/// never be mistaken for `"` or `\`.
fn json_string_end(json: &str, start: usize) -> Option<usize> {
    let bytes = json.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// Index just past the JSON value that starts at `start`.
fn json_value_end(json: &str, start: usize) -> Option<usize> {
    let bytes = json.as_bytes();
    let first = *bytes.get(start)?;
    match first {
        b'"' => json_string_end(json, start),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut index = start;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' => {
                        index = json_string_end(json, index)?;
                    }
                    b'{' | b'[' => {
                        depth += 1;
                        index += 1;
                    }
                    b'}' | b']' => {
                        depth = depth.saturating_sub(1);
                        index += 1;
                        if depth == 0 {
                            return Some(index);
                        }
                    }
                    _ => index += 1,
                }
            }
            Some(json.len())
        }
        _ => {
            let mut index = start;
            while index < bytes.len()
                && !matches!(
                    bytes[index],
                    b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r'
                )
            {
                index += 1;
            }
            Some(index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_value_that_matches_no_pattern_is_redacted_exactly() {
        let state = SecretRegistry::empty();
        // Matches no prefix the pattern pass knows: this is the whole point.
        let canary = "unpatterned-credential-canary-4c31ba";
        assert!(state.register_value(canary));

        let (text, count) = state.redact_text_with_count(&format!("before {canary} after"));
        assert_eq!(count, 1);
        assert_eq!(text, format!("before {KNOWN_SECRET_MARKER} after"));
        assert!(!text.contains(canary));
        // A look-alike the runtime was never told about is left alone here: the
        // pattern pass, not this authority, is what decides about shape.
        let lookalike = "sk-not-registered-lookalike";
        assert_eq!(state.redact_text(lookalike), lookalike);
        assert!(!state.is_known_value(lookalike));
        assert!(state.is_known_value(canary));
    }

    /// Arming, in both directions. An unarmed process writes the JSON it was
    /// given, byte for byte: the convention list is a declaration, and
    /// honouring it before anything has been declared is what made the pass
    /// unconditional.
    #[test]
    fn the_convention_list_participates_only_once_the_authority_is_armed() {
        let json = serde_json::json!({ "secret": 123, "note": "ordinary" }).to_string();

        let state = SecretRegistry::empty();
        assert_eq!(
            state.redact_json_text(&json),
            json,
            "an unarmed registry rewrites nothing, including the value type"
        );
        assert!(!state.read().declares(&normalize_field_name("secret")));

        // A value registration arms it.
        let state = SecretRegistry::empty();
        assert!(state.register_value("arming-credential-canary-3d7f"));
        let redacted = state.redact_json_text(&json);
        assert!(
            redacted.contains(DECLARED_SECRET_FIELD_MARKER),
            "{redacted}"
        );
        assert!(!redacted.contains("123"), "{redacted}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&redacted).unwrap()["note"],
            "ordinary"
        );

        // So does an explicit field-name declaration, and so does a declaration
        // the registry then *refuses*: the intent to declare is what arms pass.
        let state = SecretRegistry::empty();
        state.register_field_name("x-tenant-credential");
        assert!(
            state
                .redact_json_text(&json)
                .contains(DECLARED_SECRET_FIELD_MARKER)
        );

        let state = SecretRegistry::empty();
        assert!(!state.register_value("short"));
        assert!(
            state
                .redact_json_text(&json)
                .contains(DECLARED_SECRET_FIELD_MARKER),
            "a refused registration still declared a credential"
        );

        // The declaration question is not armed: the evidence export has always
        // applied the convention, whatever this process resolved.
        assert!(is_declared_secret_field_name("secret"));
    }

    #[test]
    fn a_declared_field_is_redacted_even_when_its_value_is_innocent() {
        let state = SecretRegistry::empty();
        state.register_field_name("x-tenant-credential");
        let json = serde_json::json!({
            "x-tenant-credential": "obviously-fine-value",
            "nested": { "x_tenant_credential": "another-fine-value" },
            "api_key": "third-fine-value",
            "total_tokens": 42,
        })
        .to_string();

        let redacted = state.redact_json_text(&json);
        assert!(!redacted.contains("obviously-fine-value"), "{redacted}");
        assert!(!redacted.contains("another-fine-value"), "{redacted}");
        assert!(!redacted.contains("third-fine-value"), "{redacted}");
        assert_eq!(redacted.matches(DECLARED_SECRET_FIELD_MARKER).count(), 3);
        // A token *count* is not a credential and stays readable.
        assert!(redacted.contains("\"total_tokens\":42"), "{redacted}");
        assert!(serde_json::from_str::<serde_json::Value>(&redacted).is_ok());
    }

    #[test]
    fn declared_field_names_combine_the_convention_with_registration() {
        assert!(is_declared_secret_field_name("Authorization"));
        assert!(is_declared_secret_field_name("github_token"));
        assert!(!is_declared_secret_field_name("github_tokens"));
        assert!(!is_declared_secret_field_name("token_estimate"));
        assert!(!is_declared_secret_field_name("total_tokens"));
        // A name the convention does not know is only declared once the
        // authority that owns it registers it. Provider profiles and MCP
        // servers register the header or environment name configuration gave.
        let unregistered = "x-api-key-unregistered-canary";
        assert!(!is_declared_secret_field_name(unregistered));
        let state = SecretRegistry::empty();
        assert!(!state.read().declares(&normalize_field_name(unregistered)));
        // Case and separators are the profile's business; the declaration is
        // the same name whichever spelling the configuration used.
        state.register_field_name("X-Api-Key");
        assert!(state.read().declares(&normalize_field_name("x-api-key")));
        assert!(state.read().declares(&normalize_field_name("X_API_KEY")));
        assert!(!state.read().declares(&normalize_field_name(unregistered)));
    }

    /// A registered credential containing `"` must not eat the quote that ends
    /// the string it sits in. The raw spelling of such a value occurs in JSON
    /// text across a delimiter, so a whole-line replace leaves a document that
    /// does not parse — and the durable reader then drops the line.
    #[test]
    fn a_credential_containing_a_quote_cannot_corrupt_the_line_it_redacts() {
        let state = SecretRegistry::empty();
        let canary = "abcdefg\"";
        assert_eq!(canary.len(), 8, "the floor is 8 bytes");
        assert!(state.register_value(canary));

        let json = serde_json::json!({ "note": canary }).to_string();
        assert!(json.contains("abcdefg\\\""), "{json}");
        let redacted = state.redact_json_text(&json);
        assert!(!redacted.contains("abcdefg"), "{redacted}");
        assert_eq!(
            redacted,
            serde_json::json!({ "note": KNOWN_SECRET_MARKER }).to_string(),
            "{redacted}"
        );
        // The point of the fix: the line is still JSON, so a durable read of it
        // cannot fail and lose the event.
        let parsed: serde_json::Value =
            serde_json::from_str(&redacted).unwrap_or_else(|error| panic!("{error}: {redacted}"));
        assert_eq!(parsed["note"], KNOWN_SECRET_MARKER);

        // A backslash-bearing value is the same hazard with the other escape.
        let state = SecretRegistry::empty();
        let backslash = "credential\\";
        assert!(state.register_value(backslash));
        let json = serde_json::json!({ "note": backslash, "keep": "keep-this" }).to_string();
        let redacted = state.redact_json_text(&json);
        let parsed: serde_json::Value =
            serde_json::from_str(&redacted).unwrap_or_else(|error| panic!("{error}: {redacted}"));
        assert_eq!(parsed["note"], KNOWN_SECRET_MARKER);
        assert_eq!(parsed["keep"], "keep-this");
    }

    /// The structural pass must be a no-op on text that carries no registered
    /// value, so a boundary cannot be charged for bytes it did not change.
    #[test]
    fn the_structural_pass_leaves_unmatched_json_byte_identical() {
        let state = SecretRegistry::empty();
        assert!(state.register_value("registered-credential-canary"));
        let json = "{\n  \"note\": \"ordinary text with \\\"escapes\\\" and \\\\slashes\\\\\",\n  \"n\": 7\n}";
        assert_eq!(state.redact_json_text(json), json);
        assert_eq!(state.redact_text(json), json);
    }

    /// A credential can be written as a bare scalar, not a quoted string.
    ///
    /// A numeric-looking credential inside `ToolCallStarted.arguments` is the
    /// realistic case: the string pass cannot see it (it only rewrites string
    /// content) and no pattern matches it either, so it used to ride out in the
    /// clear. The scalar pass replaces the whole token, and the result is still
    /// valid JSON that a trace reader can parse back.
    #[test]
    fn a_credential_written_as_a_bare_json_scalar_is_replaced_whole() {
        let state = SecretRegistry::empty();
        let numeric = "9876543210";
        assert!(state.register_value(numeric));

        let line = "{\"call_id\":\"c1\",\"name\":\"shell\",\"arguments\":{\"token\":9876543210,\"keep\":\"visible\"}}";
        let redacted = state.redact_json_text(line);
        assert!(!redacted.contains(numeric), "{redacted}");
        assert_eq!(
            redacted,
            "{\"call_id\":\"c1\",\"name\":\"shell\",\"arguments\":{\"token\":\"[REDACTED:known_secret]\",\"keep\":\"visible\"}}"
        );
        // Still JSON, and the surrounding evidence is untouched.
        let parsed: serde_json::Value = serde_json::from_str(&redacted).unwrap();
        assert_eq!(parsed["arguments"]["keep"], "visible");
        assert_eq!(
            parsed["arguments"]["token"],
            serde_json::Value::String(KNOWN_SECRET_MARKER.to_string())
        );
        // The parsed pass answers the same question the same way, so resume
        // reconciliation cannot see a mismatch between the two representations.
        let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(state.redact_json_value(&mut value), 1);
        assert_eq!(value, parsed);

        // A token that merely *contains* the credential is not a token that is
        // the credential: no replacement ever spans a delimiter or rewrites part
        // of a larger number.
        let longer = "{\"token\":198765432109,\"negative\":-9876543210,\"float\":9876543210.5}";
        let untouched = state.redact_json_text(longer);
        assert!(untouched.contains("198765432109"), "{untouched}");
        assert!(untouched.contains("-9876543210"), "{untouched}");
        assert!(untouched.contains("9876543210.5"), "{untouched}");
        assert_eq!(
            untouched.matches(KNOWN_SECRET_MARKER).count(),
            0,
            "{untouched}"
        );

        // Inside a string the string pass owns the token, and inside a *longer*
        // string token the value still matches as content.
        assert_eq!(
            state.redact_json_text("{\"note\":\"token 9876543210 stays\"}"),
            "{\"note\":\"token [REDACTED:known_secret] stays\"}"
        );
    }

    #[test]
    fn header_names_are_registrable_only_when_they_carry_a_credential() {
        // The names this repository already treats as credential-carrying.
        assert!(is_credential_bearing_header_name("Authorization"));
        assert!(is_credential_bearing_header_name("x-goog-api-key"));
        assert!(is_credential_bearing_header_name("X-Api-Key"));
        assert!(is_credential_bearing_header_name("X-Auth-Token"));
        // An ordinary header name is not: registering it would blank every JSON
        // field that happens to share it, in every trace in the process.
        assert!(!is_credential_bearing_header_name("X-Request-Tag"));
        assert!(!is_credential_bearing_header_name("X-Tenant"));
        assert!(!is_credential_bearing_header_name("traceparent"));
        assert!(!is_credential_bearing_header_name("X-Key-Id"));
    }

    #[test]
    fn field_names_inside_string_values_are_not_mistaken_for_fields() {
        let state = SecretRegistry::empty();
        // Armed by an unrelated value: this test is about the *structural*
        // detection of a key, not about arming.
        assert!(state.register_value("structural-detection-canary-8e13"));
        let json = serde_json::json!({
            "message": "the note literally says \"password\":\"still-visible\"",
            "password": "must-not-survive",
        })
        .to_string();

        let redacted = state.redact_json_text(&json);
        assert!(
            redacted.contains("still-visible"),
            "a field name inside a string value is not a field: {redacted}"
        );
        assert!(!redacted.contains("must-not-survive"), "{redacted}");
        assert!(serde_json::from_str::<serde_json::Value>(&redacted).is_ok());
    }

    #[test]
    fn pretty_and_nested_json_keep_their_shape() {
        let state = SecretRegistry::empty();
        let canary = "escaped-credential-canary-77aa";
        state.register_value(canary);
        let value = serde_json::json!({
            "outer": { "inner": [1, { "deep": format!("see {canary} here") }] },
            "trailing": true,
        });
        let pretty = serde_json::to_string_pretty(&value).unwrap();

        let redacted = state.redact_json_text(&pretty);
        assert!(!redacted.contains(canary), "{redacted}");
        assert!(redacted.contains(KNOWN_SECRET_MARKER));
        let parsed: serde_json::Value = serde_json::from_str(&redacted).unwrap();
        assert_eq!(
            parsed["outer"]["inner"][1]["deep"],
            "see [REDACTED:known_secret] here"
        );
        assert_eq!(parsed["trailing"], true);
    }

    #[test]
    fn a_credential_that_needs_json_escaping_is_redacted_in_its_escaped_spelling() {
        let state = SecretRegistry::empty();
        let canary = "quote\"inside\\credential";
        assert!(state.register_value(canary));
        let json = serde_json::json!({ "note": canary }).to_string();
        assert!(json.contains("\\\""), "{json}");

        let redacted = state.redact_json_text(&json);
        assert!(!redacted.contains("inside"), "{redacted}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&redacted).unwrap()["note"],
            KNOWN_SECRET_MARKER
        );
    }

    #[test]
    fn non_ascii_values_are_redacted_without_splitting_characters() {
        let state = SecretRegistry::empty();
        let canary = "凭据-canary-中文字符";
        assert!(state.register_value(canary));
        let json = serde_json::json!({ "note": format!("前缀 {canary} 后缀") }).to_string();
        let redacted = state.redact_json_text(&json);
        assert!(!redacted.contains("凭据"), "{redacted}");
        assert!(redacted.contains("前缀"));
        assert!(!redacted.contains('\u{fffd}'));
    }

    #[test]
    fn malformed_json_is_redacted_as_far_as_it_parses_rather_than_failing() {
        let state = SecretRegistry::empty();
        let canary = "malformed-credential-canary-90fe";
        state.register_value(canary);
        let truncated = format!("{{\"password\":\"hunter2\",\"note\":\"{canary}\"");
        let redacted = state.redact_json_text(&truncated);
        assert!(!redacted.contains(canary), "{redacted}");
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(
            redacted.contains(DECLARED_SECRET_FIELD_MARKER),
            "{redacted}"
        );
    }

    #[test]
    fn registration_is_bounded_and_counts_what_it_refuses() {
        let state = SecretRegistry::empty();
        assert!(!state.register_value("short"), "8 bytes is the floor");
        assert!(!state.register_value("        "), "whitespace only");
        assert!(!state.register_value(&"x".repeat(MAX_REGISTERED_VALUE_BYTES + 1)));
        assert_eq!(state.known_value_count(), 0);
        assert!(state.register_value("long-enough-value"));
        assert!(state.register_value("long-enough-value"), "idempotent");
        assert_eq!(state.known_value_count(), 1);

        for index in 0..MAX_REGISTERED_VALUES {
            state.register_value(&format!("canary-{index:08}-value"));
        }
        assert_eq!(state.known_value_count(), MAX_REGISTERED_VALUES);
        assert!(!state.register_value("one-canary-too-many"));
        assert_eq!(state.known_value_count(), MAX_REGISTERED_VALUES);

        state.register_field_name("");
        assert_eq!(state.declared_field_count(), 0);
        let rendered = format!("{state:?}");
        assert!(!rendered.contains("canary"), "{rendered}");
        assert!(rendered.contains(&format!("known_values: {MAX_REGISTERED_VALUES}")));
    }

    /// A refusal has to be visible and typed, not a counter on a `Debug` impl
    /// that nothing reads: a credential the registry could not hold is a
    /// credential that will be printed, which is the opposite of the guarantee it
    /// looks like.
    #[test]
    fn a_refused_value_is_loud_typed_and_never_reported_as_registered() {
        let state = SecretRegistry::empty();
        let short = "7bytes!";
        assert_eq!(short.len(), 7, "the floor is 8 bytes");
        let oversized = "L".repeat(MAX_REGISTERED_VALUE_BYTES + 1);

        assert!(!state.register_value(short));
        assert_eq!(
            state.last_value_refusal(),
            Some(SecretRefusal {
                reason: SecretRefusalReason::ValueTooShort,
                bytes: 7,
            })
        );
        assert!(!state.register_value(&oversized));
        assert_eq!(
            state.last_value_refusal(),
            Some(SecretRefusal {
                reason: SecretRefusalReason::ValueTooLong,
                bytes: MAX_REGISTERED_VALUE_BYTES + 1,
            })
        );

        // Two refusals, and neither value is reported as registered.
        assert_eq!(state.refused_value_count(), 2);
        assert_eq!(state.known_value_count(), 0);
        assert!(!state.is_known_value(short));
        assert!(!state.is_known_value(&oversized));
        // The signal is secret-free: a reason and a length, no bytes.
        let rendered = format!("{:?}", state.last_value_refusal().unwrap());
        assert!(rendered.contains("ValueTooLong"), "{rendered}");
        assert!(!rendered.contains("LLLL"), "{rendered}");

        // `is_registrable_value` answers the same question the registry does, so
        // an authority can refuse to *use* a value the registry cannot remember.
        assert!(!is_registrable_value(short));
        assert!(!is_registrable_value(&oversized));
        assert!(is_registrable_value("long-enough-value"));
        assert!(is_registrable_value(
            &"L".repeat(MAX_REGISTERED_VALUE_BYTES)
        ));
    }

    /// A dispatch that is interested in every callsite and records nothing.
    ///
    /// `tracing` decides whether an event is emitted at all from a *process-wide*
    /// cache of each callsite's interest, computed once from whichever dispatches
    /// were live when some thread first reached the callsite, and the `event!`
    /// macro checks that cached value *before* it consults the active subscriber.
    /// Every other test in this binary reaches `warn_refusal` from a thread with
    /// no subscriber, so one of them can latch "never interested" for the whole
    /// process; the loser of that one-time registration only survives its own
    /// first event, which is exactly one warning lost out of two here. Keeping an
    /// always-interested dispatch alive means every such computation sees an
    /// interested subscriber, so "never" cannot be latched.
    struct AlwaysInterested;

    impl tracing::Subscriber for AlwaysInterested {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

        fn event(&self, _: &tracing::Event<'_>) {}

        fn enter(&self, _: &tracing::span::Id) {}

        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// Keep `tracing`'s process-wide callsite-interest cache from latching
    /// "never interested" for a warning this test captures: see
    /// `AlwaysInterested`.
    fn keep_warning_callsites_interested() -> &'static tracing::Dispatch {
        static GUARD: OnceLock<tracing::Dispatch> = OnceLock::new();
        GUARD.get_or_init(|| tracing::Dispatch::new(AlwaysInterested))
    }

    /// A refusal is emitted as a warning, not only counted.
    ///
    /// The counters and `last_*_refusal` are readable, but a refusal is a
    /// security event that has to reach an operator's log by itself: a process
    /// that asked the authority to remember a credential and silently got "no"
    /// would print that credential everywhere. This captures the `tracing` event
    /// on this thread and asserts the fields it carries — the subject, the typed
    /// reason, the byte count — and that it never carries the value.
    #[test]
    fn a_refused_registration_warns_with_the_reason_and_never_the_value() {
        #[derive(Default)]
        struct CaptureWarnings {
            captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        }

        impl tracing::Subscriber for CaptureWarnings {
            fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
                *metadata.level() <= tracing::Level::WARN
            }

            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }

            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

            fn event(&self, event: &tracing::Event<'_>) {
                struct Fields(String);
                impl tracing::field::Visit for Fields {
                    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                        use std::fmt::Write;
                        let _ = write!(self.0, " {}={value}", field.name());
                    }

                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        value: &dyn std::fmt::Debug,
                    ) {
                        use std::fmt::Write;
                        let _ = write!(self.0, " {}={value:?}", field.name());
                    }
                }

                let mut fields = Fields(String::new());
                event.record(&mut fields);
                self.captured.lock().unwrap().push(fields.0);
            }

            fn enter(&self, _: &tracing::span::Id) {}

            fn exit(&self, _: &tracing::span::Id) {}
        }

        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = CaptureWarnings {
            captured: captured.clone(),
        };
        // Registered before the capturing subscriber, so both are live while
        // this test runs and the callsite's interest cannot be decided by a
        // subscriber-less thread.
        let _interest_guard = keep_warning_callsites_interested();

        let state = SecretRegistry::empty();
        let short = "7bytes!";
        tracing::subscriber::with_default(subscriber, || {
            assert!(!state.register_value(short));
            // An empty field name is the field-name refusal with no value.
            state.register_field_name("");
        });

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 2, "one warning per refusal: {captured:?}");
        let value_warning = &captured[0];
        assert!(value_warning.contains("subject=value"), "{value_warning}");
        assert!(
            value_warning.contains("reason=value_too_short"),
            "{value_warning}"
        );
        assert!(value_warning.contains("bytes=7"), "{value_warning}");
        assert!(
            value_warning.contains("will not be redacted"),
            "{value_warning}"
        );
        let field_warning = &captured[1];
        assert!(
            field_warning.contains("subject=field name"),
            "{field_warning}"
        );
        assert!(
            field_warning.contains("reason=field_name_empty"),
            "{field_warning}"
        );
        assert!(
            captured.iter().all(|warning| !warning.contains(short)),
            "a refusal warning never carries the value: {captured:?}"
        );
    }

    /// The value cap is the dangerous refusal: past it, redaction stops for
    /// every credential resolved afterwards, so it must be the loudest one.
    #[test]
    fn the_value_cap_is_reported_rather_than_silently_dropping_later_credentials() {
        let state = SecretRegistry::empty();
        for index in 0..MAX_REGISTERED_VALUES {
            assert!(state.register_value(&format!("canary-{index:08}-value")));
        }
        let overflow = "one-canary-too-many";
        assert!(!state.register_value(overflow));
        assert_eq!(
            state.last_value_refusal(),
            Some(SecretRefusal {
                reason: SecretRefusalReason::ValueCapReached,
                bytes: overflow.len(),
            })
        );
        assert_eq!(state.refused_value_count(), 1);
        assert_eq!(state.known_value_count(), MAX_REGISTERED_VALUES);
        assert!(!state.is_known_value(overflow));
    }

    #[test]
    fn a_refused_field_name_is_reported_too() {
        let state = SecretRegistry::empty();
        // Whitespace normalizes to underscores, so it is a name, not nothing;
        // only a genuinely empty name is refused.
        state.register_field_name("   ");
        assert_eq!(state.declared_field_count(), 1);
        assert_eq!(state.refused_field_count(), 0);

        state.register_field_name("");
        assert_eq!(
            state.last_field_refusal(),
            Some(SecretRefusal {
                reason: SecretRefusalReason::FieldNameEmpty,
                bytes: 0,
            })
        );
        assert_eq!(state.refused_field_count(), 1);
        assert_eq!(state.declared_field_count(), 1);
    }

    #[test]
    fn the_registry_never_prints_a_value_in_debug_or_display() {
        let state = SecretRegistry::empty();
        let canary = "debug-format-credential-canary-1c2d";
        state.register_value(canary);
        state.register_field_name("declared-field-canary");

        for rendered in [format!("{state:?}"), state.to_string()] {
            assert!(!rendered.contains(canary), "{rendered}");
            assert!(rendered.contains('1'), "{rendered}");
        }
        // Field names are names, not values, and their count stays readable so
        // the registry remains diagnosable without disclosing a credential.
        assert!(format!("{state:?}").contains("declared_fields"));
    }

    #[test]
    fn value_redaction_prefers_the_longest_match() {
        let state = SecretRegistry::empty();
        assert!(state.register_value("prefix-credential"));
        assert!(state.register_value("credential"));
        assert_eq!(state.redact_text("prefix-credential"), KNOWN_SECRET_MARKER);
    }

    #[test]
    fn json_value_redaction_matches_the_text_pass() {
        let state = SecretRegistry::empty();
        let canary = "value-pass-credential-canary-5b7e";
        state.register_value(canary);
        state.register_field_name("x-tenant-credential");
        let mut value = serde_json::json!({
            "note": format!("carrying {canary}"),
            "x-tenant-credential": "innocent",
        });

        let replacements = state.redact_json_value(&mut value);
        assert_eq!(replacements, 2);
        assert_eq!(value["note"], format!("carrying {KNOWN_SECRET_MARKER}"));
        assert_eq!(value["x-tenant-credential"], DECLARED_SECRET_FIELD_MARKER);
    }
}
