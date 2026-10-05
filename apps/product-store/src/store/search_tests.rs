//! Store-level tests for single-session message search (design §7.2 step one).
//!
//! These run against the real SQLite store, so they exercise the actual FTS5
//! `trigram` index and the `LIKE` fallback rather than a stand-in for them. The
//! tokenizer's own limits are asserted here too: a test that assumed a
//! two-character `MATCH` worked would pass on a machine with a different SQLite
//! build and fail in production.

use std::fs;

use rusqlite::Connection;
use tempfile::TempDir;

use crate::{
    CreateProductMessageRequest, CreateProductSessionRequest, CreateProductWorkspaceRequest,
    MAX_PRODUCT_MESSAGE_SEARCH_LIMIT, MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
    MAX_PRODUCT_SEARCH_LIMIT, ProductErrorCode, ProductMessageSearchQuery,
    ProductMessageSearchScope, ProductSearchCursor, ProductSearchQuery, ProductSessionId,
    ProductStore, ProductWorkspaceId, ProductWorkspaceKind,
};

use super::SqliteProductStore;
use super::search::{SHORT_TERM_SEARCH_SQL, bounded_snippet, cursor_digest};

fn open_store(temp: &TempDir) -> SqliteProductStore {
    SqliteProductStore::open(temp.path().join("product.sqlite"), 5_000).unwrap()
}

async fn create_session(store: &SqliteProductStore, temp: &TempDir) -> ProductSessionId {
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    let workspace = store
        .create_workspace(CreateProductWorkspaceRequest {
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: Some("Search workspace".to_string()),
            pinned: false,
        })
        .await
        .unwrap();
    store
        .create_session(CreateProductSessionRequest {
            workspace_id: workspace.id,
            title: Some("Search session".to_string()),
        })
        .await
        .unwrap()
        .id
}

async fn send(store: &SqliteProductStore, session_id: &ProductSessionId, content: &str) -> i64 {
    store
        .create_message(
            session_id,
            CreateProductMessageRequest {
                content: content.to_string(),
                idempotency_key: None,
                attachments: Vec::new(),
            },
        )
        .await
        .unwrap()
        .0
        .seq
}

fn query(
    session_id: &ProductSessionId,
    term: &str,
    cursor: Option<i64>,
    limit: usize,
) -> ProductMessageSearchQuery {
    ProductMessageSearchQuery {
        term: term.to_string(),
        cursor: cursor.map(|seq| {
            crate::ProductMessageSearchCursor::after(seq, &cursor_digest(session_id, term))
        }),
        limit,
    }
}

async fn search(
    store: &SqliteProductStore,
    session_id: &ProductSessionId,
    term: &str,
) -> Vec<(i64, String)> {
    store
        .search_messages(session_id, query(session_id, term, None, 32))
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| (hit.message_seq, hit.snippet))
        .collect()
}

/// Count index entries matching a phrase through the same quoting the query
/// path uses, so the assertion is about the shipped behaviour.
fn index_match_count(temp: &TempDir, term: &str) -> i64 {
    Connection::open(temp.path().join("product.sqlite"))
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM product_messages_fts WHERE product_messages_fts MATCH ?1",
            [format!("\"{}\"", term.replace('"', "\"\""))],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn chinese_and_english_messages_are_both_findable() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;
    let chinese_body = "我们讨论了运行时合同的搜索能力";
    let english_body = "Runtime contract alignment for message search";
    let chinese = send(&store, &session, chinese_body).await;
    let english = send(&store, &session, english_body).await;
    let other = send(&store, &session, "unrelated note").await;

    // Three characters reach the trigram index; two characters fall back to the
    // bounded scan. Both must find the Chinese message, and the whole body is
    // short enough to be returned verbatim.
    assert_eq!(
        search(&store, &session, "运行时合同").await,
        vec![(chinese, chinese_body.to_string())]
    );
    assert_eq!(
        search(&store, &session, "合同").await,
        vec![(chinese, chinese_body.to_string())]
    );
    assert_eq!(
        search(&store, &session, "alignment").await,
        vec![(english, english_body.to_string())]
    );
    // A one-character term is the shortest the fallback path accepts, and it
    // must not match the unrelated message.
    assert_eq!(
        search(&store, &session, "运")
            .await
            .into_iter()
            .map(|hit| hit.0)
            .collect::<Vec<_>>(),
        vec![chinese]
    );
    assert!(
        search(&store, &session, "现在时").await.is_empty(),
        "a term nothing contains must return an empty page, not an error"
    );
    assert_eq!(
        search(&store, &session, "unrelated").await[0].0,
        other,
        "search must not confuse neighbouring messages"
    );
}

