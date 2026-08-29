//! Opaque pagination cursors for the product session listing.
//!
//! Codex alignment Phase 7. The listing is ordered by a three-part key — live
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

use super::ProductSessionId;

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
}
