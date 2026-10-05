//! Bounded content search over the persisted `trace.jsonl` of a session's runs.
//!
//! Design `2026-09-26-runtime-contract-alignment-design.md` §7.2 step two. The
//! trace is the only durable record of what a run actually did — a tool
//! argument, a provider failure, a plan decision — and none of that appears in
//! the message ledger, so an operator looking for a run by something they saw
//! in the event stream has nothing else to search.
//!
//! Three properties make this a search rather than a scan:
//!
//! 1. **The session bounds the corpus.** Runs are read in binding ordinal
//!    order, and a session's bindings are already capped by the store.
//! 2. **One request reads a bounded window.** At most
//!    [`MAX_PRODUCT_TRACE_SEARCH_RUNS_PER_REQUEST`] runs and
//!    [`MAX_PRODUCT_TRACE_SEARCH_BYTES_PER_REQUEST`] bytes — the byte budget is
//!    shared across the runs of the request — streamed record by record; no
//!    trace is ever loaded whole.
//! 3. **The window boundary is a page boundary, not a silent gap.** When a
//!    request stops on its budget it returns the exact position it stopped at,
//!    so paging covers every record without skipping one or reading one twice.
//!
//! Two failure modes are typed rather than folded into "no matches": a run whose
//! persisted trace was cleaned is `product_events_expired` (the binding is still
//! in the catalog, so the content is missing rather than empty), and a record
//! larger than a whole request's budget is a storage failure instead of a
//! partial record whose misses would look real.
//!
//! The position is a byte offset into one run's file. A trace record's own
//! sequence number cannot serve as that key: finding record *n* means reading
//! records `1..n`, so a sequence-keyed resume would re-read everything the
//! cursor already covered and grow with every page. A byte offset turns the
//! resume into a seek.
//!
//! Matching is a literal, ASCII-case-insensitive substring test over the raw
//! record, not `MATCH` or `LIKE`: traces are files, the store's FTS index
//! covers the message ledger only, and a record is JSON so a hit may sit inside
//! an escaped string. The excerpt leaves through the same bounded, redacted
//! snippet builder the message search uses, and a record large enough for that
//! builder's derived work to matter has the work done on the blocking pool
//! rather than on an async worker (see [`trace_hit_bounded`]).

use std::io::{ErrorKind, SeekFrom};
use std::path::PathBuf;

use rove_runtime::state::store::StateStore;
use rove_runtime::types::RunId;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};

use crate::product::{
    MAX_PRODUCT_TRACE_SEARCH_BYTES_PER_REQUEST, MAX_PRODUCT_TRACE_SEARCH_RUNS_PER_REQUEST,
    ProductErrorCode, ProductSearchCursor, ProductSearchHit, ProductSearchPage, ProductSearchQuery,
    ProductSearchSource, ProductSessionId, ProductStore, ProductStoreError,
};

use super::store::bounded_snippet;

/// One run whose persisted trace a session trace search may read.
#[derive(Debug, Clone)]
pub(crate) struct TraceRunSource {
    pub ordinal: u64,
    pub run_id: RunId,
    pub trace_path: PathBuf,
}

/// Where a trace page scan starts.
#[derive(Debug, Clone, Copy)]
struct TraceStart {
    index: usize,
    offset: u64,
}

/// How one run's scan ended.
enum RunScan {
    /// The file was read to its end.
    Exhausted,
    /// The scan stopped at a line boundary. Resuming at that offset continues
    /// the page without a gap and without repeating a record.
    Stopped { offset: u64 },
}

/// Accumulated state of one page scan, shared with the per-run reader.
struct TraceScan<'a> {
    session_id: &'a ProductSessionId,
    term: &'a str,
    limit: usize,
    runs_per_request: usize,
    byte_budget: usize,
    scanned_bytes: usize,
    hits: Vec<ProductSearchHit>,
}

/// Search one session's persisted trace records.
///
/// The run bindings come from the product catalog and the files from the
/// workspace's runtime `StateStore`, which is the same split the transcript
/// projection uses: the catalog owns the mapping, the StateStore owns the
/// facts.
pub(crate) async fn search_session_trace(
    store: &dyn ProductStore,
    state_store: &StateStore,
    session_id: &ProductSessionId,
    query: &ProductSearchQuery,
    digest: &str,
) -> Result<ProductSearchPage, ProductStoreError> {
    let bindings = store.list_run_bindings(session_id).await?;
    let runs: Vec<TraceRunSource> = bindings
        .into_iter()
        .map(|binding| TraceRunSource {
            ordinal: binding.ordinal,
            trace_path: state_store
                .run_store
                .run_dir(&binding.runtime_run_id)
                .join("trace.jsonl"),
            run_id: binding.runtime_run_id,
        })
        .collect();
    scan_trace_runs(&runs, session_id, query, digest).await
}

