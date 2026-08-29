//! Tests for the paged session listing (codex alignment Phase 7).
//!
//! Kept apart from `tests.rs`, which is already several thousand lines, so the
//! pagination story reads as one piece.
//!
//! The listing has three properties worth defending, and each test here is
//! written so that breaking one of them makes it fail:
//!
//! 1. A paged walk sees every session exactly once, in the order an unpaged read
//!    would have produced. A cursor that re-delivers or skips a row breaks this.
//! 2. Archived sessions stay grouped after live ones. Losing the leading rank
//!    term interleaves them.
//! 3. A deep page is a seek, not a sort. This is the one that cannot be observed
//!    from the results at all, so it is asserted against the query plan.

use std::fs;

use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::product::{
    CreateProductSessionRequest, CreateProductWorkspaceRequest, ProductSessionPageQuery,
    ProductSessionStatus, ProductStore, ProductWorkspaceKind, UpdateProductSessionRequest,
};

use super::SqliteProductStore;
use super::repository::rank_page_sql;

fn open_store(temp: &TempDir) -> SqliteProductStore {
    SqliteProductStore::open(temp.path().join("product.sqlite"), 5_000).unwrap()
}

async fn workspace(store: &SqliteProductStore, temp: &TempDir) -> crate::product::ProductWorkspace {
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    store
        .create_workspace(CreateProductWorkspaceRequest {
            root,
            kind: ProductWorkspaceKind::Folder,
            display_name: Some("Pagination workspace".to_string()),
            pinned: false,
        })
        .await
        .unwrap()
}

/// A page query with the fields most tests do not care about filled in.
fn page(
    workspace_id: &crate::product::ProductWorkspaceId,
    limit: usize,
) -> ProductSessionPageQuery {
    ProductSessionPageQuery {
        workspace_id: workspace_id.clone(),
        cursor: None,
        limit,
        search: None,
        include_archived: true,
    }
}

/// Walk the whole listing `limit` rows at a time, following `next_cursor`.
///
/// The page budget is a guard, not a limit on the data: a cursor that fails to
/// advance would otherwise spin here forever instead of failing the test.
async fn walk(
    store: &SqliteProductStore,
    template: ProductSessionPageQuery,
) -> Vec<crate::product::ProductSession> {
    let mut collected = Vec::new();
    let mut cursor = None;
    for _ in 0..512 {
        let mut query = template.clone();
        query.cursor = cursor;
        let result = store.list_sessions(query).await.unwrap();
        collected.extend(result.sessions);
        match result.next_cursor {
            Some(next) => cursor = Some(next),
            None => return collected,
        }
    }
    panic!("the walk never reached a last page: a cursor is not advancing");
}

fn open_connection(temp: &TempDir) -> Connection {
    Connection::open(temp.path().join("product.sqlite")).unwrap()
}

/// Overwrite a session's `updated_at` so ordering tests do not depend on how
/// fast the machine ran.
fn set_updated_at(temp: &TempDir, session_id: &crate::product::ProductSessionId, updated_at: &str) {
    open_connection(temp)
        .execute(
            "UPDATE product_sessions SET updated_at = ?2 WHERE product_session_id = ?1",
            params![session_id.to_string(), updated_at],
        )
        .unwrap();
}

async fn seed(
    store: &SqliteProductStore,
    temp: &TempDir,
    workspace_id: &crate::product::ProductWorkspaceId,
    count: usize,
) -> Vec<crate::product::ProductSession> {
    let mut created = Vec::new();
    for index in 0..count {
        let session = store
            .create_session(CreateProductSessionRequest {
                workspace_id: workspace_id.clone(),
                title: Some(format!("session {index:03}")),
            })
            .await
            .unwrap();
        // Two sessions share every timestamp, so the walk has to rely on the id
        // tiebreak rather than on timestamps happening to be unique.
        set_updated_at(
            temp,
            &session.id,
            &format!("2026-08-26T10:{:02}:00.000Z", index / 2),
        );
        created.push(session);
    }
    created
}

#[tokio::test]
async fn a_paged_walk_sees_every_session_exactly_once_and_in_order() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    seed(&store, &temp, &workspace.id, 25).await;

    let unpaged = store.list_all_sessions(&workspace.id).await.unwrap();
    assert_eq!(unpaged.len(), 25, "the fixture did not land");

    // Page sizes that divide the total, that do not, that are 1, and that exceed
    // it: the boundary cases are where an off-by-one in the keyset shows up.
    for limit in [1, 2, 5, 7, 24, 25, 26, 100] {
        let walked = walk(&store, page(&workspace.id, limit)).await;
        let walked_ids: Vec<_> = walked.iter().map(|session| session.id.clone()).collect();
        let unpaged_ids: Vec<_> = unpaged.iter().map(|session| session.id.clone()).collect();
        assert_eq!(
            walked_ids, unpaged_ids,
            "a walk at page size {limit} did not reproduce the unpaged listing"
        );
    }
}

