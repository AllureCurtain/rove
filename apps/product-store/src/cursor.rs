//! Opaque pagination cursors for the product session listing.
//!
//! The listing is ordered by a three-part key — live
//! sessions before archived ones, then most-recently-updated first, then id as
//! a tiebreak — so "resume after this row" cannot be expressed as one number
//! the way `/messages?after_seq=` can. A cursor carries the whole key.
//!
//! It is encoded rather than exposed as three query parameters for one reason
//! that outlives convenience: the sort key is an implementation detail of the
//! index backing the listing. Clients that could name `updated_at` and the
//! archived rank would pin them, and the ordering could not be changed later
//! without breaking them. An opaque token can be re-minted at will.
//!
//! Opaque is not the same as trusted. A cursor is decoded strictly and every
//! field is validated, because it arrives from the wire like any other input.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::ProductSessionId;

/// Rank of a live (non-archived) session in the listing order.
pub const SESSION_RANK_LIVE: i64 = 0;
/// Rank of an archived session in the listing order.
pub const SESSION_RANK_ARCHIVED: i64 = 1;

/// Longest cursor this API will even attempt to decode.
///
/// A well-formed cursor is around 100 bytes. The cap exists so a client cannot
/// make the server base64-decode a megabyte to learn that it was garbage.
const MAX_ENCODED_CURSOR_BYTES: usize = 512;

/// Longest timestamp this API will accept inside a cursor.
///
/// RFC3339 with nanoseconds and a numeric offset fits well under this. The
/// value is only ever used as a bound SQL parameter, so the cap is about
/// refusing nonsense early rather than about safety.
const MAX_CURSOR_TIMESTAMP_BYTES: usize = 64;

/// A decoded position in the session listing: the exact sort key of the last
/// row a client has already seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductSessionCursor {
    /// `0` for live sessions, `1` for archived ones. Named `r` because this
    /// travels in a URL on every page request.
    #[serde(rename = "r")]
    pub archived_rank: i64,
    /// The row's `updated_at`, verbatim.
    #[serde(rename = "u")]
    pub updated_at: String,
    /// The row's id, which makes the key total.
    #[serde(rename = "i")]
    pub session_id: ProductSessionId,
}

/// Why a cursor could not be decoded.
///
/// Callers map every variant to the same client-facing error: the distinction
/// is for logs and tests, not for telling a client how to forge a better one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductCursorError {
    /// Longer than [`MAX_ENCODED_CURSOR_BYTES`].
    TooLong,
    /// Not valid base64url.
    NotBase64,
    /// Valid base64url, but not the JSON shape a cursor has.
    NotACursor,
    /// Right shape, but a field held a value the listing order cannot produce.
    OutOfRange,
}

/// A decoded position in one session's message-search results.
///
/// Message search is ordered by the ledger's `seq`, so the position itself is
/// one number — but the cursor also carries a digest of the *query* it was
/// minted for: the session id and the search term. Keyset paging is only
/// meaningful against the query that produced the key, and that query includes
/// the session: a cursor minted in session A and presented in session B would
/// silently skip every hit of B before that `seq` and answer `hits: []` as if
/// B had no match at all. Binding both turns each mismatch into a typed 400
/// rather than a silently wrong page, which is what the route documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductMessageSearchCursor {
    /// Ledger `seq` of the last hit the client has already seen. Named `s`
    /// because the token travels in a URL on every page request.
    #[serde(rename = "s")]
    pub message_seq: i64,
    /// Digest of the session id and the search term, from
    /// `rove_runtime::context::stable_hash`.
    #[serde(rename = "h")]
    pub query_digest: String,
}

