//! Single-session message search: the two index paths and the bounded snippet
//! shown for each hit.
//!
//! Design `2026-09-26-runtime-contract-alignment-design.md` §7.2 step one. The
//! first step is deliberately session-scoped: the same storage and the same
//! authorization as the message ledger, no global cross-session query, and no
//! trace-event content.
//!
//! Two query paths share one result shape, because the FTS5 `trigram`
//! tokenizer cannot match a term shorter than three characters (probed, see the
//! design record §7.4):
//!
//! - three characters or more: `product_messages_fts MATCH`, index-backed;
//! - one or two characters: a `LIKE` scan bounded by the session id, which is
//!   the fallback path the design already allows. Its cost is linear in *that
//!   session's* ledger and nothing caps that ledger — the store caps the
//!   pending queue (`MAX_PENDING_MESSAGES_PER_SESSION`), not the messages a
//!   session has already accumulated, and the `trigram` index cannot serve a
//!   term this short. Measured on the bundled build: eight VM steps per ledger
//!   row, ~52 ms of SQL for a term whose only hit is the last of 22,000
//!   messages (8.98 MB of bodies), ~2 ms for the same shape at 1,000 messages.
//!   A posting index over the one- and two-character space would add roughly
//!   15 bytes of index per byte of body (~2.6x the whole store at that size),
//!   which is not a proportionate price for those milliseconds; the design
//!   record's §7.4 deviation 1 carries the numbers and that decision.
//!
//!   What the fallback does guarantee is that it never leaves the session it
//!   was asked about — the property the workspace scope's typed 400 rests on —
//!   and `search_tests::a_short_term_search_never_leaves_its_session` pins it
//!   against the plan and the step count.
//!
//! Both paths order by the ledger's `seq` and page by keyset on that column.

use rusqlite::params;

use crate::{
    MAX_PRODUCT_MESSAGE_SEARCH_LIMIT, MAX_PRODUCT_MESSAGE_SEARCH_QUERY_BYTES,
    MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS, MAX_PRODUCT_SEARCH_LIMIT,
    ProductMessageSearchCursor, ProductMessageSearchHit, ProductMessageSearchPage,
    ProductMessageSearchQuery, ProductMessageSearchScope, ProductSearchCursor, ProductSearchHit,
    ProductSearchPage, ProductSearchQuery, ProductSearchSource, ProductSessionId,
    ProductStoreError, ProductWorkspaceId, is_unrepresentable_query_character,
};

use super::schema::storage_error;
use super::validation::invalid;

/// Smallest term the FTS5 `trigram` tokenizer can match.
///
/// Probed against the bundled build: `MATCH` of a one- or two-character term
/// returns no rows for Chinese and English alike, while three characters
/// match. Shorter terms therefore take the `LIKE` path.
pub(super) const MIN_TRIGRAM_TERM_CHARACTERS: usize = 3;

/// The one- and two-character fallback, verbatim.
///
/// It is a constant rather than an inline literal because a test asserts the
/// plan and the VM steps *this* statement takes (that it walks the session and
/// nothing else); measuring a copy would let the shipped query drift away from
/// its evidence.
pub(super) const SHORT_TERM_SEARCH_SQL: &str = r#"
            SELECT seq, content, created_at
            FROM product_session_controls
            WHERE product_session_id = ?1
              AND message_contract_version = 1
              AND content LIKE ?2 ESCAPE '\'
              AND seq > ?3
            ORDER BY seq ASC
            LIMIT ?4
            "#;

/// Escape a term for use as an FTS5 string literal.
///
/// A `MATCH` operand is parsed as an FTS5 *query expression*, where `OR`, `-`,
/// `*`, `(`, `NEAR`, and `"` all carry meaning. Binding the raw term would let
/// a user's prose change the query's shape — and an unbalanced quote is a
/// syntax error rather than a no-match. Quoting the whole term makes it a
/// single phrase, which the trigram tokenizer evaluates as a substring match,
/// and doubling embedded quotes is FTS5's own escape.
pub(super) fn fts_phrase(term: &str) -> String {
    let mut phrase = String::with_capacity(term.len() + 2);
    phrase.push('"');
    for character in term.chars() {
        if character == '"' {
            phrase.push('"');
        }
        phrase.push(character);
    }
    phrase.push('"');
    phrase
}