#[tokio::test]
async fn search_folds_ascii_case_the_way_both_query_paths_do() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;
    let seq = send(&store, &session, "A CONTRACT in UPPER case").await;
    let short_seq = send(&store, &session, "MiXeD CoNtRaCt").await;

    // The trigram index path folds case in both directions.
    for term in ["CONTRACT", "contract", "CoNtRaCt"] {
        assert!(
            search(&store, &session, term)
                .await
                .iter()
                .any(|hit| hit.0 == seq),
            "{term} must match an upper-case body through the index"
        );
    }
    // So does the two-character `LIKE` path.
    for term in ["co", "CO", "Co"] {
        assert!(
            search(&store, &session, term)
                .await
                .iter()
                .any(|hit| hit.0 == short_seq),
            "{term} must match a mixed-case body through the fallback scan"
        );
    }
}

#[tokio::test]
async fn a_search_page_walks_the_hits_once_with_cursor_pagination() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;
    let mut written = Vec::new();
    for index in 0..7 {
        written.push(send(&store, &session, &format!("pageable message {index}")).await);
    }
    // A message that must never appear in these pages.
    let excluded = send(&store, &session, "different topic entirely").await;

    let mut seen = Vec::new();
    let mut cursor: Option<i64> = None;
    let mut pages = 0;
    loop {
        let page = store
            .search_messages(&session, query(&session, "pageable", cursor, 3))
            .await
            .unwrap();
        pages += 1;
        assert!(page.hits.len() <= 3);
        seen.extend(page.hits.iter().map(|hit| hit.message_seq));
        match page.next_cursor {
            Some(next) => cursor = Some(next.message_seq),
            None => break,
        }
        assert!(pages < 10, "pagination must terminate");
    }

    assert_eq!(pages, 3, "seven hits at three per page is three pages");
    assert_eq!(seen, written, "the walk must return every hit, in order");
    assert!(!seen.contains(&excluded));
    // The cursor is a pure resume key: the term is part of it, so a token from
    // a different search is refused instead of silently mis-paging.
    let mismatched = ProductMessageSearchQuery {
        term: "different".to_string(),
        cursor: Some(crate::ProductMessageSearchCursor::after(
            written[0],
            &cursor_digest(&session, "pageable"),
        )),
        limit: 3,
    };
    let error = store
        .search_messages(&session, mismatched)
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductInvalidInput);

    // The session is part of the key too. The same term and the same `seq` in
    // another session must not be accepted: `seq` 4 of session B exists, so
    // without the session in the digest this request would answer with B's
    // hits from 5 on and read as "the earlier ones do not match" — or, when the
    // cursor skips past every hit, as an empty page.
    let other_session = create_session(&store, &temp).await;
    for index in 0..3 {
        send(&store, &other_session, &format!("pageable message {index}")).await;
    }
    let foreign_cursor = ProductMessageSearchQuery {
        term: "pageable".to_string(),
        cursor: Some(crate::ProductMessageSearchCursor::after(
            written[0],
            &cursor_digest(&session, "pageable"),
        )),
        limit: 3,
    };
    let error = store
        .search_messages(&other_session, foreign_cursor)
        .await
        .unwrap_err();
    assert_eq!(
        error.code,
        ProductErrorCode::ProductInvalidInput,
        "a cursor minted in another session must be refused, never answered with that session's hits"
    );
}