/// Scan run traces in order and return one bounded page.
pub(crate) async fn scan_trace_runs(
    runs: &[TraceRunSource],
    session_id: &ProductSessionId,
    query: &ProductSearchQuery,
    digest: &str,
) -> Result<ProductSearchPage, ProductStoreError> {
    scan_trace_runs_with_budget(
        runs,
        session_id,
        query,
        digest,
        MAX_PRODUCT_TRACE_SEARCH_RUNS_PER_REQUEST,
        MAX_PRODUCT_TRACE_SEARCH_BYTES_PER_REQUEST,
    )
    .await
}

async fn scan_trace_runs_with_budget(
    runs: &[TraceRunSource],
    session_id: &ProductSessionId,
    query: &ProductSearchQuery,
    digest: &str,
    runs_per_request: usize,
    byte_budget: usize,
) -> Result<ProductSearchPage, ProductStoreError> {
    let start = resolve_trace_cursor(runs, query.cursor.as_ref(), digest)?;
    let mut scan = TraceScan {
        session_id,
        term: &query.term,
        limit: query.limit,
        runs_per_request,
        byte_budget,
        scanned_bytes: 0,
        hits: Vec::new(),
    };
    let mut next_cursor = None;

    for (index, run) in runs.iter().enumerate().skip(start.index) {
        let resume_offset = if index == start.index {
            start.offset
        } else {
            0
        };
        // Two independent budgets, checked before any file is opened: the run
        // window, and the bytes this request may read in total. Reaching either
        // one stops the page at a position it can name.
        if index - start.index >= scan.runs_per_request || scan.scanned_bytes >= scan.byte_budget {
            next_cursor = Some(ProductSearchCursor::after_trace(
                run.ordinal,
                resume_offset,
                digest,
            ));
            break;
        }
        match scan_one_run(run, resume_offset, &mut scan).await? {
            RunScan::Exhausted => {}
            RunScan::Stopped { offset } => {
                next_cursor = Some(ProductSearchCursor::after_trace(
                    run.ordinal,
                    offset,
                    digest,
                ));
                break;
            }
        }
    }

    let mut hits = scan.hits;
    let has_more = hits.len() > query.limit;
    if has_more {
        hits.pop();
    }
    // `next_cursor` is the position the scan reached, whether it stopped on a
    // hit budget or a byte budget. Both mean "there may be more", and neither
    // means "a record was skipped": the resume reads from exactly here.
    Ok(ProductSearchPage { hits, next_cursor })
}

/// Resolve the page's resume position against the runs this session has now.
fn resolve_trace_cursor(
    runs: &[TraceRunSource],
    cursor: Option<&ProductSearchCursor>,
    digest: &str,
) -> Result<TraceStart, ProductStoreError> {
    let Some(cursor) = cursor else {
        return Ok(TraceStart {
            index: 0,
            offset: 0,
        });
    };
    if cursor.query_digest != digest {
        return Err(invalid(
            "search cursor was issued for a different scope or search term",
        ));
    }
    let Some(position) = cursor.trace_position() else {
        return Err(invalid("search cursor does not hold a trace position"));
    };
    // A binding that is gone cannot be resumed, and restarting the scan would
    // answer a resume with page one. A session's binding ordinals are
    // contiguous, so a missing one means the run history this cursor was paging
    // through is no longer there.
    let index = runs
        .iter()
        .position(|run| run.ordinal == position.run_ordinal)
        .ok_or_else(|| {
            ProductStoreError::new(
                ProductErrorCode::ProductEventsExpired,
                "the trace search cursor names a run this session no longer has",
            )
        })?;
    Ok(TraceStart {
        index,
        offset: position.record_offset,
    })
}