/// The digest a search cursor is bound to: the session **and** the term.
///
/// Binding the term alone leaves the session free, and a keyset position is
/// only meaningful inside the session it was minted in: a cursor from another
/// session is a valid `seq` with a matching term, so it would silently skip
/// every earlier hit of *this* session and answer `hits: []` with no
/// `next_cursor` — the client would read "no match" while the hits exist. The
/// unit separator keeps the two fields unambiguous even if a session id or a
/// term ever contains the other's bytes.
pub(super) fn cursor_digest(session_id: &ProductSessionId, term: &str) -> String {
    rove_runtime::context::stable_hash(&format!("{session_id}\u{1f}{term}"))
}

impl super::repository::ProductRepository {
    pub(super) fn search_messages(
        &self,
        session_id: &ProductSessionId,
        query: &ProductMessageSearchQuery,
    ) -> Result<ProductMessageSearchPage, ProductStoreError> {
        validate_search_query(query)?;
        let mut connection = self.database.connect()?;
        // A plain read transaction, like the message listing: the search is
        // read-only and must not take the store's immediate write lock.
        let transaction = connection.transaction().map_err(storage_error)?;
        // The session check comes first and is the only authorization this
        // endpoint needs: a session that is not in the catalog is not found,
        // and no query runs for it.
        super::repository::get_session(&transaction, session_id)?;
        if let Some(cursor) = &query.cursor
            && cursor.query_digest != cursor_digest(session_id, &query.term)
        {
            return Err(invalid(
                "search cursor was issued for a different session or search term",
            ));
        }
        let rows = fetch_hits(
            &transaction,
            session_id,
            &query.term,
            query.cursor.as_ref().map(|cursor| cursor.message_seq),
            query.limit,
        )?;
        transaction.commit().map_err(storage_error)?;

        let mut hits: Vec<ProductMessageSearchHit> = rows
            .into_iter()
            .map(|row| ProductMessageSearchHit {
                message_seq: row.seq,
                snippet: bounded_snippet(&row.content, &query.term),
                created_at: row.created_at,
            })
            .collect();
        // Fetching one extra row is how the page learns there is a next one
        // without a second COUNT query.
        let has_more = hits.len() > query.limit;
        if has_more {
            hits.pop();
        }
        let next_cursor = has_more.then(|| {
            ProductMessageSearchCursor::after(
                hits.last().map(|hit| hit.message_seq).unwrap_or_default(),
                &cursor_digest(session_id, &query.term),
            )
        });
        Ok(ProductMessageSearchPage { hits, next_cursor })
    }

    /// One page of the unified search over the message ledger.
    ///
    /// Both message scopes walk the same ledger and the same FTS5 index as
    /// [`Self::search_messages`]; the difference is the key. The session scope
    /// keeps the per-session `seq` keyset, and the workspace scope keys on
    /// `(session_id, seq)` so the hits of one session stay contiguous and a
    /// single page answers "which sessions contain this term" without a second
    /// query. The foreign key on `product_session_controls` means no separate
    /// session association has to be maintained for either key, which is why
    /// this costs no migration.
    pub(super) fn search_scoped_messages(
        &self,
        scope: &ProductMessageSearchScope,
        query: &ProductSearchQuery,
    ) -> Result<ProductSearchPage, ProductStoreError> {
        validate_scoped_search_query(scope, query)?;
        let digest = scope.cursor_digest(&query.term);
        let after = resolve_message_cursor(scope, query.cursor.as_ref(), &digest)?;
        let mut connection = self.database.connect()?;
        // Read-only, like the session-scoped search: it must not take the
        // store's immediate write lock.
        let transaction = connection.transaction().map_err(storage_error)?;
        let mut hits = match scope {
            ProductMessageSearchScope::Session(session_id) => {
                // The session check comes first and is the whole authorization:
                // an unknown or deleted session is not found, and no query runs
                // for it.
                super::repository::get_session(&transaction, session_id)?;
                fetch_hits(
                    &transaction,
                    session_id,
                    &query.term,
                    after.map(|(_, seq)| seq),
                    query.limit,
                )?
                .into_iter()
                .map(|row| message_hit(session_id.clone(), row, &query.term))
                .collect::<Vec<_>>()
            }
            ProductMessageSearchScope::Workspace(workspace_id) => {
                super::repository::get_workspace(&transaction, workspace_id)?;
                fetch_workspace_hits(&transaction, workspace_id, query, after)?
                    .into_iter()
                    .map(|row| {
                        let session_id = row.session_id.clone();
                        message_hit(session_id, row.into(), &query.term)
                    })
                    .collect::<Vec<_>>()
            }
        };
        transaction.commit().map_err(storage_error)?;

        let has_more = hits.len() > query.limit;
        if has_more {
            hits.pop();
        }
        let next_cursor = has_more.then(|| {
            let last = hits.last().expect("a page with more rows is not empty");
            let session_id = match scope {
                ProductMessageSearchScope::Session(_) => None,
                ProductMessageSearchScope::Workspace(_) => Some(last.session_id.clone()),
            };
            ProductSearchCursor::after_message(session_id, last.seq, &digest)
        });
        Ok(ProductSearchPage { hits, next_cursor })
    }
}