#[tokio::test]
async fn a_freshly_written_message_is_searchable_immediately() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;

    let seq = send(&store, &session, "a freshly written searchable message").await;

    assert_eq!(
        search(&store, &session, "freshly written").await,
        vec![(seq, "a freshly written searchable message".to_string())],
        "the index must be written in the message's own transaction"
    );
    assert_eq!(
        index_match_count(&temp, "freshly written"),
        1,
        "the ledger and the index must agree after one insert"
    );
}

#[tokio::test]
async fn a_deleted_message_stops_matching_in_the_index_and_the_ledger_alike() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;
    let kept = send(&store, &session, "keeper message about contracts").await;
    let doomed = send(&store, &session, "doomed message about contracts").await;

    assert_eq!(search(&store, &session, "contracts").await.len(), 2);

    // The only delete path in the product store is the foreign-key cascade from
    // a removed session, so delete a second session that holds the doomed row
    // through a direct statement: this exercises the same AFTER DELETE trigger
    // the cascade fires.
    Connection::open(temp.path().join("product.sqlite"))
        .unwrap()
        .execute(
            "DELETE FROM product_session_controls WHERE control_id = (
                 SELECT control_id FROM product_session_controls
                 WHERE product_session_id = ?1 AND seq = ?2
             )",
            rusqlite::params![session.to_string(), doomed],
        )
        .unwrap();

    let hits = search(&store, &session, "contracts").await;
    assert_eq!(
        hits.iter().map(|hit| hit.0).collect::<Vec<_>>(),
        vec![kept],
        "a deleted message must stop matching"
    );
    assert_eq!(
        index_match_count(&temp, "doomed message about contracts"),
        0,
        "the index must not keep an entry for a deleted message"
    );
    assert_eq!(
        index_match_count(&temp, "keeper message about contracts"),
        1,
        "deleting one message must leave the others indexed"
    );
}

#[tokio::test]
async fn deleting_a_session_cascades_the_removed_messages_out_of_the_index() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;
    send(&store, &session, "a cascade searchable sentence").await;
    assert_eq!(index_match_count(&temp, "cascade searchable"), 1);

    store.delete_session(&session).await.unwrap();

    assert_eq!(
        index_match_count(&temp, "cascade searchable"),
        0,
        "the cascade delete must remove the index entries with the rows"
    );
    let error = store
        .search_messages(&session, query(&session, "cascade", None, 10))
        .await
        .unwrap_err();
    assert_eq!(
        error.code,
        ProductErrorCode::ProductNotFound,
        "a removed session is not found, never an empty hit list"
    );
}

#[tokio::test]
async fn an_unknown_session_is_not_found_rather_than_empty() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let unknown = ProductSessionId::new();
    let error = store
        .search_messages(&unknown, query(&unknown, "anything", None, 10))
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductNotFound);
}

#[tokio::test]
async fn the_store_refuses_an_invalid_search_request() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;

    for (term, limit) in [
        (String::new(), 10),
        ("   ".to_string(), 10),
        ("x".repeat(129), 10),
        ("contract".to_string(), 0),
        ("contract".to_string(), MAX_PRODUCT_MESSAGE_SEARCH_LIMIT + 1),
    ] {
        let error = store
            .search_messages(&session, query(&session, &term, None, limit))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            ProductErrorCode::ProductInvalidInput,
            "term {term:?} with limit {limit} must be refused"
        );
    }
    // A cross-page limit is accepted: the page cap is the bound, not the default.
    let page = store
        .search_messages(
            &session,
            query(&session, "contract", None, MAX_PRODUCT_MESSAGE_SEARCH_LIMIT),
        )
        .await
        .unwrap();
    assert!(page.hits.is_empty());

    // A term holding the UTF-8 replacement character is refused here too: the
    // route rejects `q=%FF` for the same reason, and this is the surface a
    // second caller would reach with the lossy decode already done.
    let error = store
        .search_messages(&session, query(&session, "contract\u{FFFD}", None, 10))
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductInvalidInput);
}