/// A decoded position in the unified product search.
///
/// One token serves the workspace, session, and trace scopes because they share
/// one page shape and one failure vocabulary; which fields are populated *is*
/// the description of the position, and the scope that minted the token is
/// checked separately through the query digest.
///
/// Message scopes stop at a `(session, seq)` key, which is a range scan over
/// the ledger index. Trace scope cannot use that key: a trace is a file, and a
/// record's sequence number is only discoverable by reading the records before
/// it. The position is therefore a byte offset into the run's `trace.jsonl`,
/// which makes resuming a seek instead of a replay — the difference between
/// bounded work and a scan whose cost grows with every page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductSearchCursor {
    /// Ledger `seq` of the last message hit. Message scopes only.
    #[serde(rename = "m", default, skip_serializing_if = "Option::is_none")]
    pub message_seq: Option<i64>,
    /// Session holding that message. Workspace scope only: in session scope the
    /// id is already fixed by the scope, so repeating it would be noise.
    #[serde(rename = "i", default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<ProductSessionId>,
    /// Binding ordinal of the run the trace scan stopped in. Trace scope only.
    #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
    pub run_ordinal: Option<u64>,
    /// Byte offset in that run's `trace.jsonl` to resume reading at. Trace scope
    /// only, and always a line boundary this server produced.
    #[serde(rename = "o", default, skip_serializing_if = "Option::is_none")]
    pub record_offset: Option<u64>,
    /// Digest of the resolved scope and the term, from
    /// `rove_runtime::context::stable_hash`.
    #[serde(rename = "h")]
    pub query_digest: String,
}

/// Longest accepted cursor digest. `stable_hash` renders `sha256:` plus 64 hex
/// characters; the cap refuses nonsense before it reaches a comparison.
const MAX_CURSOR_DIGEST_BYTES: usize = 128;

impl ProductMessageSearchCursor {
    /// Build the cursor that resumes after `message_seq`.
    pub fn after(message_seq: i64, query_digest: &str) -> Self {
        Self {
            message_seq,
            query_digest: query_digest.to_string(),
        }
    }

    /// Render the cursor as a URL-safe token, unpadded like the session cursor
    /// so it needs no escaping in a query string.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a cursor is always serializable");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    /// Recover a cursor from a client-supplied token.
    ///
    /// A `seq` below 1 cannot be produced by the ledger counter, and an empty
    /// or oversized digest cannot be produced by `stable_hash`; both mean the
    /// token was not minted here. The digest's *value* is checked against the
    /// request by the caller, which is where the session and term are known.
    pub fn decode(encoded: &str) -> Result<Self, ProductCursorError> {
        if encoded.len() > MAX_ENCODED_CURSOR_BYTES {
            return Err(ProductCursorError::TooLong);
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| ProductCursorError::NotBase64)?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| ProductCursorError::NotACursor)?;
        if cursor.message_seq < 1 {
            return Err(ProductCursorError::OutOfRange);
        }
        if cursor.query_digest.is_empty() || cursor.query_digest.len() > MAX_CURSOR_DIGEST_BYTES {
            return Err(ProductCursorError::OutOfRange);
        }
        Ok(cursor)
    }
}

/// A decoded position inside one trace file: which run, and how far into it.
///
/// Both fields are always present together. That is what makes a cursor with
/// half a trace position — a run and no offset, or an offset and no run —
/// unrepresentable rather than merely suspicious.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductTraceCursorPosition {
    pub run_ordinal: u64,
    pub record_offset: u64,
}

impl ProductSearchCursor {
    /// Build the cursor that resumes after one message hit.
    ///
    /// `session_id` is `Some` only for the workspace scope, where a position is
    /// not meaningful without knowing which session it sits in.
    pub fn after_message(
        session_id: Option<ProductSessionId>,
        message_seq: i64,
        query_digest: &str,
    ) -> Self {
        Self {
            message_seq: Some(message_seq),
            session_id,
            run_ordinal: None,
            record_offset: None,
            query_digest: query_digest.to_string(),
        }
    }

    /// Build the cursor that resumes reading a run's trace at `record_offset`.
    pub fn after_trace(run_ordinal: u64, record_offset: u64, query_digest: &str) -> Self {
        Self {
            message_seq: None,
            session_id: None,
            run_ordinal: Some(run_ordinal),
            record_offset: Some(record_offset),
            query_digest: query_digest.to_string(),
        }
    }