/// One message hit, named by the session that owns it.
fn message_hit(session_id: ProductSessionId, row: HitRow, term: &str) -> ProductSearchHit {
    ProductSearchHit {
        session_id,
        source: ProductSearchSource::Message,
        seq: row.seq,
        run_id: None,
        run_ordinal: None,
        snippet: bounded_snippet(&row.content, term),
        created_at: Some(row.created_at),
    }
}

/// The resume position of a message cursor, checked against its scope.
///
/// The digest is what refuses a cursor minted for another scope or another
/// term. The shape is checked separately, because a token can only ever be
/// minted in the shape its scope uses, and a page that accepted the other shape
/// would read it as "no position" and silently answer page one.
fn resolve_message_cursor(
    scope: &ProductMessageSearchScope,
    cursor: Option<&ProductSearchCursor>,
    digest: &str,
) -> Result<Option<(Option<ProductSessionId>, i64)>, ProductStoreError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    if cursor.query_digest != digest {
        return Err(invalid(
            "search cursor was issued for a different scope or search term",
        ));
    }
    let Some((session_id, seq)) = cursor.message_position() else {
        return Err(invalid("search cursor does not hold a message position"));
    };
    match scope {
        ProductMessageSearchScope::Session(_) if session_id.is_some() => {
            Err(invalid("search cursor was issued for a different scope"))
        }
        ProductMessageSearchScope::Workspace(_) if session_id.is_none() => {
            Err(invalid("search cursor was issued for a different scope"))
        }
        _ => Ok(Some((session_id.cloned(), seq))),
    }
}

struct HitRow {
    seq: i64,
    content: String,
    created_at: String,
}

/// A hit row that also carries the session it belongs to.
struct ScopedHitRow {
    session_id: ProductSessionId,
    seq: i64,
    content: String,
    created_at: String,
}

impl From<ScopedHitRow> for HitRow {
    fn from(row: ScopedHitRow) -> Self {
        Self {
            seq: row.seq,
            content: row.content,
            created_at: row.created_at,
        }
    }
}