#[test]
fn a_snippet_is_bounded_around_the_hit_and_never_carries_the_whole_message() {
    let needle = "needle";
    let content = format!("{}{}{}", "pre ".repeat(200), needle, " post".repeat(200));

    let snippet = bounded_snippet(&content, needle);

    assert!(snippet.contains(needle), "the snippet must show the hit");
    assert!(
        snippet.chars().count() <= MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
        "a snippet must stay inside its codepoint budget, got {}",
        snippet.chars().count()
    );
    assert!(
        snippet.chars().count() < content.chars().count(),
        "a long message must not be returned whole"
    );
    assert!(snippet.starts_with('…') && snippet.ends_with('…'));
}

#[test]
fn a_snippet_cannot_be_widened_by_a_long_unbroken_token() {
    // One "token" of several thousand characters: a token-count budget such as
    // FTS5's `snippet()` would return all of it.
    let content = format!("{}{}{}", "A".repeat(4_000), "needle", "B".repeat(4_000));

    let snippet = bounded_snippet(&content, "needle");

    assert!(snippet.contains("needle"));
    assert!(
        snippet.chars().count() <= MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
        "an unbroken token must not widen the snippet, got {}",
        snippet.chars().count()
    );
}

#[test]
fn a_snippet_shorter_than_the_budget_is_returned_verbatim() {
    let snippet = bounded_snippet("a short message", "short");
    assert_eq!(snippet, "a short message");
    assert!(!snippet.contains('…'));
}

#[test]
fn a_snippet_cannot_expose_a_secret_whose_prefix_falls_outside_the_window() {
    // The secret sits immediately before the hit and is far longer than the
    // window, so a snippet built by windowing first would start inside the
    // token and print its tail. Redacting the body first is what prevents it.
    let content = format!(
        "{}Authorization: Bearer sk-ant-{}{}合同讨论",
        "pre ".repeat(60),
        "y".repeat(400),
        " tail"
    );

    let snippet = bounded_snippet(&content, "合同");

    assert!(snippet.contains("合同"), "the hit itself is not a secret");
    assert!(
        snippet.contains("[REDACTED:secret_pattern]"),
        "the known secret pattern must be redacted in the snippet, got {snippet}"
    );
    assert!(
        !snippet.contains("sk-ant-"),
        "no fragment of the secret prefix may survive"
    );
    assert!(
        !snippet.contains('y'),
        "no fragment of the secret body may survive the window, got {snippet}"
    );
}

#[test]
fn a_secret_token_of_multi_byte_characters_is_clamped_between_characters() {
    // `token=` followed by 300 repetitions of a two-character CJK word (1800
    // bytes) and no ASCII separator: this is one unbroken multi-byte run behind
    // a secret-shaped prefix. The clamp is a *byte* budget, so it has to land on
    // a character boundary; slicing at the raw offset used to panic here, and
    // the same helper runs over every message body a snippet is built from.
    let body = format!("token={}", "密码".repeat(300));

    let (redacted, count) = crate::secret_patterns::redact_secret_patterns(body);

    assert_eq!(count, 1, "the secret-shaped prefix must still be detected");
    let tail = redacted
        .split_once("[REDACTED:secret_pattern]")
        .expect("the token body must be replaced")
        .1;
    assert!(
        redacted.starts_with("token=[REDACTED:secret_pattern]"),
        "the prefix is preserved and the token body is replaced: {redacted}"
    );
    assert_eq!(
        tail,
        "密码".repeat(215),
        "the clamp must cut between characters, not through one: 1800 bytes of \
         token minus the 510-byte clamp leaves 1290 bytes"
    );
}

#[test]
fn a_snippet_survives_a_secret_prefix_with_a_multi_byte_tail() {
    // The message-search snippet reaches the same clamp, so a panic here was a
    // 500 for every search that matched such a message.
    let content = format!("搜索命中 --- token={}", "密码".repeat(300));

    let snippet = bounded_snippet(&content, "搜索命中");

    assert!(
        snippet.contains("搜索命中"),
        "the hit must still be shown, got {snippet}"
    );
    assert!(
        snippet.chars().count() <= MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
        "a snippet must stay inside its codepoint budget, got {}",
        snippet.chars().count()
    );
}

