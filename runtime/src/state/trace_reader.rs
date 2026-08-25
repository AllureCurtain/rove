//! Version-tolerant reader for `trace.jsonl` files.
//!
//! Traces carry every line inside a [`TraceLine`] envelope (`{ts, seq,
//! event}`), where `event` is a [`TraceEntry`] payload: either an explicit
//! model-visible [`HistoryItem`] (Phase 2 Codex alignment) or a UI/audit
//! [`StreamEvent`]. Legacy traces contain bare `StreamEvent` objects with no
//! sequence and no explicit history stream. This reader accepts all format
//! generations in the same file (lazy upgrade: old lines keep their
//! line-number-derived sequence), skips a truncated final line left behind by
//! a crash, and reports what it saw so callers can surface degraded reads
//! instead of failing.
use std::path::Path;

use crate::events::{StreamEvent, TraceEntry};

use super::trace::TraceLine;

/// One decoded trace record with its effective sequence number.
#[derive(Debug, Clone)]
pub struct TraceRecord {
    /// Envelope sequence when present; line number for legacy lines.
    pub seq: u64,
    /// RFC3339 timestamp from the envelope, if the line carried one.
    pub ts: Option<String>,
    /// The decoded payload: explicit history item or UI/audit event.
    pub entry: TraceEntry,
}

/// One explicitly persisted model-visible history item with its position in
/// the run's sequence space.
#[derive(Debug, Clone)]
pub struct HistoryRecord {
    pub seq: u64,
    pub item: rove_core::history::HistoryItem,
}

/// Bounded outcome of reading a whole trace file.
#[derive(Debug, Clone, Default)]
pub struct TraceReadOutcome {
    /// Successfully decoded records in file order.
    pub entries: Vec<TraceRecord>,
    /// Explicit model-visible history items in file order. Empty for legacy
    /// traces, which never carried a history stream.
    pub history_items: Vec<HistoryRecord>,
    /// Lines that parsed as neither envelope nor bare event.
    pub corrupt_line_count: usize,
    /// 1-based positions of the corrupt lines.
    pub corrupt_line_numbers: Vec<u64>,
    /// True when the last non-empty line failed to parse — the signature of
    /// a crash mid-write.
    pub truncated_tail: bool,
}

impl TraceReadOutcome {
    /// Sequence continuity check over the decoded records.
    ///
    /// Mixed legacy/envelope files may legitimately interleave sequences, so
    /// this only asserts that the run is *replayable*: records are ordered by
    /// file position and no record is duplicated within one format generation.
    pub fn is_monotonic_by_file_order(&self) -> bool {
        self.entries
            .windows(2)
            .all(|pair| pair[0].seq <= pair[1].seq + 1)
    }

    /// Whether the trace carries an explicit model-visible history stream.
    ///
    /// When false, resume must fall back to snapshot-derived history because
    /// the trace cannot rebuild model context without heuristics.
    pub fn has_explicit_history(&self) -> bool {
        !self.history_items.is_empty()
    }
}

fn parse_line(line: &str, fallback_seq: u64) -> std::io::Result<TraceRecord> {
    // New format first. The untagged payload distinguishes explicit history
    // items (`kind` tag) from UI/audit events (`type` tag) on disk.
    if let Ok(enveloped) = serde_json::from_str::<TraceLine>(line) {
        return Ok(TraceRecord {
            seq: enveloped.seq,
            ts: Some(enveloped.ts),
            entry: enveloped.event,
        });
    }
    // Legacy bare-event fallback: a UI-stream line with no explicit history.
    match serde_json::from_str::<StreamEvent>(line) {
        Ok(event) => Ok(TraceRecord {
            seq: fallback_seq,
            ts: None,
            entry: TraceEntry::Ui(event),
        }),
        Err(error) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    }
}