    /// Render the cursor as a URL-safe token, unpadded like the session cursor
    /// so it needs no escaping in a query string.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a cursor is always serializable");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    /// Recover a cursor from a client-supplied token.
    ///
    /// Decoding is shape-only: it proves the token is one this server could
    /// have minted, not that it belongs to *this* request. The digest's value
    /// is checked by the caller, which is the only place that knows the
    /// resolved scope and the term.
    pub fn decode(encoded: &str) -> Result<Self, ProductCursorError> {
        if encoded.len() > MAX_ENCODED_CURSOR_BYTES {
            return Err(ProductCursorError::TooLong);
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| ProductCursorError::NotBase64)?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| ProductCursorError::NotACursor)?;
        if cursor.query_digest.is_empty() || cursor.query_digest.len() > MAX_CURSOR_DIGEST_BYTES {
            return Err(ProductCursorError::OutOfRange);
        }
        if cursor.message_position().is_none() && cursor.trace_position().is_none() {
            return Err(ProductCursorError::OutOfRange);
        }
        Ok(cursor)
    }

    /// The message position this cursor holds, when it holds exactly one.
    pub fn message_position(&self) -> Option<(Option<&ProductSessionId>, i64)> {
        if self.run_ordinal.is_some() || self.record_offset.is_some() {
            return None;
        }
        let message_seq = self.message_seq?;
        (message_seq >= 1).then_some((self.session_id.as_ref(), message_seq))
    }

    /// The trace position this cursor holds, when it holds exactly one.
    pub fn trace_position(&self) -> Option<ProductTraceCursorPosition> {
        if self.message_seq.is_some() || self.session_id.is_some() {
            return None;
        }
        let run_ordinal = self.run_ordinal?;
        let record_offset = self.record_offset?;
        // Bindings are numbered from one, so ordinal zero is not a run this
        // catalog can produce.
        (run_ordinal >= 1).then_some(ProductTraceCursorPosition {
            run_ordinal,
            record_offset,
        })
    }
}

impl ProductSessionCursor {
    /// Build the cursor that a client should send to resume after `session`.
    pub fn after(archived_rank: i64, updated_at: &str, session_id: ProductSessionId) -> Self {
        Self {
            archived_rank,
            updated_at: updated_at.to_string(),
            session_id,
        }
    }