// ---------------------------------------------------------------------------
// Unified search: workspace and session scope (design §7.2 step two)
// ---------------------------------------------------------------------------

async fn create_workspace(
    store: &SqliteProductStore,
    temp: &TempDir,
    name: &str,
) -> ProductWorkspaceId {
    let root = temp.path().join(name);
    fs::create_dir_all(&root).unwrap();
    store
        .create_workspace(CreateProductWorkspaceRequest {
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: Some(name.to_string()),
            pinned: false,
        })
        .await
        .unwrap()
        .id
}

async fn create_session_in(
    store: &SqliteProductStore,
    workspace_id: &ProductWorkspaceId,
    title: &str,
) -> ProductSessionId {
    store
        .create_session(CreateProductSessionRequest {
            workspace_id: workspace_id.clone(),
            title: Some(title.to_string()),
        })
        .await
        .unwrap()
        .id
}

fn scoped(term: &str, cursor: Option<ProductSearchCursor>, limit: usize) -> ProductSearchQuery {
    ProductSearchQuery {
        term: term.to_string(),
        cursor,
        limit,
    }
}

async fn workspace_hits(
    store: &SqliteProductStore,
    workspace_id: &ProductWorkspaceId,
    query: ProductSearchQuery,
) -> Vec<(ProductSessionId, i64)> {
    store
        .search_scoped_messages(
            &ProductMessageSearchScope::Workspace(workspace_id.clone()),
            query,
        )
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| (hit.session_id, hit.seq))
        .collect()
}

#[tokio::test]
async fn workspace_scope_walks_every_session_without_gaps_or_repeats() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_id = create_workspace(&store, &temp, "workspace-scope").await;
    let mut written = Vec::new();
    for (index, title) in ["first", "second", "third"].iter().enumerate() {
        let session = create_session_in(&store, &workspace_id, title).await;
        for hit in 0..=index {
            let seq = send(&store, &session, &format!("scoped contract {hit}")).await;
            written.push((session.clone(), seq));
        }
        // A message that must never appear in these pages.
        send(&store, &session, "unrelated topic").await;
    }
    // A session in another workspace holds matching text and must stay out.
    let other_workspace = create_workspace(&store, &temp, "other-workspace").await;
    let outsider = create_session_in(&store, &other_workspace, "outsider").await;
    send(&store, &outsider, "scoped contract outsider").await;

    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = store
            .search_scoped_messages(
                &ProductMessageSearchScope::Workspace(workspace_id.clone()),
                scoped("scoped contract", cursor, 2),
            )
            .await
            .unwrap();
        pages += 1;
        assert!(page.hits.len() <= 2, "a page stays inside its limit");
        seen.extend(
            page.hits
                .iter()
                .map(|hit| (hit.session_id.clone(), hit.seq)),
        );
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
        assert!(pages < 12, "pagination must terminate");
    }

    assert_eq!(pages, 3, "six hits at two per page is three pages");
    assert_eq!(seen, written, "the walk returns every hit once, in order");
    assert!(
        seen.iter().all(|(session, _)| *session != outsider),
        "a workspace scope must not leave its workspace"
    );
    // The order is the paging key, which is also what makes one page answer
    // "which sessions contain this term": a session's hits are contiguous.
    let mut ordered = seen.clone();
    ordered.sort_by(|left, right| {
        left.0
            .to_string()
            .cmp(&right.0.to_string())
            .then(left.1.cmp(&right.1))
    });
    assert_eq!(seen, ordered, "the workspace page order is the cursor key");

    // A term no session in the workspace contains is an empty page, not an
    // error and not a page from somewhere else.
    let empty = workspace_hits(
        &store,
        &workspace_id,
        scoped("nothing anywhere carries this", None, 32),
    )
    .await;
    assert!(empty.is_empty());
}