#[tokio::test]
async fn a_full_page_is_distinguished_from_the_last_page_without_a_count() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    seed(&store, &temp, &workspace.id, 4).await;

    // Four rows read two at a time. Both pages are exactly full, so their row
    // counts say nothing about whether more exists; only the probe row does.
    let first = store.list_sessions(page(&workspace.id, 2)).await.unwrap();
    assert_eq!(first.sessions.len(), 2);
    let cursor = first
        .next_cursor
        .expect("a full page with two rows behind it must offer a cursor");

    let mut query = page(&workspace.id, 2);
    query.cursor = Some(cursor);
    let second = store.list_sessions(query).await.unwrap();
    assert_eq!(second.sessions.len(), 2, "the second page came back short");
    assert!(
        second.next_cursor.is_none(),
        "a full page that exhausts the listing must not ask the client to \
         request an empty one"
    );

    // Both pages together are the whole listing, which is what makes the absent
    // cursor above correct rather than merely convenient.
    let walked: Vec<_> = first
        .sessions
        .iter()
        .chain(second.sessions.iter())
        .map(|session| session.id.clone())
        .collect();
    let unpaged: Vec<_> = store
        .list_all_sessions(&workspace.id)
        .await
        .unwrap()
        .iter()
        .map(|session| session.id.clone())
        .collect();
    assert_eq!(walked, unpaged);
}

/// Archive every other session, leaving the two groups interleaved by timestamp.
///
/// That interleaving is the point: if the listing's rank term were dropped, the
/// archived rows would come back mixed in among the live ones, and the grouping
/// assertion below would catch it.
async fn seed_half_archived(
    store: &SqliteProductStore,
    temp: &TempDir,
    workspace_id: &crate::product::ProductWorkspaceId,
    count: usize,
) {
    let sessions = seed(store, temp, workspace_id, count).await;
    for session in sessions.iter().step_by(2) {
        store
            .update_session(
                &session.id,
                UpdateProductSessionRequest {
                    title: None,
                    archived: Some(true),
                },
            )
            .await
            .unwrap();
        // Archiving touches `updated_at`, which would otherwise put every
        // archived row at the front of its group and hide ordering mistakes.
        set_updated_at(temp, &session.id, &session.updated_at);
    }
}

#[tokio::test]
async fn archived_sessions_stay_grouped_after_the_live_ones_across_page_boundaries() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    seed_half_archived(&store, &temp, &workspace.id, 16).await;

    // A page size that does not align with the group boundary, so the transition
    // from live to archived happens mid-page at least once.
    let walked = walk(&store, page(&workspace.id, 3)).await;
    assert_eq!(walked.len(), 16, "the walk lost rows");

    let ranks: Vec<bool> = walked
        .iter()
        .map(|session| session.status == ProductSessionStatus::Archived)
        .collect();
    let first_archived = ranks.iter().position(|archived| *archived);
    assert_eq!(
        first_archived,
        Some(8),
        "the eight live sessions should come first: {ranks:?}"
    );
    assert!(
        ranks[8..].iter().all(|archived| *archived),
        "archived sessions must be contiguous at the end: {ranks:?}"
    );
}

#[tokio::test]
async fn archived_sessions_can_be_excluded_entirely() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    seed_half_archived(&store, &temp, &workspace.id, 16).await;

    let mut query = page(&workspace.id, 3);
    query.include_archived = false;
    let walked = walk(&store, query).await;

    assert_eq!(walked.len(), 8, "only the live sessions should be listed");
    assert!(
        walked
            .iter()
            .all(|session| session.status != ProductSessionStatus::Archived),
        "an archived session survived the filter"
    );
}

#[tokio::test]
async fn a_search_matches_case_insensitively_and_treats_wildcards_literally() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    for title in [
        "Deploy the API",
        "deploy the CLI",
        "Review 100% of it",
        "Rename a_b",
    ] {
        store
            .create_session(CreateProductSessionRequest {
                workspace_id: workspace.id.clone(),
                title: Some(title.to_string()),
            })
            .await
            .unwrap();
    }

    let search = |term: &str| {
        let mut query = page(&workspace.id, 10);
        query.search = Some(term.to_string());
        query
    };

    let deploys = store.list_sessions(search("DEPLOY")).await.unwrap();
    assert_eq!(
        deploys.sessions.len(),
        2,
        "the search should ignore case in both directions"
    );

    // `%` and `_` are LIKE metacharacters. Unescaped, the first would match every
    // title and the second would match any single character.
    let percent = store.list_sessions(search("100%")).await.unwrap();
    assert_eq!(
        percent.sessions.len(),
        1,
        "a literal percent sign matched more than the one title containing it"
    );
    let underscore = store.list_sessions(search("a_b")).await.unwrap();
    assert_eq!(
        underscore.sessions.len(),
        1,
        "a literal underscore behaved as a wildcard"
    );
    let wildcard_only = store.list_sessions(search("%")).await.unwrap();
    assert_eq!(
        wildcard_only.sessions.len(),
        1,
        "a bare percent sign should be a search for that character, not for everything"
    );
}