/// The workspace-scope page query.
///
/// Only the indexed path is reachable here: the fallback `LIKE` scan is bounded
/// by one session, so across a workspace it would leave the workspace's ledger
/// and scan the whole store. `validate_scoped_search_query` refuses a term
/// below the trigram floor for this scope rather than letting the caller think
/// an empty page means "no match".
///
/// The cost is proportional to the matches the index finds inside the
/// workspace, not to the size of the ledger: `MATCH` drives the join, the
/// session membership is an `IN` over the workspace's own sessions, and the
/// bound is applied before the sort that gives the page its order.
fn fetch_workspace_hits(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &ProductWorkspaceId,
    query: &ProductSearchQuery,
    after: Option<(Option<ProductSessionId>, i64)>,
) -> Result<Vec<ScopedHitRow>, ProductStoreError> {
    let limit = i64::try_from(query.limit + 1).map_err(storage_error)?;
    // No cursor is an empty key, not a fabricated one: every product session id
    // is a non-empty ULID, so `product_session_id > ''` holds for every row and
    // the `seq > 0` arm is never reached. Inventing an id here would silently
    // drop every session that sorts before it.
    let after_session = after
        .as_ref()
        .and_then(|(session_id, _)| session_id.as_ref())
        .map(ProductSessionId::to_string)
        .unwrap_or_default();
    let after_seq = after.map(|(_, seq)| seq).unwrap_or(0);
    let mut statement = transaction
        .prepare(
            r#"
            SELECT c.product_session_id, c.seq, c.content, c.created_at
            FROM product_messages_fts
            JOIN product_session_controls c
                ON c.rowid = product_messages_fts.rowid
            WHERE product_messages_fts MATCH ?1
              AND c.message_contract_version = 1
              AND c.product_session_id IN (
                  SELECT product_session_id FROM product_sessions WHERE workspace_id = ?2
              )
              AND (c.product_session_id > ?3
                   OR (c.product_session_id = ?3 AND c.seq > ?4))
            ORDER BY c.product_session_id ASC, c.seq ASC
            LIMIT ?5
            "#,
        )
        .map_err(storage_error)?;
    let mapped = statement
        .query_map(
            params![
                fts_phrase(&query.term),
                workspace_id.to_string(),
                after_session,
                after_seq,
                limit
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(storage_error)?;
    let mut rows = Vec::new();
    for row in mapped {
        let (session_id, seq, content, created_at) = row.map_err(storage_error)?;
        rows.push(ScopedHitRow {
            session_id: super::repository::parse_product_id(&session_id, "product session id")?,
            seq,
            content,
            created_at,
        });
    }
    Ok(rows)
}

/// Run whichever query path the term length selects, oldest hit first.
fn fetch_hits(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &ProductSessionId,
    term: &str,
    after_seq: Option<i64>,
    limit: usize,
) -> Result<Vec<HitRow>, ProductStoreError> {
    let limit = i64::try_from(limit + 1).map_err(storage_error)?;
    let mut rows = Vec::new();
    if term.chars().count() >= MIN_TRIGRAM_TERM_CHARACTERS {
        let mut statement = transaction
            .prepare(
                r#"
                SELECT c.seq, c.content, c.created_at
                FROM product_messages_fts
                JOIN product_session_controls c
                    ON c.rowid = product_messages_fts.rowid
                WHERE product_messages_fts MATCH ?1
                  AND c.product_session_id = ?2
                  AND c.message_contract_version = 1
                  AND c.seq > ?3
                ORDER BY c.seq ASC
                LIMIT ?4
                "#,
            )
            .map_err(storage_error)?;
        let mapped = statement
            .query_map(
                params![
                    fts_phrase(term),
                    session_id.to_string(),
                    after_seq.unwrap_or(0),
                    limit
                ],
                row_to_hit,
            )
            .map_err(storage_error)?;
        for row in mapped {
            rows.push(row.map_err(storage_error)?);
        }
        return Ok(rows);
    }
    let mut statement = transaction
        .prepare(SHORT_TERM_SEARCH_SQL)
        .map_err(storage_error)?;
    let mapped = statement
        .query_map(
            params![
                session_id.to_string(),
                super::repository::like_pattern(term),
                after_seq.unwrap_or(0),
                limit
            ],
            row_to_hit,
        )
        .map_err(storage_error)?;
    for row in mapped {
        rows.push(row.map_err(storage_error)?);
    }
    Ok(rows)
}

fn row_to_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<HitRow> {
    Ok(HitRow {
        seq: row.get(0)?,
        content: row.get(1)?,
        created_at: row.get(2)?,
    })
}

/// Validate a resolved search request before any SQL runs.
///
/// The route rejects the same inputs with the same typed error; this second
/// check exists because `ProductStore` is a public trait surface and must not
/// depend on one caller having validated its arguments.
fn validate_search_query(query: &ProductMessageSearchQuery) -> Result<(), ProductStoreError> {
    if query.term.trim().is_empty()
        || query.term.len() > MAX_PRODUCT_MESSAGE_SEARCH_QUERY_BYTES
        || query.term.chars().any(is_unrepresentable_query_character)
    {
        return Err(invalid("message search query is invalid"));
    }
    if query.limit == 0 || query.limit > MAX_PRODUCT_MESSAGE_SEARCH_LIMIT {
        return Err(invalid("message search page limit is invalid"));
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.message_seq < 1)
    {
        return Err(invalid("message search cursor is invalid"));
    }
    Ok(())
}

/// Validate a unified search request before any SQL runs.
///
/// The route rejects the same inputs with the same typed errors; this second
/// check exists because `ProductStore` is a public trait surface and must not
/// depend on one caller having validated its arguments.
fn validate_scoped_search_query(
    scope: &ProductMessageSearchScope,
    query: &ProductSearchQuery,
) -> Result<(), ProductStoreError> {
    if query.term.trim().is_empty()
        || query.term.len() > MAX_PRODUCT_MESSAGE_SEARCH_QUERY_BYTES
        || query.term.chars().any(is_unrepresentable_query_character)
    {
        return Err(invalid("product search query is invalid"));
    }
    if query.limit == 0 || query.limit > MAX_PRODUCT_SEARCH_LIMIT {
        return Err(invalid("product search page limit is invalid"));
    }
    // The trigram index is what keeps a workspace-wide search inside the
    // workspace. The fallback `LIKE` scan is bounded by a session id, so across
    // a workspace it would scan every control row in the store; refusing the
    // term is the honest answer, and the session scope still serves short terms.
    if matches!(scope, ProductMessageSearchScope::Workspace(_))
        && query.term.chars().count() < MIN_TRIGRAM_TERM_CHARACTERS
    {
        return Err(invalid(
            "workspace search requires a term of at least three characters; \
             search one session for shorter terms",
        ));
    }
    Ok(())
}

/// Build the excerpt shown for one hit.
///
/// Two properties matter, and both are why this is not FTS5's `snippet()`:
///
/// 1. **The body is redacted before it is windowed.** Two passes run over the
///    whole body first: the authoritative registry removes values the process
///    was told are credentials, and the text-level pattern rule the evidence
///    export uses (`redact_secret_patterns`) runs as the backstop. Windowing
///    first would be a hole: a window starting inside `sk-ant-…` would carry
///    the token without its prefix, and no prefix-based rule could catch it
///    afterwards.
/// 2. **The result is bounded in codepoints, not tokens.** `snippet()` counts
///    tokens, and one token can be a very long run of text with no separators,
///    so a token budget is not a size budget. A codepoint budget cannot be
///    defeated that way.
///
/// Residual risk, recorded in the design record §7.4 and in the security
/// checklist of the runtime documents: a secret the process was never told
/// about *and* that matches no known pattern is the user's own text in the
/// user's own session, which the transcript already shows in full; the bounded
/// snippet does not widen that exposure, and it never leaves the window around
/// the hit. A credential the runtime resolved (a provider key from a file or
/// keyring, an MCP environment value the configuration declared secret) is no
/// longer in that residual class: the registry pass removes it on read as well
/// as on write.
pub fn bounded_snippet(content: &str, term: &str) -> String {
    let authoritative = rove_runtime::secrets::registry().redact_text(content);
    let (redacted, _) = crate::secret_patterns::redact_secret_patterns(authoritative);
    let characters: Vec<char> = redacted.chars().collect();
    if characters.len() <= MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS {
        return redacted;
    }
    let (hit_start, hit_length) = find_ascii_case_insensitive(&characters, term).unwrap_or((0, 0));
    // Reserve the two ellipsis codepoints up front: the body is known to be
    // longer than the budget here, so at least one side is always truncated.
    let budget = MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS - 2;
    let shown_hit = hit_length.min(budget);
    let remaining = budget - shown_hit;
    let lead = remaining / 2;
    let trail = remaining - lead;
    let start = hit_start.saturating_sub(lead);
    let end = (hit_start + shown_hit + trail).min(characters.len());
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.extend(&characters[start..end]);
    if end < characters.len() {
        snippet.push('…');
    }
    // Backstop. The arithmetic above stays inside the budget, but the bound is
    // a security property, so it is enforced on the value that is returned
    // rather than trusted to the derivation.
    truncate_codepoints(snippet, MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS)
}

/// Keep at most `limit` codepoints of `value`.
fn truncate_codepoints(value: String, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value;
    }
    value.chars().take(limit).collect()
}

/// Locate `needle` in `haystack`, folding ASCII case like the tokenizers do.
///
/// The FTS5 trigram tokenizer and SQLite's `LIKE` both fold ASCII case, so this
/// finds the hit they matched. A term that folds differently outside ASCII
/// simply is not found here, and the snippet falls back to a bounded window
/// from the start of the message rather than showing nothing.
fn find_ascii_case_insensitive(haystack: &[char], needle: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = needle.chars().map(|c| c.to_ascii_lowercase()).collect();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find_map(|start| {
        haystack[start..start + needle.len()]
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .eq(needle.iter().copied())
            .then_some((start, needle.len()))
    })
}