#[tokio::test]
async fn workspace_scope_refuses_a_term_the_trigram_index_cannot_serve() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_id = create_workspace(&store, &temp, "short-term").await;
    let session = create_session_in(&store, &workspace_id, "short").await;
    let seq = send(&store, &session, "ab contract").await;

    // Below the trigram floor the index cannot answer, and the fallback scan is
    // bounded by one session: across a workspace it would scan every control row
    // in the store, which is exactly the unbounded work this must not do.
    for term in ["ab", "a"] {
        let error = store
            .search_scoped_messages(
                &ProductMessageSearchScope::Workspace(workspace_id.clone()),
                scoped(term, None, 32),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            ProductErrorCode::ProductInvalidInput,
            "{term} must be refused for a workspace scope"
        );
    }
    // The session scope keeps the bounded fallback, so the same term still
    // answers there.
    let page = store
        .search_scoped_messages(
            &ProductMessageSearchScope::Session(session.clone()),
            scoped("ab", None, 32),
        )
        .await
        .unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].seq, seq);
}

#[tokio::test]
async fn an_unknown_scope_target_is_not_found_rather_than_empty() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_id = create_workspace(&store, &temp, "known").await;
    let session = create_session_in(&store, &workspace_id, "known session").await;

    let missing_workspace = ProductWorkspaceId::new();
    let error = store
        .search_scoped_messages(
            &ProductMessageSearchScope::Workspace(missing_workspace),
            scoped("contract", None, 32),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductNotFound);

    let missing_session = ProductSessionId::new();
    let error = store
        .search_scoped_messages(
            &ProductMessageSearchScope::Session(missing_session),
            scoped("contract", None, 32),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ProductErrorCode::ProductNotFound);

    // A session that exists but has no matching message is an empty page.
    let page = store
        .search_scoped_messages(
            &ProductMessageSearchScope::Session(session),
            scoped("contract", None, 32),
        )
        .await
        .unwrap();
    assert!(page.hits.is_empty());
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn the_session_scope_answers_exactly_what_the_step_one_search_answers() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_id = create_workspace(&store, &temp, "equivalent").await;
    let session = create_session_in(&store, &workspace_id, "equivalent").await;
    // A neighbour's identical text must not be reachable from this scope.
    let neighbour = create_session_in(&store, &workspace_id, "neighbour").await;
    send(&store, &neighbour, "contract in the neighbour").await;
    for index in 0..3 {
        send(&store, &session, &format!("contract {index}")).await;
    }

    let step_one = store
        .search_messages(&session, query(&session, "contract", None, 32))
        .await
        .unwrap();
    let scoped_page = store
        .search_scoped_messages(
            &ProductMessageSearchScope::Session(session.clone()),
            scoped("contract", None, 32),
        )
        .await
        .unwrap();

    assert_eq!(
        scoped_page
            .hits
            .iter()
            .map(|hit| (hit.session_id.clone(), hit.seq, hit.snippet.clone()))
            .collect::<Vec<_>>(),
        step_one
            .hits
            .iter()
            .map(|hit| (session.clone(), hit.message_seq, hit.snippet.clone()))
            .collect::<Vec<_>>(),
        "the unified session scope must not change what step one returned"
    );
    assert_eq!(
        scoped_page
            .hits
            .iter()
            .map(|hit| hit.source)
            .collect::<Vec<_>>(),
        vec![crate::ProductSearchSource::Message; 3]
    );
}