/// Insert sessions with direct SQL, bypassing `create_session`.
///
/// `create_session` enforces `MAX_PRODUCT_SESSIONS` across the whole table, so a
/// fixture of this size is not reachable through the public API. Inserting rows
/// directly is legitimate here because the read path does not care how a row
/// arrived, and the point is to show the listing has headroom well past the
/// current write cap.
fn insert_sessions_directly(
    temp: &TempDir,
    workspace_id: &crate::product::ProductWorkspaceId,
    count: usize,
) {
    let mut connection = open_connection(temp);
    let transaction = connection.transaction().unwrap();
    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO product_sessions(
                    product_session_id, workspace_id, title, status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                "#,
            )
            .unwrap();
        for index in 0..count {
            // A quarter archived, so both rank groups are deep enough that a sort
            // over either would be visible in the plan.
            let status = if index % 4 == 3 { "archived" } else { "idle" };
            let stamp = format!("2026-01-01T00:00:00.{:03}Z", index % 1000);
            statement
                .execute(params![
                    crate::product::ProductSessionId::new().to_string(),
                    workspace_id.to_string(),
                    format!("bulk session {index:05}"),
                    status,
                    stamp,
                ])
                .unwrap();
        }
    }
    transaction.commit().unwrap();
}

#[tokio::test]
async fn a_deep_page_seeks_the_index_instead_of_sorting_the_workspace() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    insert_sessions_directly(&temp, &workspace.id, 10_000);

    // Read a real page first, so the plan below is asserted against a query the
    // listing actually issues rather than one this test invented.
    let first = store.list_sessions(page(&workspace.id, 50)).await.unwrap();
    assert_eq!(first.sessions.len(), 50);
    let cursor = first.next_cursor.expect("10k rows do not fit in one page");

    let mut query = page(&workspace.id, 50);
    query.cursor = Some(cursor.clone());
    let resumed = store.list_sessions(query.clone()).await.unwrap();
    assert_eq!(resumed.sessions.len(), 50, "the deep page came back short");

    let connection = open_connection(&temp);
    let sql = rank_page_sql(&query, true);
    let plan: Vec<String> = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(
            params![
                workspace.id.to_string(),
                cursor.archived_rank,
                cursor.updated_at,
                cursor.session_id.to_string(),
                51_i64
            ],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let plan = plan.join("\n");
    println!("query plan:\n{plan}");

    assert!(
        plan.contains("idx_product_sessions_workspace_page"),
        "the page query is not using the index built for it:\n{plan}"
    );
    // The assertion that matters. Without it the listing would still return the
    // right rows, and would still sort the entire workspace to do it.
    assert!(
        !plan.contains("TEMP B-TREE"),
        "the page is being sorted rather than seeked:\n{plan}"
    );
    assert!(
        plan.contains("updated_at<?"),
        "the cursor's timestamp is not bounding the scan:\n{plan}"
    );
}

/// Time a walk deep into a 10k-session workspace.
///
/// This is a budget check, not a proof of the paging design. Two things it was
/// measured *not* to catch, so nobody reads more into a green run than is there:
///
/// - Reintroducing the sort raises per-page cost by roughly half on this
///   fixture, which stays well inside any threshold loose enough to survive a
///   shared machine.
/// - Page latency under the sort is *also* flat with depth, because the sort is
///   over one rank group whose size does not depend on how far in the page is.
///   So a "deep pages cost no more than early ones" ratio does not separate the
///   two shapes either.
///
/// The query-plan test above is the actual guarantee. This test's job is to
/// notice an order-of-magnitude regression — a listing that went back to
/// scanning without an index, or a per-page cost that grows with the workspace.
#[tokio::test]
async fn paging_deep_into_a_ten_thousand_session_workspace_stays_flat() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let workspace = workspace(&store, &temp).await;
    insert_sessions_directly(&temp, &workspace.id, 10_000);

    let mut timings = Vec::new();
    let mut cursor = None;
    for _ in 0..60 {
        let mut query = page(&workspace.id, 50);
        query.cursor = cursor;
        let started = std::time::Instant::now();
        let result = store.list_sessions(query).await.unwrap();
        timings.push(started.elapsed());
        match result.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(
        timings.len() >= 50,
        "the walk stopped early, so it never got deep enough to measure"
    );

    let early: std::time::Duration = timings[..10].iter().sum();
    let deep: std::time::Duration = timings[timings.len() - 10..].iter().sum();
    timings.sort_unstable();
    let p95 = timings[timings.len() * 95 / 100];
    // Printed rather than asserted: the depth comparison is useful when reading a
    // failure and misleading as a gate, for the reason in the doc comment.
    println!(
        "pages: {}, p95: {p95:?}, first ten: {early:?}, last ten: {deep:?}",
        timings.len()
    );

    assert!(
        p95 < std::time::Duration::from_millis(50),
        "p95 page latency was {p95:?} across {} pages of a 10k-session workspace",
        timings.len()
    );
}