/// Stream one run's trace, collecting hits until the run or a budget ends.
async fn scan_one_run(
    run: &TraceRunSource,
    resume_offset: u64,
    scan: &mut TraceScan<'_>,
) -> Result<RunScan, ProductStoreError> {
    let mut file = match tokio::fs::File::open(&run.trace_path).await {
        Ok(file) => file,
        // The binding is still in the catalog, so this run is part of the corpus
        // and its content is gone: artifact cleanup removed it. Answering "no
        // matches" would report a hole in the corpus as a fact about the
        // session, and skipping the run would hide the difference between "the
        // term is not there" and "this part of the trace no longer exists".
        // Both a first pass and a resume therefore fail typed and explicitly,
        // which is what the cursor contract documents.
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductEventsExpired,
                "a run of this session no longer has its persisted trace",
            ));
        }
        Err(error) => return Err(trace_read_error(error)),
    };
    if resume_offset > 0 {
        file.seek(SeekFrom::Start(resume_offset))
            .await
            .map_err(trace_read_error)?;
    }
    let mut reader = BufReader::new(file);
    let mut offset = resume_offset;
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        let remaining = scan.byte_budget.saturating_sub(scan.scanned_bytes);
        if remaining == 0 {
            // The budget ran out exactly on a record boundary. If the file ends
            // here too, this run is finished and the page must not claim that
            // more of it can be read; the outer loop then moves on to the next
            // run, or ends the walk without a cursor at all.
            return if is_at_eof(&mut reader).await.map_err(trace_read_error)? {
                Ok(RunScan::Exhausted)
            } else {
                Ok(RunScan::Stopped { offset })
            };
        }
        let record_start = offset;
        bytes.clear();
        // `take` is what caps one record: reading on an unbounded reader would
        // follow a pathological record to its end, which is exactly the "one
        // record can cost anything" failure this bound exists to prevent.
        // Allocating a bounded reader per record is the price of that cap.
        //
        // The read is into bytes and never into a `String`, because a `take` cut
        // lands wherever the budget lands — including inside a multi-byte
        // character. `read_line` on a `String` validates the whole window and
        // returns `InvalidData` for exactly that cut, which would turn the
        // documented "stop before the record and return its offset" into a 500
        // that every retry reproduces; cutting a byte sequence is not an error
        // here, it is the boundary case this scanner is built around.
        let read = {
            let mut bounded = (&mut reader).take(u64::try_from(remaining).unwrap_or(u64::MAX));
            bounded
                .read_until(b'\n', &mut bytes)
                .await
                .map_err(trace_read_error)?
        };
        if read == 0 {
            return Ok(RunScan::Exhausted);
        }
        if bytes.last() != Some(&b'\n') {
            // The read stopped early for one of two very different reasons, and
            // they must not be conflated.
            //
            // A record longer than what was left of the budget must not be
            // searched in part: a miss on the part that was read would be
            // reported as a miss on the whole record. The page stops *before*
            // it instead, and the next request starts at this record's first
            // byte with a full budget.
            //
            // A file that simply ends without a trailing newline is the other
            // case: the runtime always terminates a record, so this is an append
            // that was interrupted, and the fragment is the whole of the last
            // record that exists. It is searchable, and stopping in front of it
            // would return a cursor pointing at the same byte forever.
            //
            // Both look identical in the buffer, so the reader itself decides.
            // One more byte means the record continues past the budget; end of
            // file means the fragment is all there is.
            let cut_by_budget =
                read >= remaining && !is_at_eof(&mut reader).await.map_err(trace_read_error)?;
            if cut_by_budget {
                if scan.scanned_bytes == 0 {
                    return Err(ProductStoreError::new(
                        ProductErrorCode::ProductStorageFailure,
                        "a single trace record exceeds the search read budget",
                    ));
                }
                return Ok(RunScan::Stopped {
                    offset: record_start,
                });
            }
        }
        scan.scanned_bytes += read;
        offset = offset.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        let line = String::from_utf8_lossy(&bytes);
        if let Some(hit) = trace_hit_bounded(&line, run, scan.session_id, scan.term).await? {
            scan.hits.push(hit);
            if scan.hits.len() > scan.limit {
                // The page is full and this record proves there is more. The
                // cursor names *this* record rather than the byte after it, so
                // the next page starts with the hit this one drops.
                return Ok(RunScan::Stopped {
                    offset: record_start,
                });
            }
        }
    }
}

/// Whether the reader has reached the end of its file.
///
/// A bounded read hides the difference between "the allowance ended" and "the
/// file ended", because both stop it with nothing left. The only way to tell
/// them apart is to ask for one more byte, so this probe is what keeps the two
/// cases honest. It consumes a byte only on the paths that answer with a
/// position the next request re-reads from, so no record is lost.
async fn is_at_eof<R>(reader: &mut R) -> std::io::Result<bool>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut probe = [0u8; 1];
    Ok(reader.read(&mut probe).await? == 0)
}

/// The envelope fields a hit needs, decoded on their own.
///
/// `rove_runtime::state::trace::TraceLine` is the canonical envelope and the
/// transcript projection decodes it whole. A search must not: requiring the
/// event payload to parse would report "no sequence, no timestamp" for a record
/// whose payload this build cannot read, even though it matched the term and is
/// exactly what the operator is looking for. Only the stable identity fields
/// are read here; the rest of the record is searched as text.
#[derive(Debug, Deserialize)]
struct TraceIdentity {
    ts: String,
    seq: u64,
}