/// Read and decode a whole trace file, tolerating all line generations and a
/// truncated tail. A missing file yields an empty outcome.
pub async fn read_trace_file(path: &Path) -> std::io::Result<TraceReadOutcome> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TraceReadOutcome::default());
        }
        Err(error) => return Err(error),
    };
    Ok(read_trace_content(&content))
}

/// Synchronous variant of [`read_trace_file`].
pub fn read_trace_file_sync(path: &Path) -> std::io::Result<TraceReadOutcome> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TraceReadOutcome::default());
        }
        Err(error) => return Err(error),
    };
    Ok(read_trace_content(&content))
}

/// Decode in-memory trace content using the same rules as the file readers.
pub fn read_trace_content(content: &str) -> TraceReadOutcome {
    let mut outcome = TraceReadOutcome::default();
    let mut last_line_failed = false;
    // Legacy lines have no seq of their own; they take their 1-based line
    // position among all non-empty lines, matching historical behavior where
    // sequence == append order.
    let mut line_number: u64 = 0;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        line_number += 1;
        match parse_line(line, line_number) {
            Ok(record) => {
                if let TraceEntry::History(item) = &record.entry {
                    outcome.history_items.push(HistoryRecord {
                        seq: record.seq,
                        item: item.clone(),
                    });
                }
                outcome.entries.push(record);
                last_line_failed = false;
            }
            Err(_) => {
                // An interior bad line is plain corruption; only a trailing
                // unparsable line is treated as a truncated write.
                outcome.corrupt_line_count += 1;
                outcome.corrupt_line_numbers.push(line_number);
                last_line_failed = true;
            }
        }
    }
    outcome.truncated_tail = last_line_failed;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::index::StateIndex;
    use crate::state::trace::TraceWriter;
    use crate::types::{JobId, RunId, SessionId};
    use rove_core::history::HistoryItem;

    fn sample_event(delta: &str) -> StreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "llm_chunk",
            "delta": delta
        }))
        .unwrap()
    }

    fn sample_history_item(text: &str) -> HistoryItem {
        HistoryItem::Message(rove_models::Message::assistant(text))
    }

    #[test]
    fn new_format_lines_carry_ts_and_seq() {
        let content = concat!(
            r#"{"ts":"2026-08-25T00:00:00+00:00","seq":1,"event":{"type":"llm_chunk","delta":"a"}}"#,
            "\n",
            r#"{"ts":"2026-08-25T00:00:01+00:00","seq":2,"event":{"type":"llm_chunk","delta":"b"}}"#,
            "\n"
        );
        let outcome = read_trace_content(content);
        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.entries[0].seq, 1);
        assert_eq!(
            outcome.entries[0].ts.as_deref(),
            Some("2026-08-25T00:00:00+00:00")
        );
        assert_eq!(outcome.entries[1].seq, 2);
        assert!(!outcome.truncated_tail);
        assert!(outcome.is_monotonic_by_file_order());
        insta::assert_debug_snapshot!(
            outcome
                .entries
                .iter()
                .map(|record| (
                    record.seq,
                    record.ts.clone(),
                    match &record.entry {
                        TraceEntry::Ui(event) => event.event_name().to_string(),
                        TraceEntry::History(_) => "history".to_string(),
                    }
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn legacy_bare_event_lines_fall_back_to_line_number_seq() {
        let content = concat!(
            r#"{"type":"run_started","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","job_id":"01ARZ3NDEKTSV4RRFFQ69G5FAW","user_message":"hi"}"#,
            "\n",
            r#"{"type":"llm_chunk","delta":"x"}"#,
            "\n"
        );
        let outcome = read_trace_content(content);
        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.entries[0].seq, 1);
        assert_eq!(outcome.entries[1].seq, 2);
        assert!(outcome.entries.iter().all(|record| record.ts.is_none()));
        assert_eq!(outcome.corrupt_line_count, 0);
        assert!(!outcome.has_explicit_history());
    }

    #[test]
    fn mixed_legacy_and_enveloped_lines_read_in_one_pass() {
        let content = concat!(
            r#"{"type":"llm_chunk","delta":"legacy"}"#,
            "\n",
            r#"{"ts":"2026-08-25T00:00:02+00:00","seq":42,"event":{"type":"llm_chunk","delta":"enveloped"}}"#,
            "\n"
        );
        let outcome = read_trace_content(content);
        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.entries[0].seq, 1);
        assert_eq!(outcome.entries[1].seq, 42);
        assert!(outcome.is_monotonic_by_file_order());
    }

    #[test]
    fn explicit_history_lines_decode_into_the_history_stream() {
        let content = concat!(
            r#"{"ts":"2026-08-25T00:00:00+00:00","seq":1,"event":{"kind":"message","role":"user","content":"fix the bug"}}"#,
            "\n",
            r#"{"ts":"2026-08-25T00:00:01+00:00","seq":2,"event":{"type":"llm_chunk","delta":"thinking"}}"#,
            "\n",
            r#"{"ts":"2026-08-25T00:00:02+00:00","seq":3,"event":{"kind":"message","role":"assistant","content":"done"}}"#,
            "\n"
        );
        let outcome = read_trace_content(content);
        assert_eq!(outcome.entries.len(), 3);
        assert_eq!(outcome.history_items.len(), 2);
        assert_eq!(outcome.history_items[0].seq, 1);
        assert_eq!(outcome.history_items[1].seq, 3);
        assert!(outcome.has_explicit_history());
        let messages = rove_core::history::history_to_messages(&[
            outcome.history_items[0].item.clone(),
            outcome.history_items[1].item.clone(),
        ]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "fix the bug");
        assert_eq!(messages[1].content, "done");
    }

    #[test]
    fn writer_round_trips_history_and_ui_payloads_in_sequence_order() {
        let tmp = tempfile::TempDir::new().unwrap();
        let writer = TraceWriter::new(&tmp.path().join("runs").join("r1")).unwrap();
        writer.append(&sample_event("ui-one")).unwrap();
        writer
            .append_history(&sample_history_item("assistant text"))
            .unwrap();
        let outcome =
            read_trace_file_sync(&tmp.path().join("runs").join("r1").join("trace.jsonl")).unwrap();
        let seqs: Vec<u64> = outcome.entries.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![1, 2]);
        assert_eq!(outcome.history_items.len(), 1);
        assert!(matches!(
            &outcome.entries[0].entry,
            TraceEntry::Ui(StreamEvent::LlmChunk { .. })
        ));
        assert!(matches!(
            &outcome.entries[1].entry,
            TraceEntry::History(HistoryItem::Message(_))
        ));
    }

    #[test]
    fn truncated_final_line_is_skipped_and_reported() {
        // Simulates a kill mid-write: the final line is a partial JSON object.
        let content = concat!(
            r#"{"ts":"2026-08-25T00:00:00+00:00","seq":1,"event":{"type":"llm_chunk","delta":"ok"}}"#,
            "\n",
            r#"{"ts":"2026-08-25T00:00:01+00:00","seq":2,"event":{"type":"llm_chu"#,
            "\n"
        );
        let outcome = read_trace_content(content);
        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.entries[0].seq, 1);
        assert!(outcome.truncated_tail);
        assert_eq!(outcome.corrupt_line_count, 1);
    }

    #[test]
    fn interior_corruption_is_counted_without_truncated_flag() {
        let content = concat!(
            "not json at all\n",
            r#"{"type":"llm_chunk","delta":"after"}"#,
            "\n",
            r#"{"ts":"2026-08-25T00:00:01+00:00","seq":9,"event":{"type":"llm_chunk","delta":"end"}}"#,
            "\n"
        );
        let outcome = read_trace_content(content);
        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.corrupt_line_count, 1);
        assert_eq!(outcome.corrupt_line_numbers, vec![1]);
        assert!(!outcome.truncated_tail);
    }

    #[test]
    fn writer_assigns_continuous_seq_from_memory_counter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let writer = TraceWriter::new(&tmp.path().join("runs").join("r1")).unwrap();
        writer.append(&sample_event("one")).unwrap();
        writer.append(&sample_event("two")).unwrap();
        let outcome =
            read_trace_file_sync(&tmp.path().join("runs").join("r1").join("trace.jsonl")).unwrap();
        let seqs: Vec<u64> = outcome.entries.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![1, 2]);
        assert!(
            outcome
                .entries
                .iter()
                .all(|record| record.ts.as_ref().is_some_and(|ts| !ts.is_empty()))
        );
        // Envelope continuity holds even after an explicit-seq append.
        writer
            .append_with_seq(7, &sample_event("explicit"))
            .unwrap();
        writer.append(&sample_event("three")).unwrap();
        let outcome =
            read_trace_file_sync(&tmp.path().join("runs").join("r1").join("trace.jsonl")).unwrap();
        let seqs: Vec<u64> = outcome.entries.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![1, 2, 7, 8]);
    }

    #[tokio::test]
    async fn writer_seeds_counter_from_index_once_and_keeps_sse_payload_stable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_dir = tmp.path().to_path_buf();
        let index = StateIndex::new(&state_dir);
        let run_id = RunId::new();
        let run_dir = state_dir.join("runs").join(run_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let trace_path = run_dir.join("trace.jsonl");
        // The events table carries a foreign key on runs; register the run.
        index
            .record_run_started(
                SessionId::new(),
                JobId::new(),
                run_id,
                &run_dir,
                &trace_path,
            )
            .unwrap();

        let writer = TraceWriter::for_run(&run_dir, run_id, index.clone()).unwrap();
        writer.append(&sample_event("a")).unwrap();
        writer.append(&sample_event("b")).unwrap();

        // A fresh writer resumes numbering from the durable high-water mark.
        drop(writer);
        let resumed = TraceWriter::for_run(&run_dir, run_id, index.clone()).unwrap();
        resumed.append(&sample_event("c")).unwrap();
        assert_eq!(index.last_event_seq(run_id).unwrap(), 3);

        // The index stores bare event JSON so SSE consumers keep their shape.
        let records = index.event_records(run_id).unwrap();
        assert_eq!(records.len(), 3);
        for record in &records {
            let event: StreamEvent = serde_json::from_str(&record.event_json)
                .expect("index payload must stay a bare StreamEvent");
            let _ = event.event_name();
        }
    }

    #[tokio::test]
    async fn history_lines_advance_the_index_high_water_mark_without_event_rows() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_dir = tmp.path().to_path_buf();
        let index = StateIndex::new(&state_dir);
        let run_id = RunId::new();
        let run_dir = state_dir.join("runs").join(run_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let trace_path = run_dir.join("trace.jsonl");
        index
            .record_run_started(
                SessionId::new(),
                JobId::new(),
                run_id,
                &run_dir,
                &trace_path,
            )
            .unwrap();

        let writer = TraceWriter::for_run(&run_dir, run_id, index.clone()).unwrap();
        writer.append(&sample_event("ui")).unwrap();
        writer
            .append_history(&sample_history_item("visible"))
            .unwrap();
        drop(writer);

        // The history line consumed seq 2 without inserting an event row, but
        // the high-water mark moved so a restarted writer cannot reuse it.
        assert_eq!(index.last_event_seq(run_id).unwrap(), 2);
        assert_eq!(index.event_records(run_id).unwrap().len(), 1);

        let resumed = TraceWriter::for_run(&run_dir, run_id, index.clone()).unwrap();
        resumed.append(&sample_event("after-restart")).unwrap();
        let outcome = read_trace_file_sync(&trace_path).unwrap();
        let seqs: Vec<u64> = outcome.entries.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
    }
}