    /// Render the cursor as a URL-safe token.
    ///
    /// Padding is omitted so the token needs no escaping in a query string.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a cursor is always serializable");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    /// Recover a cursor from a client-supplied token.
    ///
    /// Every failure mode is a rejection rather than a silent fallback to the
    /// first page: a client that sends a broken cursor and receives page one
    /// would read the whole list again and never learn why.
    pub fn decode(encoded: &str) -> Result<Self, ProductCursorError> {
        if encoded.len() > MAX_ENCODED_CURSOR_BYTES {
            return Err(ProductCursorError::TooLong);
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| ProductCursorError::NotBase64)?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| ProductCursorError::NotACursor)?;
        if cursor.archived_rank != SESSION_RANK_LIVE
            && cursor.archived_rank != SESSION_RANK_ARCHIVED
        {
            return Err(ProductCursorError::OutOfRange);
        }
        if cursor.updated_at.is_empty() || cursor.updated_at.len() > MAX_CURSOR_TIMESTAMP_BYTES {
            return Err(ProductCursorError::OutOfRange);
        }
        Ok(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProductSessionCursor {
        ProductSessionCursor::after(
            SESSION_RANK_LIVE,
            "2026-08-26T10:00:00.000000000+00:00",
            ProductSessionId::new(),
        )
    }

    #[test]
    fn a_cursor_survives_a_round_trip() {
        let cursor = sample();
        let decoded = ProductSessionCursor::decode(&cursor.encode()).unwrap();
        assert_eq!(decoded, cursor);
    }

    #[test]
    fn an_encoded_cursor_is_safe_to_put_in_a_query_string() {
        // Anything outside this set would need percent-encoding, and a client
        // that echoed the token verbatim would then send a different string.
        let encoded = sample().encode();
        assert!(
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
            "cursor must be URL-safe and unpadded, got {encoded}"
        );
    }

    #[test]
    fn a_cursor_does_not_leak_the_sort_key_in_plain_text() {
        // The point of encoding is that clients cannot come to depend on the
        // column names. If the token contained them, they would.
        let encoded = sample().encode();
        assert!(!encoded.contains("updated_at"));
        assert!(!encoded.contains("archived"));
    }

    #[test]
    fn every_malformed_cursor_is_refused_rather_than_treated_as_the_first_page() {
        assert_eq!(
            ProductSessionCursor::decode(&"A".repeat(MAX_ENCODED_CURSOR_BYTES + 1)),
            Err(ProductCursorError::TooLong)
        );
        assert_eq!(
            ProductSessionCursor::decode("not base64!!"),
            Err(ProductCursorError::NotBase64)
        );
        let not_json = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{oops");
        assert_eq!(
            ProductSessionCursor::decode(&not_json),
            Err(ProductCursorError::NotACursor)
        );
    }

    #[test]
    fn a_cursor_with_an_unknown_field_is_refused() {
        // `deny_unknown_fields` is what stops a future cursor version from
        // being silently reinterpreted by an older build as this version.
        let extra = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            br#"{"r":0,"u":"2026-08-26T10:00:00Z","i":"01J0000000000000000000000A","x":1}"#,
        );
        assert_eq!(
            ProductSessionCursor::decode(&extra),
            Err(ProductCursorError::NotACursor)
        );
    }

    #[test]
    fn a_rank_outside_the_listing_order_is_refused() {
        // Ranks are produced by a CASE expression that yields only 0 or 1. A
        // cursor claiming 2 would page past every row and return nothing,
        // which is worse than an error because it looks like an empty list.
        let mut cursor = sample();
        cursor.archived_rank = 2;
        assert_eq!(
            ProductSessionCursor::decode(&cursor.encode()),
            Err(ProductCursorError::OutOfRange)
        );
        cursor.archived_rank = -1;
        assert_eq!(
            ProductSessionCursor::decode(&cursor.encode()),
            Err(ProductCursorError::OutOfRange)
        );
    }

    #[test]
    fn an_absent_or_oversized_timestamp_is_refused() {
        let mut cursor = sample();
        cursor.updated_at = String::new();
        assert_eq!(
            ProductSessionCursor::decode(&cursor.encode()),
            Err(ProductCursorError::OutOfRange)
        );
        cursor.updated_at = "9".repeat(MAX_CURSOR_TIMESTAMP_BYTES + 1);
        assert_eq!(
            ProductSessionCursor::decode(&cursor.encode()),
            Err(ProductCursorError::OutOfRange)
        );
    }

    #[test]
    fn a_search_cursor_survives_a_round_trip_and_stays_url_safe() {
        let cursor = ProductMessageSearchCursor::after(7, "sha256:abc");
        let decoded = ProductMessageSearchCursor::decode(&cursor.encode()).unwrap();
        assert_eq!(decoded, cursor);
        let encoded = cursor.encode();
        assert!(
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
            "cursor must be URL-safe and unpadded, got {encoded}"
        );
    }

    #[test]
    fn every_malformed_search_cursor_is_refused() {
        assert_eq!(
            ProductMessageSearchCursor::decode(&"A".repeat(MAX_ENCODED_CURSOR_BYTES + 1)),
            Err(ProductCursorError::TooLong)
        );
        assert_eq!(
            ProductMessageSearchCursor::decode("not base64!!"),
            Err(ProductCursorError::NotBase64)
        );
        let not_json = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{oops");
        assert_eq!(
            ProductMessageSearchCursor::decode(&not_json),
            Err(ProductCursorError::NotACursor)
        );
        // `deny_unknown_fields`, so a future cursor shape is never silently
        // reinterpreted as this one.
        let extra = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"s":1,"h":"sha256:abc","x":1}"#);
        assert_eq!(
            ProductMessageSearchCursor::decode(&extra),
            Err(ProductCursorError::NotACursor)
        );
    }

    #[test]
    fn a_search_cursor_the_ledger_could_not_have_produced_is_refused() {
        for cursor in [
            ProductMessageSearchCursor::after(0, "sha256:abc"),
            ProductMessageSearchCursor::after(-1, "sha256:abc"),
            ProductMessageSearchCursor::after(1, ""),
            ProductMessageSearchCursor::after(1, &"9".repeat(MAX_CURSOR_DIGEST_BYTES + 1)),
        ] {
            assert_eq!(
                ProductMessageSearchCursor::decode(&cursor.encode()),
                Err(ProductCursorError::OutOfRange)
            );
        }
    }

    #[test]
    fn both_unified_search_positions_survive_a_round_trip_and_stay_url_safe() {
        let session_id = ProductSessionId::new();
        for cursor in [
            ProductSearchCursor::after_message(Some(session_id.clone()), 7, "sha256:abc"),
            ProductSearchCursor::after_message(None, 7, "sha256:abc"),
            ProductSearchCursor::after_trace(3, 4096, "sha256:abc"),
        ] {
            let encoded = cursor.encode();
            assert_eq!(ProductSearchCursor::decode(&encoded).unwrap(), cursor);
            assert!(
                encoded
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
                "cursor must be URL-safe and unpadded, got {encoded}"
            );
        }
    }

    #[test]
    fn a_unified_search_cursor_describes_exactly_one_position() {
        let session_id = ProductSessionId::new();
        let message = ProductSearchCursor::after_message(Some(session_id.clone()), 7, "sha256:abc");
        assert_eq!(
            message.message_position(),
            Some((Some(&session_id), 7)),
            "a workspace message cursor carries its session"
        );
        assert_eq!(message.trace_position(), None);
        let session_only = ProductSearchCursor::after_message(None, 7, "sha256:abc");
        assert_eq!(session_only.message_position(), Some((None, 7)));
        let trace = ProductSearchCursor::after_trace(3, 4096, "sha256:abc");
        assert_eq!(trace.message_position(), None);
        assert_eq!(
            trace.trace_position(),
            Some(ProductTraceCursorPosition {
                run_ordinal: 3,
                record_offset: 4096,
            })
        );
    }

    #[test]
    fn a_unified_search_cursor_with_half_a_position_or_two_is_refused() {
        let session_id = ProductSessionId::new();
        // A cursor is only ever minted complete. Anything else is either a
        // forged token or one from a shape this build does not know, and
        // answering it with page one would hide the mistake.
        for json in [
            // No position at all.
            r#"{"h":"sha256:abc"}"#.to_string(),
            // A session with no sequence: the workspace position is incomplete.
            format!(r#"{{"i":"{session_id}","h":"sha256:abc"}}"#),
            // Both positions at once.
            format!(r#"{{"m":7,"i":"{session_id}","r":3,"o":0,"h":"sha256:abc"}}"#),
            // Half a trace position.
            r#"{"r":3,"h":"sha256:abc"}"#.to_string(),
            r#"{"o":4096,"h":"sha256:abc"}"#.to_string(),
            // A run ordinal the binding order cannot produce, and a message
            // sequence the ledger cannot produce.
            r#"{"r":0,"o":0,"h":"sha256:abc"}"#.to_string(),
            r#"{"m":0,"h":"sha256:abc"}"#.to_string(),
            r#"{"m":-1,"h":"sha256:abc"}"#.to_string(),
        ] {
            let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());
            assert_eq!(
                ProductSearchCursor::decode(&encoded),
                Err(ProductCursorError::OutOfRange),
                "{json} should have been refused"
            );
        }
        // `deny_unknown_fields`, like every other cursor here.
        let extra = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"m":1,"h":"sha256:abc","x":1}"#);
        assert_eq!(
            ProductSearchCursor::decode(&extra),
            Err(ProductCursorError::NotACursor)
        );
    }
}