#[tokio::test]
async fn a_scoped_cursor_is_refused_across_terms_scopes_and_shapes() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_id = create_workspace(&store, &temp, "cursor").await;
    let session = create_session_in(&store, &workspace_id, "cursor").await;
    for index in 0..4 {
        send(&store, &session, &format!("cursor contract {index}")).await;
    }
    let workspace_scope = ProductMessageSearchScope::Workspace(workspace_id.clone());
    let session_scope = ProductMessageSearchScope::Session(session.clone());
    let first = store
        .search_scoped_messages(&workspace_scope, scoped("cursor contract", None, 2))
        .await
        .unwrap();
    let workspace_cursor = first.next_cursor.expect("four hits at two per page");

    // The same page one more time proves the cursor advances rather than
    // repeating: a cursor that read as "no position" would answer page one and
    // hide the mistake.
    let second = store
        .search_scoped_messages(
            &workspace_scope,
            scoped("cursor contract", Some(workspace_cursor.clone()), 2),
        )
        .await
        .unwrap();
    assert_eq!(second.hits.len(), 2);
    assert_ne!(second.hits[0].seq, first.hits[0].seq);

    let refused: [(ProductMessageSearchScope, ProductSearchQuery); 4] = [
        // Another term's digest, offered to the scope that minted it.
        (
            workspace_scope.clone(),
            scoped(
                "cursor contract",
                Some(ProductSearchCursor::after_message(
                    Some(session.clone()),
                    first.hits[0].seq,
                    &workspace_scope.cursor_digest("cursor"),
                )),
                2,
            ),
        ),
        // A workspace position offered to the session scope, whose key is a
        // bare `seq` and cannot read a `(session_id, seq)` position.
        (
            session_scope.clone(),
            scoped("cursor contract", Some(workspace_cursor.clone()), 2),
        ),
        // The other way round: a session position — a bare `seq` with no
        // session — offered to the workspace scope, which pages on
        // `(session_id, seq)`. The digest is correct, so only the shape refuses
        // it; accepting it would read a session's `seq` as a workspace-wide key
        // and skip every session that sorts before the last one on the page.
        (
            workspace_scope.clone(),
            scoped(
                "cursor contract",
                Some(ProductSearchCursor::after_message(
                    None,
                    first.hits[0].seq,
                    &workspace_scope.cursor_digest("cursor contract"),
                )),
                2,
            ),
        ),
        // A trace position, which the message ledger cannot read at all.
        (
            workspace_scope.clone(),
            scoped(
                "cursor contract",
                Some(ProductSearchCursor::after_trace(
                    1,
                    0,
                    &workspace_scope.cursor_digest("cursor contract"),
                )),
                2,
            ),
        ),
    ];
    for (scope, query) in refused {
        let error = store
            .search_scoped_messages(&scope, query.clone())
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            ProductErrorCode::ProductInvalidInput,
            "{} must be refused on {}",
            query.term,
            scope.canonical()
        );
    }
}

#[tokio::test]
async fn the_scoped_search_enforces_its_term_and_page_bounds_before_any_query() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace_id = create_workspace(&store, &temp, "bounds").await;
    let session = create_session_in(&store, &workspace_id, "bounds").await;
    send(&store, &session, "contract").await;
    let workspace_scope = ProductMessageSearchScope::Workspace(workspace_id);
    let session_scope = ProductMessageSearchScope::Session(session);

    let too_long = "c".repeat(129);
    for (term, limit) in [
        ("", 32),
        ("   ", 32),
        (too_long.as_str(), 32),
        ("contract", 0),
        ("contract", MAX_PRODUCT_SEARCH_LIMIT + 1),
        ("contract\u{FFFD}", 32),
    ] {
        for scope in [&workspace_scope, &session_scope] {
            let error = store
                .search_scoped_messages(scope, scoped(term, None, limit))
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                ProductErrorCode::ProductInvalidInput,
                "term {term:?} with limit {limit} must be refused"
            );
        }
    }
    // The page cap is a bound, not the default.
    let page = store
        .search_scoped_messages(
            &workspace_scope,
            scoped("contract", None, MAX_PRODUCT_SEARCH_LIMIT),
        )
        .await
        .unwrap();
    assert_eq!(page.hits.len(), 1);
    assert!(page.next_cursor.is_none());
}