/// Describe one matching trace record as a bounded, redacted hit.
///
/// The bytes read per request are bounded by
/// [`MAX_PRODUCT_TRACE_SEARCH_BYTES_PER_REQUEST`], but the *derived* work is not
/// the same number: the shared secret redaction makes a dozen whole-record
/// scans and can expand the text it rewrites, and the excerpt materialises the
/// record as characters — a bounded multiple of the record, not of the snippet.
/// That multiple is why a large record's match and excerpt run on the blocking
/// pool instead of on an async worker, the same place the message search's own
/// excerpt work runs. Records below
/// [`TRACE_DERIVED_WORK_BLOCKING_THRESHOLD_BYTES`] are answered inline: their
/// derived work is bounded by the threshold, and handing a task off would cost
/// more than the work itself.
async fn trace_hit_bounded(
    line: &str,
    run: &TraceRunSource,
    session_id: &ProductSessionId,
    term: &str,
) -> Result<Option<ProductSearchHit>, ProductStoreError> {
    if line.len() < TRACE_DERIVED_WORK_BLOCKING_THRESHOLD_BYTES {
        return Ok(trace_hit(line, run, session_id, term));
    }
    let line = line.to_owned();
    let run = run.clone();
    let session_id = session_id.clone();
    let term = term.to_owned();
    tokio::task::spawn_blocking(move || trace_hit(&line, &run, &session_id, &term))
        .await
        .map_err(|_| {
            ProductStoreError::new(
                ProductErrorCode::ProductStorageFailure,
                "trace search could not be completed",
            )
        })
}

/// The record size from which the derived work leaves the async executor.
///
/// Below it the whole per-record cost — a dozen redaction scans, the rewritten
/// text, and the excerpt's character buffer — is a small multiple of a small
/// number, so it is cheaper to do inline than to hand to another thread. Above
/// it the multiple is worth a task hand-off. Either way the per-request bound
/// comes from the read budget, not from this threshold: it decides where the
/// work runs, never how much of the trace is read.
const TRACE_DERIVED_WORK_BLOCKING_THRESHOLD_BYTES: usize = 64 * 1024;

/// Describe one matching trace record as a bounded, redacted hit.
fn trace_hit(
    line: &str,
    run: &TraceRunSource,
    session_id: &ProductSessionId,
    term: &str,
) -> Option<ProductSearchHit> {
    if !contains_ascii_case_insensitive(line, term) {
        return None;
    }
    // Identity is best effort and never invented. A record this build cannot
    // decode is still searchable — that is the point of matching the raw text —
    // but it has no sequence or timestamp of its own, and the legacy
    // bare-event format genuinely carries neither.
    let identity = serde_json::from_str::<TraceIdentity>(line).ok();
    Some(ProductSearchHit {
        session_id: session_id.clone(),
        source: ProductSearchSource::Trace,
        seq: identity
            .as_ref()
            .and_then(|identity| i64::try_from(identity.seq).ok())
            .unwrap_or(0),
        run_id: Some(run.run_id),
        run_ordinal: Some(run.ordinal),
        snippet: bounded_snippet(line, term),
        created_at: identity.map(|identity| identity.ts),
    })
}

/// Case-insensitive substring test that folds ASCII exactly like the message
/// search's two query paths do.
///
/// A byte window is sound here because UTF-8 is prefix-free: a byte sequence
/// that equals the needle's bytes decodes to the needle's characters, and ASCII
/// folding only ever touches single-byte characters.
pub(super) fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn trace_read_error(error: std::io::Error) -> ProductStoreError {
    ProductStoreError::new(
        ProductErrorCode::ProductStorageFailure,
        format!("trace could not be read: {}", error.kind()),
    )
}

