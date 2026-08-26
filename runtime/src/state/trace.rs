use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::index::StateIndex;
use crate::events::{StreamEvent, TraceEntry};
use crate::types::RunId;

/// Self-describing envelope for one `trace.jsonl` line.
///
/// Codex-style: every line carries its own timestamp and monotonic sequence
/// number, so the file proves its own ordering without consulting SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceLine {
    /// RFC3339 UTC timestamp of when the line was written.
    pub ts: String,
    /// Monotonic per-run sequence assigned by the writer's in-memory counter.
    pub seq: u64,
    pub event: TraceEntry,
}

/// Sequence reserved for a trace's identity header.
///
/// Event sequences start at 1 because `after=0` means "everything", so 0 is the
/// one slot no event can occupy — which makes it the right home for a line that
/// describes the file instead of belonging to its stream.
pub const RUN_META_SEQ: u64 = 0;

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Manages trace file writing for a run.
///
/// Each run gets a `trace.jsonl` file with one [`TraceLine`] envelope per
/// line. Sequence numbers are allocated from an in-memory counter seeded once
/// from the state index, so the append path no longer queries SQLite per
/// event. The file remains authoritative; the index is a derived cache that
/// keeps SSE continuation working unchanged.
#[derive(Clone)]
pub struct TraceWriter {
    path: PathBuf,
    run_id: Option<RunId>,
    index: Option<StateIndex>,
    next_seq: Arc<AtomicU64>,
}

impl TraceWriter {
    /// Create a new trace writer for the given run directory.
    pub fn new(run_dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(run_dir)?;
        let path = run_dir.join("trace.jsonl");
        Ok(Self {
            path,
            run_id: None,
            index: None,
            next_seq: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn for_run(run_dir: &Path, run_id: RunId, index: StateIndex) -> std::io::Result<Self> {
        let mut writer = Self::new(run_dir)?;
        writer.run_id = Some(run_id);
        writer.index = Some(index.clone());
        // Seed the in-memory counter from the durable high-water mark exactly
        // once; subsequent appends never query the database again.
        let last = index.last_event_seq(run_id).unwrap_or(0);
        writer.next_seq = Arc::new(AtomicU64::new(last.saturating_add(1)));
        Ok(writer)
    }

    /// Append an event to the trace file with the next in-memory sequence.
    pub fn append(&self, event: &StreamEvent) -> std::io::Result<()> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        self.append_with_seq(seq, event)
    }

    /// Append an event with an interface-assigned sequence number.
    ///
    /// The in-memory counter is kept ahead of any explicitly provided seq so
    /// later counter-based appends cannot collide with it.
    pub fn append_with_seq(&self, seq: u64, event: &StreamEvent) -> std::io::Result<()> {
        self.next_seq
            .fetch_max(seq.saturating_add(1), Ordering::SeqCst);
        let line = TraceLine {
            ts: now_rfc3339(),
            seq,
            event: TraceEntry::Ui(event.clone()),
        };
        self.append_line(&line)?;
        if let (Some(index), Some(run_id)) = (&self.index, self.run_id) {
            // The index stores the bare event JSON so existing SSE/transcript
            // projections keep their wire format unchanged.
            let bare = serde_json::to_string(event).map_err(std::io::Error::other)?;
            index.append_event(run_id, seq, event, &bare)?;
        }
        Ok(())
    }

    /// Append an explicit model-visible history item (Phase 2 Codex
    /// alignment). History lines share the run's monotonic sequence space so
    /// file ordering stays provable, but they are not projected into the
    /// event index: they never travel on SSE/transcript replays. The index
    /// high-water mark is still advanced so a writer restart cannot reuse a
    /// sequence number already written to the trace file.
    pub fn append_history(&self, item: &rove_core::history::HistoryItem) -> std::io::Result<()> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let line = TraceLine {
            ts: now_rfc3339(),
            seq,
            event: TraceEntry::History(item.clone()),
        };
        self.append_line(&line)?;
        if let (Some(index), Some(run_id)) = (&self.index, self.run_id) {
            index.advance_event_seq(run_id, seq)?;
        }
        Ok(())
    }

    /// Open a resumed run's trace with the run it continues.
    ///
    /// rove owns a directory per run, so a resumed run gets its own trace file
    /// rather than appending to its predecessor's. This marker is what keeps
    /// the chain replayable from the files alone. It takes a sequence number
    /// like any other line, so the hand-off is itself ordered.
    pub fn append_resume_link(&self, from_run: RunId, through_seq: u64) -> std::io::Result<()> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let line = TraceLine {
            ts: now_rfc3339(),
            seq,
            event: TraceEntry::Link(crate::events::TraceLink::ResumedFrom {
                from_run,
                through_seq,
            }),
        };
        self.append_line(&line)?;
        if let (Some(index), Some(run_id)) = (&self.index, self.run_id) {
            index.advance_event_seq(run_id, seq)?;
        }
        Ok(())
    }

    /// Write the run's identity as the trace's opening line.
    ///
    /// Codex alignment Phase 5: without this, a run that died before its first
    /// `task_state.json` left a trace whose owning session was unknowable, so
    /// rebuilding a deleted index could not insert its `runs` row and the whole
    /// repair failed on a foreign key. Callers invoke this once, at run start.
    ///
    /// The line takes [`RUN_META_SEQ`] rather than drawing from the counter:
    /// the header is not an event, and `after=`/`Last-Event-ID` resumption is a
    /// wire contract keyed on the first event being seq 1. Spending a sequence
    /// number here would shift every event by one and silently replay
    /// `run_started` to a client that had already seen it.
    pub fn append_run_meta(
        &self,
        session_id: crate::types::SessionId,
        job_id: crate::types::JobId,
        run_id: RunId,
    ) -> std::io::Result<()> {
        let started_at = now_rfc3339();
        let line = TraceLine {
            ts: started_at.clone(),
            seq: RUN_META_SEQ,
            event: TraceEntry::Meta(crate::events::RunMeta::RunIdentity {
                session_id,
                job_id,
                run_id,
                started_at,
            }),
        };
        self.append_line(&line)?;
        Ok(())
    }

    fn append_line(&self, line: &TraceLine) -> std::io::Result<String> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let json = serde_json::to_string(line).map_err(std::io::Error::other)?;
        writeln!(file, "{}", json)?;
        Ok(json)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Manages the run directory structure under `.rove/runs/<run_id>/`.
pub struct RunStore {
    base_dir: PathBuf,
    index: Option<StateIndex>,
}

impl RunStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            base_dir: state_dir.join("runs"),
            index: None,
        }
    }

    pub fn with_index(state_dir: &Path, index: StateIndex) -> Self {
        Self {
            base_dir: state_dir.join("runs"),
            index: Some(index),
        }
    }

    /// Get the directory path for a specific run.
    pub fn run_dir(&self, run_id: &RunId) -> PathBuf {
        self.base_dir.join(run_id.to_string())
    }

    /// Create a trace writer for a new run.
    pub fn create_trace(&self, run_id: &RunId) -> std::io::Result<TraceWriter> {
        let run_dir = self.run_dir(run_id);
        if let Some(index) = &self.index {
            TraceWriter::for_run(&run_dir, *run_id, index.clone())
        } else {
            TraceWriter::new(&run_dir)
        }
    }
}