/// The one- and two-character fallback is bounded by one session, never the store.
///
/// A term shorter than the `trigram` floor takes the `LIKE` statement in
/// `search::SHORT_TERM_SEARCH_SQL` (design §7.4 deviation 1). That path is
/// linear in the session's messages and *nothing* caps the session's ledger —
/// `MAX_PENDING_MESSAGES_PER_SESSION` bounds the pending queue, not the
/// messages a session has already accumulated — so the design records its
/// measured cost (eight VM steps per ledger row) rather than claiming it is
/// free, and prices the one- and two-character postings index that would remove
/// it.
///
/// The property that makes the cost acceptable is that the scan visits the
/// requested session and stops there. That is also what the workspace scope's
/// typed 400 for short terms rests on: a fallback that scanned the store would
/// make the refusal meaningless and would let one session's search read
/// another's rows. This test pins it against the shipped statement — the plan
/// it takes and the steps it spends while a session ten times larger sits in
/// the same store — and checks that the same statement still answers with the
/// hit it is supposed to find.
#[tokio::test]
async fn a_short_term_search_never_leaves_its_session() {
    use rusqlite::StatementStatus;

    const SESSION_MESSAGES: usize = 400;
    const OTHER_MESSAGES: usize = 4_000;

    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let session = create_session(&store, &temp).await;
    let other = create_session(&store, &temp).await;
    let connection = Connection::open(temp.path().join("product.sqlite")).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO product_session_controls(
                     control_id, product_session_id, kind, content, status, seq,
                     created_at, message_contract_version, requested_delivery
                 ) VALUES (?1, ?2, 'followup', ?3, 'accepted', ?4, '2026-09-26T00:00:00Z', 1, 'successor')",
            )
            .unwrap();
        for (session_id, count) in [(&session, SESSION_MESSAGES), (&other, OTHER_MESSAGES)] {
            for index in 0..count {
                // Roughly the size of a product message body, and the marker
                // only in the *last* one, so a matching term is reached at the
                // end of the session and the scan has to walk the whole thing.
                let mut body = "runtime contract message ledger search session ".repeat(8);
                if session_id == &session && index + 1 == SESSION_MESSAGES {
                    body.push_str(" rare marker 合同");
                }
                statement
                    .execute(rusqlite::params![
                        format!("probe-{session_id}-{index}"),
                        session_id.to_string(),
                        body,
                        index as i64 + 1,
                    ])
                    .unwrap();
            }
        }
    }
    transaction.commit().unwrap();

    // The plan of the statement the store actually issues.
    let plan = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {SHORT_TERM_SEARCH_SQL}"))
        .unwrap()
        .query_map(
            rusqlite::params![session.to_string(), "%zz%", 0_i64, 33_i64],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .map(|row| row.unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains(
            "SEARCH product_session_controls USING INDEX idx_product_session_controls_session_seq"
        ),
        "the fallback must seek the requested session: {plan}"
    );
    assert!(
        !plan.contains("SCAN product_session_controls"),
        "the fallback must not scan the whole ledger: {plan}"
    );

    // The cost, in VM steps: deterministic, unlike a wall clock.
    let mut statement = connection.prepare(SHORT_TERM_SEARCH_SQL).unwrap();
    let _ = statement.reset_status(StatementStatus::VmStep);
    let absent = statement
        .query_map(
            rusqlite::params![session.to_string(), "%zz%", 0_i64, 33_i64],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        .count();
    let steps = statement.reset_status(StatementStatus::VmStep);
    assert_eq!(absent, 0, "a term that is absent finds nothing");
    // Measured at eight steps per session row on the bundled build; this bound
    // is four times looser, so it survives a plan detail changing without
    // letting the walk grow into another session's rows (the other session is
    // already ten times this one).
    assert!(
        i64::from(steps) <= SESSION_MESSAGES as i64 * 32,
        "a short term may only pay for its own session: {steps} steps for {SESSION_MESSAGES} messages"
    );

    // The same statement still answers with the hit, through the store.
    let hits = search(&store, &session, "合同").await;
    assert_eq!(hits.len(), 1, "the marker message must be found: {hits:?}");
    assert_eq!(hits[0].0, SESSION_MESSAGES as i64);
}