fn invalid(message: &'static str) -> ProductStoreError {
    ProductStoreError::new(ProductErrorCode::ProductInvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::{
        DEFAULT_PRODUCT_SEARCH_LIMIT, MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
    };

    const DIGEST: &str = "sha256:test";

    fn run(ordinal: u64, path: PathBuf) -> TraceRunSource {
        TraceRunSource {
            ordinal,
            run_id: RunId::new(),
            trace_path: path,
        }
    }

    fn query(term: &str, cursor: Option<ProductSearchCursor>, limit: usize) -> ProductSearchQuery {
        ProductSearchQuery {
            term: term.to_string(),
            cursor,
            limit,
        }
    }

    fn write_trace(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn envelope(seq: u64, type_name: &str, extra: &str) -> String {
        format!(
            "{{\"ts\":\"2026-09-26T10:00:0{seq}Z\",\"seq\":{seq},\"event\":{{\"type\":\"{type_name}\"{extra}}}}}\n"
        )
    }

    async fn page_with_budget(
        runs: &[TraceRunSource],
        session_id: &ProductSessionId,
        query: &ProductSearchQuery,
        byte_budget: usize,
    ) -> Result<ProductSearchPage, ProductStoreError> {
        scan_trace_runs_with_budget(runs, session_id, query, DIGEST, 8, byte_budget).await
    }

    #[tokio::test]
    async fn a_trace_page_walks_every_record_once_without_gaps_or_repeats() {
        let dir = tempfile::TempDir::new().unwrap();
        let body = format!(
            "{}{}{}",
            envelope(1, "run_started", ""),
            envelope(2, "llm_message", ",\"full\":\"needle one\""),
            envelope(3, "tool_call", ",\"name\":\"needle two\"")
        );
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let session_id = ProductSessionId::new();

        let mut seen = Vec::new();
        let mut cursor = None;
        for _ in 0..8 {
            let page = scan_trace_runs(
                &runs,
                &session_id,
                &query("needle", cursor.clone(), 1),
                DIGEST,
            )
            .await
            .unwrap();
            assert!(page.hits.len() <= 1, "a page stays inside its limit");
            seen.extend(page.hits.iter().map(|hit| hit.seq).collect::<Vec<_>>());
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(
            seen,
            vec![2, 3],
            "each matching record is returned exactly once, oldest first"
        );
    }

    #[tokio::test]
    async fn a_trace_scan_stops_on_its_byte_budget_and_resumes_exactly_where_it_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        // Three records of *identical* byte length, so a one-record budget is
        // exactly one record on every page and the boundary arithmetic in the
        // assertion is about the scanner rather than about the fixture.
        let first = envelope(1, "note", ",\"full\":\"aaaaaa\"");
        let body = format!(
            "{}{}{}",
            first,
            envelope(2, "note", ",\"full\":\"needle\""),
            envelope(3, "note", ",\"full\":\"zzzzzz\"")
        );
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let session_id = ProductSessionId::new();
        let record_bytes = first.len();

        // A budget that admits exactly one record: the page must stop after it
        // and say where, rather than claim the history ended.
        let first_page =
            page_with_budget(&runs, &session_id, &query("needle", None, 4), record_bytes)
                .await
                .unwrap();
        assert!(first_page.hits.is_empty(), "record one has no match");
        let cursor = first_page
            .next_cursor
            .expect("a scan stopped by its budget must offer a cursor");
        assert_eq!(
            cursor.trace_position(),
            Some(crate::product::ProductTraceCursorPosition {
                run_ordinal: 1,
                record_offset: record_bytes as u64,
            }),
            "the cursor is the byte the scan reached"
        );

        let second = page_with_budget(
            &runs,
            &session_id,
            &query("needle", Some(cursor), 4),
            record_bytes,
        )
        .await
        .unwrap();
        assert_eq!(
            second.hits.iter().map(|hit| hit.seq).collect::<Vec<_>>(),
            vec![2],
            "the resumed page starts at the byte the first page stopped at"
        );
        assert!(
            first_page
                .hits
                .iter()
                .all(|hit| second.hits.iter().all(|next| next.seq != hit.seq)),
            "a resumed page never repeats a hit"
        );
    }

    #[tokio::test]
    async fn an_interrupted_final_record_is_searched_and_does_not_stall_the_cursor() {
        let dir = tempfile::TempDir::new().unwrap();
        // The writer always terminates a record with a newline, so a trace whose
        // last record has none is one whose final append was interrupted by a
        // crash. That is exactly the run an operator is searching for, and the
        // fragment is the whole of the record that exists.
        let first = envelope(1, "note", ",\"full\":\"aaaaaa\"");
        let fragment = envelope(2, "llm_message", ",\"full\":\"needle tail\"");
        let body = format!("{}{}", first, fragment.trim_end_matches('\n'));
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let session_id = ProductSessionId::new();

        // A budget that admits the first record exactly: the page stops at the
        // first record's boundary, so the fragment is reached by the resume.
        let page = page_with_budget(&runs, &session_id, &query("needle", None, 4), first.len())
            .await
            .unwrap();
        assert!(page.hits.is_empty(), "the first record has no match");
        let cursor = page.next_cursor.expect("the budget stopped the scan");

        let resumed = page_with_budget(
            &runs,
            &session_id,
            &query("needle", Some(cursor), 4),
            first.len() + fragment.len(),
        )
        .await
        .unwrap();
        assert_eq!(
            resumed.hits.iter().map(|hit| hit.seq).collect::<Vec<_>>(),
            vec![2],
            "a record with no trailing newline is still searchable"
        );
        assert!(
            resumed.next_cursor.is_none(),
            "the fragment is the end of the file, so the page must not hand back a \
             cursor that points at the same byte forever"
        );
    }

    #[tokio::test]
    async fn a_trace_record_larger_than_the_whole_budget_is_a_typed_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let body = envelope(
            1,
            "llm_message",
            &format!(",\"full\":\"{}\"", "x".repeat(256)),
        );
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let error = page_with_budget(
            &runs,
            &ProductSessionId::new(),
            &query("needle", None, 4),
            16,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductStorageFailure);
    }

    #[tokio::test]
    async fn a_budget_cut_inside_a_character_stops_before_the_record_instead_of_failing() {
        let dir = tempfile::TempDir::new().unwrap();
        // The first record fills the budget up to a byte, so the budget lands
        // inside the second record — and the second record's first non-ASCII
        // character sits exactly there, so the cut splits it. A reader that
        // validated that window as UTF-8 would answer a storage failure, the
        // same one on every retry, with no cursor minted to skip the byte; the
        // documented behavior is "stop before the record and name its offset".
        let first = envelope(1, "note", ",\"full\":\"aaaaaa\"");
        let second = "{\"full\":\"漢字漢字needle\"}\n";
        let cut = "{\"full\":\"".len() + 1;
        assert!(
            !second.is_char_boundary(cut),
            "the fixture must cut inside a character, or this test proves nothing"
        );
        let body = format!("{first}{second}");
        // A window that ends inside a character is not valid UTF-8 by
        // construction, which is what a `read_line` into a `String` refuses to
        // accept — so this fixture is the one that used to answer a storage
        // failure instead of a page boundary.
        assert!(
            second.as_bytes()[cut] & 0b1100_0000 == 0b1000_0000,
            "the byte at the cut must be a continuation byte"
        );
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let session_id = ProductSessionId::new();

        let page = page_with_budget(
            &runs,
            &session_id,
            &query("needle", None, 4),
            first.len() + cut,
        )
        .await
        .expect("a cut inside a character is a page boundary, not a storage failure");
        assert!(page.hits.is_empty(), "the match is in the cut record");
        let cursor = page
            .next_cursor
            .expect("the scan stopped before the cut record, so it must name it");
        assert_eq!(
            cursor.trace_position(),
            Some(crate::product::ProductTraceCursorPosition {
                run_ordinal: 1,
                record_offset: first.len() as u64,
            }),
            "the cursor is the cut record's first byte, so the resume re-reads it whole"
        );

        // With a budget that holds the whole record, the same hit is found: the
        // first page skipped nothing, it only deferred the record.
        let resumed = page_with_budget(
            &runs,
            &session_id,
            &query("needle", Some(cursor), 4),
            first.len() + second.len(),
        )
        .await
        .unwrap();
        assert_eq!(
            resumed.hits.len(),
            1,
            "the deferred record is searched next"
        );
        assert!(
            resumed.hits[0].snippet.contains("needle"),
            "the deferred record's excerpt is what the first page deferred"
        );
    }

    #[tokio::test]
    async fn a_record_exactly_as_long_as_the_remaining_budget_is_searched() {
        let dir = tempfile::TempDir::new().unwrap();
        // A file whose only record is exactly as long as the budget and has no
        // trailing newline (the fragment an interrupted append leaves). It is
        // not a record larger than the budget — the file simply ends at the
        // limit — so it must be searched rather than reported as oversized.
        let record = envelope(1, "llm_message", ",\"full\":\"needle\"");
        let body = record.trim_end_matches('\n').to_string();
        assert_eq!(body.len(), record.len() - 1);
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let page = page_with_budget(
            &runs,
            &ProductSessionId::new(),
            &query("needle", None, 4),
            body.len(),
        )
        .await
        .expect("a record that exactly fills the budget is not an oversized record");
        assert_eq!(
            page.hits.iter().map(|hit| hit.seq).collect::<Vec<_>>(),
            vec![1]
        );
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn exhausting_the_budget_on_the_last_record_ends_the_walk() {
        let dir = tempfile::TempDir::new().unwrap();
        // Two records, and a budget that is used up exactly at the second one's
        // final byte. Nothing is left to read, so the page must not answer with
        // a cursor that promises another page.
        let first = envelope(1, "note", ",\"full\":\"aaaaaa\"");
        let second = envelope(2, "note", ",\"full\":\"bbbbbb\"");
        let body = format!("{first}{second}");
        let runs = vec![run(1, write_trace(dir.path(), "a.jsonl", &body))];
        let page = page_with_budget(
            &runs,
            &ProductSessionId::new(),
            &query("needle", None, 4),
            body.len(),
        )
        .await
        .unwrap();
        assert!(page.hits.is_empty());
        assert!(
            page.next_cursor.is_none(),
            "the walk reached the end of the only run, so there is no next page"
        );

        // A budget that stops one byte earlier still points at the record it
        // could not read, so the same walk does not lose the tail.
        let page = page_with_budget(
            &runs,
            &ProductSessionId::new(),
            &query("needle", None, 4),
            body.len() - 1,
        )
        .await
        .unwrap();
        assert_eq!(
            page.next_cursor
                .expect("one byte short of the file means the last record is unread")
                .trace_position(),
            Some(crate::product::ProductTraceCursorPosition {
                run_ordinal: 1,
                record_offset: first.len() as u64,
            })
        );
    }

    #[tokio::test]
    async fn a_large_record_is_matched_and_redacted_through_the_offloaded_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let session = ProductSessionId::new();
        // Records at or above the blocking threshold take a different code path
        // for their match and excerpt. That path must produce exactly the same
        // bounded, redacted hit as the inline one, so the offload cannot become
        // a place where redaction or the excerpt bound is quietly skipped. The
        // filler is multi-byte so the excerpt window also crosses characters.
        let below = TRACE_DERIVED_WORK_BLOCKING_THRESHOLD_BYTES / 2;
        let above = TRACE_DERIVED_WORK_BLOCKING_THRESHOLD_BYTES * 2;
        for (name, padding) in [("inline.jsonl", below), ("offloaded.jsonl", above)] {
            let body = envelope(
                1,
                "llm_message",
                &format!(
                    ",\"full\":\"{}甲needle token=sk-export-content-canary-058761eb\"",
                    "x".repeat(padding)
                ),
            );
            assert_eq!(
                body.len() >= TRACE_DERIVED_WORK_BLOCKING_THRESHOLD_BYTES,
                padding == above,
                "{name}: the fixture must sit on the side of the threshold it names"
            );
            let runs = vec![run(1, write_trace(dir.path(), name, &body))];
            let page = scan_trace_runs(&runs, &session, &query("needle", None, 4), DIGEST)
                .await
                .unwrap();
            let hit = page.hits.first().expect("the record matches");
            assert!(
                !hit.snippet.contains("sk-export-content-canary-058761eb"),
                "{name}: a large record's excerpt must still be redacted"
            );
            assert!(
                hit.snippet.contains("[REDACTED:secret_pattern]"),
                "{name}: the redaction must be visible in the excerpt"
            );
            assert!(
                hit.snippet.contains("needle"),
                "{name}: the excerpt must show the hit"
            );
            assert!(
                hit.snippet.chars().count() <= MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
                "{name}: the excerpt stays inside its codepoint budget"
            );
        }
    }

    #[tokio::test]
    async fn the_run_window_bounds_one_page_and_the_cursor_reaches_the_next_run() {
        let dir = tempfile::TempDir::new().unwrap();
        let runs: Vec<TraceRunSource> = (1..=3)
            .map(|ordinal| {
                run(
                    ordinal,
                    write_trace(
                        dir.path(),
                        &format!("run-{ordinal}.jsonl"),
                        &envelope(1, "llm_message", ",\"full\":\"needle\""),
                    ),
                )
            })
            .collect();
        let session_id = ProductSessionId::new();
        let first = scan_trace_runs_with_budget(
            &runs,
            &session_id,
            &query("needle", None, 8),
            DIGEST,
            1,
            MAX_PRODUCT_TRACE_SEARCH_BYTES_PER_REQUEST,
        )
        .await
        .unwrap();
        assert_eq!(first.hits.len(), 1);
        assert_eq!(first.hits[0].run_ordinal, Some(1));
        let cursor = first.next_cursor.expect("a bounded run window must page");
        let second = scan_trace_runs_with_budget(
            &runs,
            &session_id,
            &query("needle", Some(cursor), 8),
            DIGEST,
            1,
            MAX_PRODUCT_TRACE_SEARCH_BYTES_PER_REQUEST,
        )
        .await
        .unwrap();
        assert_eq!(second.hits[0].run_ordinal, Some(2));
    }

    #[tokio::test]
    async fn a_trace_with_no_match_is_an_empty_page_and_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let runs = vec![run(
            1,
            write_trace(dir.path(), "a.jsonl", &envelope(1, "run_started", "")),
        )];
        let page = scan_trace_runs(
            &runs,
            &ProductSessionId::new(),
            &query("absent", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap();
        assert!(page.hits.is_empty());
        assert!(page.next_cursor.is_none());

        // A session with no runs at all is the same empty page.
        let page = scan_trace_runs(
            &[],
            &ProductSessionId::new(),
            &query("absent", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap();
        assert!(page.hits.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn a_missing_trace_file_is_expired_on_a_resume_and_on_a_first_page() {
        let dir = tempfile::TempDir::new().unwrap();
        let runs = vec![run(1, dir.path().join("gone.jsonl"))];
        let session_id = ProductSessionId::new();
        // The binding is still in the catalog, so the run is part of the corpus
        // and its content is missing rather than empty: a first page that
        // answered "no matches" would report a hole as a fact.
        let first = scan_trace_runs(
            &runs,
            &session_id,
            &query("needle", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap_err();
        assert_eq!(first.code, ProductErrorCode::ProductEventsExpired);

        let expired = scan_trace_runs(
            &runs,
            &session_id,
            &query(
                "needle",
                Some(ProductSearchCursor::after_trace(1, 32, DIGEST)),
                DEFAULT_PRODUCT_SEARCH_LIMIT,
            ),
            DIGEST,
        )
        .await
        .unwrap_err();
        assert_eq!(expired.code, ProductErrorCode::ProductEventsExpired);
    }

    #[tokio::test]
    async fn a_cleaned_run_beside_a_live_one_is_expired_rather_than_a_partial_answer() {
        let dir = tempfile::TempDir::new().unwrap();
        // Run one is fully searchable and run two's trace is gone. Answering the
        // hits of run one would read as "run two has no match", which is the one
        // thing the corpus cannot claim.
        let runs = vec![
            run(
                1,
                write_trace(
                    dir.path(),
                    "one.jsonl",
                    &envelope(1, "llm_message", ",\"full\":\"needle\""),
                ),
            ),
            run(2, dir.path().join("gone.jsonl")),
        ];
        let error = scan_trace_runs(
            &runs,
            &ProductSessionId::new(),
            &query("needle", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductEventsExpired);
    }

    #[tokio::test]
    async fn a_cursor_naming_a_run_this_session_no_longer_has_is_expired() {
        let dir = tempfile::TempDir::new().unwrap();
        let runs = vec![run(
            1,
            write_trace(dir.path(), "a.jsonl", &envelope(1, "run_started", "")),
        )];
        let error = scan_trace_runs(
            &runs,
            &ProductSessionId::new(),
            &query(
                "needle",
                Some(ProductSearchCursor::after_trace(7, 0, DIGEST)),
                4,
            ),
            DIGEST,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ProductErrorCode::ProductEventsExpired);
    }

    #[tokio::test]
    async fn a_trace_cursor_from_another_scope_or_term_is_refused_rather_than_reused() {
        let dir = tempfile::TempDir::new().unwrap();
        let runs = vec![run(
            1,
            write_trace(dir.path(), "a.jsonl", &envelope(1, "run_started", "")),
        )];
        for cursor in [
            // Another term's digest.
            ProductSearchCursor::after_trace(1, 0, "sha256:other"),
            // A message position, which this scope cannot read.
            ProductSearchCursor::after_message(None, 4, DIGEST),
        ] {
            let error = scan_trace_runs(
                &runs,
                &ProductSessionId::new(),
                &query("needle", Some(cursor), 4),
                DIGEST,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, ProductErrorCode::ProductInvalidInput);
        }
    }

    #[tokio::test]
    async fn a_legacy_bare_event_record_is_searchable_without_an_invented_sequence() {
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = "{\"type\":\"llm_message\",\"full\":\"legacy needle\"}\n";
        let runs = vec![run(1, write_trace(dir.path(), "legacy.jsonl", legacy))];
        let page = scan_trace_runs(
            &runs,
            &ProductSessionId::new(),
            &query("needle", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(
            page.hits[0].seq, 0,
            "a record with no envelope has no sequence to report"
        );
        assert!(page.hits[0].created_at.is_none());
    }

    #[tokio::test]
    async fn a_secret_shaped_trace_record_never_reaches_the_snippet() {
        let dir = tempfile::TempDir::new().unwrap();
        // The canary the evidence-export tests already use, so no fixture gains
        // a new secret-shaped string.
        let canary = "sk-export-content-canary-058761eb";
        let body = format!(
            "{}{}",
            envelope(1, "llm_message", ",\"full\":\"ordinary text\""),
            envelope(2, "tool_call", &format!(",\"args\":\"{canary}\"")),
        );
        let runs = vec![run(1, write_trace(dir.path(), "secret.jsonl", &body))];
        let page = scan_trace_runs(
            &runs,
            &ProductSessionId::new(),
            &query("canary", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap();
        assert_eq!(page.hits.len(), 1, "the redacted record is still a hit");
        let snippet = &page.hits[0].snippet;
        assert!(
            !snippet.contains(canary),
            "a trace snippet must not carry a secret pattern: {snippet}"
        );
        assert!(
            snippet.contains("[REDACTED:secret_pattern]"),
            "the snippet must show that something was redacted: {snippet}"
        );
    }

    #[tokio::test]
    async fn a_secret_prefix_with_a_multi_byte_body_is_clamped_on_a_character_boundary() {
        // `token=` followed by a long CJK run with no ASCII separator. The
        // secret clamp is a byte budget, and a byte budget applied without a
        // character-boundary check panics on exactly this input.
        let dir = tempfile::TempDir::new().unwrap();
        let body = format!("token={}", "密".repeat(1_800));
        let line = format!(
            "{{\"ts\":\"2026-09-26T10:00:00Z\",\"seq\":1,\"event\":{{\"type\":\"llm_message\",\"full\":\"{body}\"}}}}\n"
        );
        let runs = vec![run(1, write_trace(dir.path(), "multibyte.jsonl", &line))];
        let page = scan_trace_runs(
            &runs,
            &ProductSessionId::new(),
            &query("token=", None, DEFAULT_PRODUCT_SEARCH_LIMIT),
            DIGEST,
        )
        .await
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        let snippet = &page.hits[0].snippet;
        assert!(
            snippet.chars().count() <= MAX_PRODUCT_MESSAGE_SEARCH_SNIPPET_CODEPOINTS,
            "the snippet stays inside its codepoint budget"
        );
        assert!(snippet.contains("[REDACTED:secret_pattern]"), "{snippet}");
    }

    #[test]
    fn the_literal_matcher_folds_ascii_case_only() {
        assert!(contains_ascii_case_insensitive("RunStarted", "runstarted"));
        assert!(contains_ascii_case_insensitive("运行时合同", "合同"));
        assert!(!contains_ascii_case_insensitive("运行时合同", "运行合同"));
        assert!(!contains_ascii_case_insensitive("abc", "abcd"));
        // A non-ASCII term is matched exactly; ASCII folding must not change it.
        assert!(!contains_ascii_case_insensitive("Ä", "ä"));
    }
}
