//! Where a run's model context comes from, resolved before the run starts.
//!
//! A run begins in exactly one of three ways, and conflating them is what makes
//! resume logic drift: a fresh conversation, a continuation of a run that
//! stopped, or a branch off a run that may still be going. [`InitialHistory`]
//! makes the caller name which one, so the engine never has to infer it from
//! whether some optional field happened to be populated.
//!
//! The history itself is read from `trace.jsonl`, which is authoritative — it
//! holds the explicit [`HistoryItem`] stream written during the original run.
//! Only the tail is read, via [`ReverseJsonlScanner`], because model context is
//! bounded while a trace is not.

use std::path::Path;

use rove_core::history::HistoryItem;

use crate::events::{TraceEntry, TraceLink};
use crate::types::RunId;

use super::reverse_trace_scanner::{ReverseJsonlScanner, ScanOutcome};
use super::trace::TraceLine;

/// History items to carry into a resumed or forked run by default.
///
/// Generous enough that ordinary conversations are unaffected, bounded so a
/// very long run cannot make startup cost grow without limit.
pub const DEFAULT_HISTORY_TAIL_ITEMS: usize = 400;

/// Ceiling on a single trace record. Records above it are skipped rather than
/// buffered, so one pathological tool payload cannot dominate startup memory.
const MAX_TRACE_RECORD_BYTES: usize = 8 * 1024 * 1024;

/// How a run's history begins.
#[derive(Debug, Clone)]
pub enum InitialHistory {
    /// A fresh conversation with no prior context.
    New,
    /// A continuation of `from_run`, whose history this run inherits.
    Resumed(ResumedHistory),
    /// A branch off an existing run. The source keeps its own history; this run
    /// gets an independent copy.
    Forked(ResumedHistory),
}

impl InitialHistory {
    /// The inherited items, oldest first. Empty for [`InitialHistory::New`].
    pub fn items(&self) -> &[HistoryItem] {
        match self {
            Self::New => &[],
            Self::Resumed(history) | Self::Forked(history) => &history.items,
        }
    }

    /// The run this history came from, if any.
    pub fn source_run(&self) -> Option<RunId> {
        match self {
            Self::New => None,
            Self::Resumed(history) | Self::Forked(history) => Some(history.from_run),
        }
    }

    /// Provider-neutral messages for the first model request of the new run,
    /// with any interrupted tool round closed.
    pub fn to_messages(&self) -> Vec<rove_models::Message> {
        let mut messages = rove_core::history::history_to_messages(self.items());
        close_unresolved_tool_calls(&mut messages);
        messages
    }
}

/// History inherited from an earlier run, with what the read did or could not
/// establish.
#[derive(Debug, Clone)]
pub struct ResumedHistory {
    /// The run this history was read from.
    pub from_run: RunId,
    /// Inherited items in replay order (oldest first).
    pub items: Vec<HistoryItem>,
    /// Highest sequence number seen in the source trace — the hand-off point.
    pub through_seq: u64,
    /// True when the read stopped at a bound rather than at the start of the
    /// conversation, so `items` is a suffix. A compaction marker also ends the
    /// read, and counts as complete: everything before it is already summarised.
    pub truncated: bool,
    /// Records that could not be decoded. A torn tail from a crash normally
    /// accounts for exactly one.
    pub corrupt_record_count: usize,
    /// The resume link opening the source trace, when the source was itself a
    /// resumed run. Following these backwards walks the whole chain.
    pub source_link: Option<TraceLink>,
}

impl ResumedHistory {
    /// A complete history that happens to contain nothing.
    fn empty(from_run: RunId) -> Self {
        Self {
            from_run,
            items: Vec::new(),
            through_seq: 0,
            truncated: false,
            corrupt_record_count: 0,
            source_link: None,
        }
    }

    /// Whether the inherited history reaches back to the start of the
    /// conversation (directly, or through a compaction summary).
    pub fn is_complete(&self) -> bool {
        !self.truncated
    }
}

/// Which run, if any, the new run inherits history from.
#[derive(Debug, Clone, Copy)]
pub enum HistorySource {
    /// Nothing to inherit.
    New,
    /// Continue `run_id`.
    Resume(RunId),
    /// Branch off `run_id`.
    Fork(RunId),
}

/// Resolve a run's starting history, reading only as much trace tail as needed.
///
/// `run_dir_for` maps a run id to its directory, so this stays independent of
/// the store layout. A missing or empty trace yields empty history rather than
/// an error: a run that crashed before writing anything is resumable, just with
/// nothing to inherit.
pub fn get_initial_history(
    source: HistorySource,
    run_dir_for: impl Fn(RunId) -> std::path::PathBuf,
    max_items: usize,
) -> std::io::Result<InitialHistory> {
    let (run_id, fork) = match source {
        HistorySource::New => return Ok(InitialHistory::New),
        HistorySource::Resume(run_id) => (run_id, false),
        HistorySource::Fork(run_id) => (run_id, true),
    };

    let trace_path = run_dir_for(run_id).join("trace.jsonl");
    let history = read_history_tail(&trace_path, run_id, max_items)?;
    Ok(if fork {
        InitialHistory::Forked(history)
    } else {
        InitialHistory::Resumed(history)
    })
}

/// The highest sequence number durably recorded in one trace.
///
/// This is the hand-off point a resumed run records in its own trace. The
/// writer allocates sequences monotonically and appends in order, so the last
/// intact record carries the high-water mark and one bounded tail read settles
/// it. Returns `0` for a trace that is missing or holds nothing readable, which
/// is the right answer: nothing was durably recorded.
pub fn read_trace_high_water_seq(trace_path: &Path) -> std::io::Result<u64> {
    let file = match std::fs::File::open(trace_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut scanner = ReverseJsonlScanner::new(std::io::BufReader::new(file))?
        .with_max_record_bytes(MAX_TRACE_RECORD_BYTES);
    while let Some(outcome) = scanner.scan_next::<TraceLine>()? {
        // A torn final record from a crash is skipped; the one before it is
        // still the durable high-water mark.
        if let ScanOutcome::Parsed(line) = outcome {
            return Ok(line.seq);
        }
    }
    Ok(0)
}

/// Read the last `max_items` history items of one trace, tail first.
pub fn read_history_tail(
    trace_path: &Path,
    run_id: RunId,
    max_items: usize,
) -> std::io::Result<ResumedHistory> {
    let file = match std::fs::File::open(trace_path) {
        Ok(file) => file,
        // A run that never wrote a trace inherits nothing; that is not a
        // failure, and forcing the caller to special-case it would only push
        // the same decision outwards.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ResumedHistory::empty(run_id));
        }
        Err(error) => return Err(error),
    };
    read_history_tail_from(std::io::BufReader::new(file), run_id, max_items)
}

/// Read a history tail from any seekable JSONL source.
///
/// The trace on disk is the only production source, but keeping the scan
/// independent of the filesystem is what makes the read's cost measurable: a
/// caller can wrap the reader and observe that a bounded tail touches a bounded
/// number of bytes regardless of how large the source is.
pub fn read_history_tail_from<R: std::io::Read + std::io::Seek>(
    source: R,
    run_id: RunId,
    max_items: usize,
) -> std::io::Result<ResumedHistory> {
    let mut history = ResumedHistory::empty(run_id);
    let mut scanner =
        ReverseJsonlScanner::new(source)?.with_max_record_bytes(MAX_TRACE_RECORD_BYTES);
    // Collected newest-first, reversed once at the end.
    let mut reversed = Vec::new();
    loop {
        if reversed.len() >= max_items {
            history.truncated = true;
            break;
        }
        let Some(outcome) = scanner.scan_next::<TraceLine>()? else {
            break;
        };
        let line = match outcome {
            ScanOutcome::Parsed(line) => line,
            ScanOutcome::Rejected(_) => {
                history.corrupt_record_count += 1;
                continue;
            }
        };
        history.through_seq = history.through_seq.max(line.seq);
        match line.event {
            TraceEntry::History(item) => {
                let compacted = matches!(item, HistoryItem::Compacted(_));
                reversed.push(item);
                if compacted {
                    // Everything older is already represented by this summary,
                    // so the scan is finished rather than cut short.
                    break;
                }
            }
            // The source run was itself resumed. Recorded so a caller can walk
            // the chain further back; not followed here, because how much of an
            // ancestor to inherit is the caller's policy, not this reader's.
            TraceEntry::Link(link) => history.source_link = Some(link),
            // Neither carries model-visible history: one is presentation, the
            // other is the file's own identity header.
            TraceEntry::Ui(_) | TraceEntry::Meta(_) => {}
        }
    }

    reversed.reverse();
    history.items = reversed;
    Ok(history)
}

/// Close tool calls that have no recorded result, so replayed history is a
/// shape a provider will accept.
///
/// A run interrupted between dispatching a tool call and recording its result
/// leaves an assistant message whose calls are unanswered. Providers that pair
/// calls with results reject that, and dropping the assistant message instead
/// would erase the model's own reasoning. So each unanswered call gains an
/// explicit unknown-effect result: replay is refused rather than assumed, and
/// the call identity survives for audit.
///
/// This mirrors what `Session::close_unresolved_tool_calls` does for canonical
/// checkpoints, at the `Message` level the trace path works in.
pub fn close_unresolved_tool_calls(messages: &mut Vec<rove_models::Message>) -> usize {
    use rove_models::{Message, Role};

    let answered: std::collections::BTreeSet<String> = messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .filter_map(|message| message.tool_call_id.clone())
        .collect();

    // Walk backwards so each repair is inserted directly after the assistant
    // message that made the call, leaving earlier indices untouched.
    let mut repaired = 0usize;
    for index in (0..messages.len()).rev() {
        if messages[index].role != Role::Assistant {
            continue;
        }
        let unanswered: Vec<_> = messages[index]
            .tool_calls
            .iter()
            .filter(|call| !answered.contains(&call.id))
            .cloned()
            .collect();
        for call in unanswered.into_iter().rev() {
            let mut result = Message::tool(
                format!(
                    "[interrupted] `{}` was dispatched but its result was never recorded. \
                     Its effect on the workspace is unknown; verify before relying on it.",
                    call.name
                ),
                Some(call.id.clone()),
            );
            result.tool_name = Some(call.name.clone());
            messages.insert(index + 1, result);
            repaired += 1;
        }
    }
    repaired
}

/// Upper bound on how many ancestors [`read_history_chain`] will follow.
///
/// A chain is built one resume at a time, so it is naturally short. The bound
/// exists so a link cycle written by a bug cannot hang startup; visited-run
/// tracking handles the cycle itself, and this covers pathological depth.
pub const MAX_RESUME_CHAIN_DEPTH: usize = 64;

/// One trace in a resume chain, with where it sat in the walk.
#[derive(Debug, Clone)]
pub struct ChainSegment {
    /// The run this segment was read from.
    pub run_id: RunId,
    /// The segment's own history, in replay order.
    pub history: ResumedHistory,
}

/// A resume chain flattened into one continuously replayable history.
#[derive(Debug, Clone)]
pub struct HistoryChain {
    /// Segments oldest-run first, matching replay order.
    pub segments: Vec<ChainSegment>,
    /// Every segment's items concatenated, oldest first.
    pub items: Vec<HistoryItem>,
    /// True when the walk stopped at a bound (item budget, depth cap, a cycle,
    /// or a truncated segment) rather than at a run that began fresh.
    pub truncated: bool,
}

impl HistoryChain {
    /// Provider-neutral messages for the first model request of the new run,
    /// with any interrupted tool round closed.
    pub fn to_messages(&self) -> Vec<rove_models::Message> {
        let mut messages = rove_core::history::history_to_messages(&self.items);
        close_unresolved_tool_calls(&mut messages);
        messages
    }

    /// Whether the chain reaches back to a run that started fresh.
    pub fn is_complete(&self) -> bool {
        !self.truncated
    }
}

/// Walk a resume chain backwards from `run_id`, newest run first, and return
/// the whole thing as one replayable history.
///
/// rove owns a directory per run, so a resumed run writes its own trace and
/// records a [`TraceLink::ResumedFrom`] marker instead of appending to its
/// predecessor's file. Replaying a resumed session therefore means replaying
/// several traces in order, which is what this reconstructs. `max_items` is a
/// budget across the whole chain, not per segment, so a long chain costs no
/// more to open than a single long run.
pub fn read_history_chain(
    run_id: RunId,
    run_dir_for: impl Fn(RunId) -> std::path::PathBuf,
    max_items: usize,
) -> std::io::Result<HistoryChain> {
    let mut chain = HistoryChain {
        segments: Vec::new(),
        items: Vec::new(),
        truncated: false,
    };
    let mut visited = std::collections::HashSet::new();
    let mut next = Some(run_id);
    let mut remaining = max_items;

    while let Some(current) = next {
        if !visited.insert(current) {
            // A cycle can only come from a corrupt or buggy link. Stopping and
            // reporting a truncated chain keeps startup finite and honest.
            chain.truncated = true;
            break;
        }
        if chain.segments.len() >= MAX_RESUME_CHAIN_DEPTH {
            chain.truncated = true;
            break;
        }
        if remaining == 0 {
            chain.truncated = true;
            break;
        }

        let trace_path = run_dir_for(current).join("trace.jsonl");
        let history = read_history_tail(&trace_path, current, remaining)?;
        remaining = remaining.saturating_sub(history.items.len());
        let segment_truncated = history.truncated;
        let ancestor = history
            .source_link
            .as_ref()
            .map(|TraceLink::ResumedFrom { from_run, .. }| *from_run);
        // A compaction marker ends a segment's read as complete, and it also
        // makes the ancestors redundant: the summary already stands in for
        // them. Walking further back would double-count that history.
        let stops_at_compaction = matches!(history.items.first(), Some(HistoryItem::Compacted(_)));

        chain.segments.push(ChainSegment {
            run_id: current,
            history,
        });

        if segment_truncated {
            chain.truncated = true;
            break;
        }
        next = if stops_at_compaction { None } else { ancestor };
    }

    // Segments were collected newest-run first; replay wants the oldest first.
    chain.segments.reverse();
    chain.items = chain
        .segments
        .iter()
        .flat_map(|segment| segment.history.items.iter().cloned())
        .collect();
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::trace::TraceWriter;
    use crate::types::RunId;

    fn message(text: &str) -> HistoryItem {
        HistoryItem::Message(rove_models::Message::assistant(text))
    }

    fn ui_event(delta: &str) -> crate::events::StreamEvent {
        crate::events::StreamEvent::LlmChunk {
            delta: delta.to_string(),
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::TempDir::new().unwrap();
            let root = temp.path().to_path_buf();
            Self { _temp: temp, root }
        }

        fn run_dir(&self, run_id: RunId) -> std::path::PathBuf {
            self.root.join(run_id.to_string())
        }

        fn writer(&self, run_id: RunId) -> TraceWriter {
            TraceWriter::new(&self.run_dir(run_id)).unwrap()
        }

        fn resolve(&self, source: HistorySource, max_items: usize) -> InitialHistory {
            let root = self.root.clone();
            get_initial_history(source, |run_id| root.join(run_id.to_string()), max_items).unwrap()
        }

        fn chain(&self, run_id: RunId, max_items: usize) -> HistoryChain {
            let root = self.root.clone();
            read_history_chain(run_id, |run_id| root.join(run_id.to_string()), max_items).unwrap()
        }
    }

    #[test]
    fn a_new_run_inherits_nothing_and_names_no_source() {
        let fixture = Fixture::new();
        let history = fixture.resolve(HistorySource::New, 10);
        assert!(history.items().is_empty());
        assert_eq!(history.source_run(), None);
        assert!(matches!(history, InitialHistory::New));
    }

    /// The reader must return items in replay order even though it walks the
    /// file backwards, and must ignore the UI events interleaved with them.
    #[test]
    fn resumed_history_comes_back_in_replay_order_without_ui_events() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append_history(&message("first")).unwrap();
        writer.append(&ui_event("noise")).unwrap();
        writer.append_history(&message("second")).unwrap();
        writer.append(&ui_event("more noise")).unwrap();
        writer.append_history(&message("third")).unwrap();

        let history = fixture.resolve(HistorySource::Resume(run_id), 100);

        let texts: Vec<String> = history
            .to_messages()
            .into_iter()
            .map(|message| message.content)
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
        assert_eq!(history.source_run(), Some(run_id));
        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert!(resumed.is_complete());
        assert_eq!(resumed.corrupt_record_count, 0);
    }

    /// A fork and a resume read identically; only the reported intent differs,
    /// so the engine can treat the source run differently without re-deriving
    /// which case it is in.
    #[test]
    fn a_fork_reads_the_same_history_but_reports_a_distinct_intent() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append_history(&message("shared")).unwrap();

        let resumed = fixture.resolve(HistorySource::Resume(run_id), 100);
        let forked = fixture.resolve(HistorySource::Fork(run_id), 100);

        assert_eq!(resumed.items().len(), forked.items().len());
        assert!(matches!(resumed, InitialHistory::Resumed(_)));
        assert!(matches!(forked, InitialHistory::Forked(_)));
    }

    /// Only the tail is inherited, and the caller is told the history was cut
    /// so it can decide whether that is acceptable.
    #[test]
    fn a_bounded_read_keeps_the_newest_items_and_reports_truncation() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        for n in 0..20 {
            writer
                .append_history(&message(&format!("item-{n}")))
                .unwrap();
        }

        let history = fixture.resolve(HistorySource::Resume(run_id), 3);

        let texts: Vec<String> = history
            .to_messages()
            .into_iter()
            .map(|message| message.content)
            .collect();
        assert_eq!(texts, vec!["item-17", "item-18", "item-19"]);
        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert!(!resumed.is_complete());
    }

    /// A crash mid-write leaves a torn final line. It must be counted and
    /// skipped, never allowed to hide the intact history in front of it.
    #[test]
    fn a_torn_tail_is_reported_and_the_history_before_it_survives() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append_history(&message("durable")).unwrap();
        let trace_path = fixture.run_dir(run_id).join("trace.jsonl");
        let mut content = std::fs::read_to_string(&trace_path).unwrap();
        content.push_str("{\"ts\":\"2026-08-26T00:00:00Z\",\"seq\":9,\"eve");
        std::fs::write(&trace_path, content).unwrap();

        let history = fixture.resolve(HistorySource::Resume(run_id), 100);

        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert_eq!(resumed.corrupt_record_count, 1);
        assert_eq!(resumed.items.len(), 1);
        assert_eq!(history.to_messages()[0].content, "durable");
    }

    /// Compaction already summarises everything older, so the scan stops there
    /// and the result is complete rather than truncated.
    #[test]
    fn a_compaction_marker_ends_the_scan_and_still_counts_as_complete() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append_history(&message("ancient")).unwrap();
        writer
            .append_history(&HistoryItem::Compacted(rove_core::history::CompactedItem {
                summary: "earlier turns summarised".to_string(),
                covered_messages: 1_u32,
            }))
            .unwrap();
        writer.append_history(&message("recent")).unwrap();

        let history = fixture.resolve(HistorySource::Resume(run_id), 100);

        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert!(resumed.is_complete());
        // The pre-compaction item is not inherited: the summary stands in for it.
        assert_eq!(resumed.items.len(), 2);
        let texts: Vec<String> = history
            .to_messages()
            .into_iter()
            .map(|message| message.content)
            .collect();
        assert_eq!(
            texts,
            vec![
                "[conversation compacted] earlier turns summarised",
                "recent"
            ]
        );
    }

    #[test]
    fn a_run_that_never_wrote_a_trace_is_resumable_with_empty_history() {
        let fixture = Fixture::new();
        let history = fixture.resolve(HistorySource::Resume(RunId::new()), 100);
        assert!(history.items().is_empty());
        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert!(resumed.is_complete());
        assert_eq!(resumed.through_seq, 0);
    }

    /// The resume link opening a trace is surfaced so a caller can walk further
    /// back along the chain.
    #[test]
    fn a_source_that_was_itself_resumed_reports_its_own_link() {
        let fixture = Fixture::new();
        let ancestor = RunId::new();
        let middle = RunId::new();
        let writer = fixture.writer(middle);
        writer.append_resume_link(ancestor, 12).unwrap();
        writer.append_history(&message("continued")).unwrap();

        let history = fixture.resolve(HistorySource::Resume(middle), 100);

        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert_eq!(
            resumed.source_link,
            Some(TraceLink::ResumedFrom {
                from_run: ancestor,
                through_seq: 12,
            })
        );
    }

    /// The headline acceptance item: a session resumed twice replays as one
    /// continuous conversation, in original order, across three trace files.
    #[test]
    fn a_twice_resumed_session_replays_continuously_across_its_whole_chain() {
        let fixture = Fixture::new();
        let first = RunId::new();
        let second = RunId::new();
        let third = RunId::new();

        let writer = fixture.writer(first);
        writer.append_history(&message("turn-1")).unwrap();
        writer.append_history(&message("turn-2")).unwrap();

        let writer = fixture.writer(second);
        writer.append_resume_link(first, 2).unwrap();
        writer.append_history(&message("turn-3")).unwrap();

        let writer = fixture.writer(third);
        writer.append_resume_link(second, 2).unwrap();
        writer.append_history(&message("turn-4")).unwrap();

        let chain = fixture.chain(third, 100);

        let texts: Vec<String> = chain
            .to_messages()
            .into_iter()
            .map(|message| message.content)
            .collect();
        assert_eq!(texts, vec!["turn-1", "turn-2", "turn-3", "turn-4"]);
        assert!(chain.is_complete());
        // Oldest run first, so the segment order matches the replay order.
        let runs: Vec<RunId> = chain
            .segments
            .iter()
            .map(|segment| segment.run_id)
            .collect();
        assert_eq!(runs, vec![first, second, third]);
    }

    /// The item budget spans the chain rather than each segment, so opening a
    /// long chain costs no more than opening one long run.
    #[test]
    fn the_item_budget_is_shared_across_the_chain_and_reports_truncation() {
        let fixture = Fixture::new();
        let older = RunId::new();
        let newer = RunId::new();

        let writer = fixture.writer(older);
        for n in 0..10 {
            writer
                .append_history(&message(&format!("old-{n}")))
                .unwrap();
        }
        let writer = fixture.writer(newer);
        writer.append_resume_link(older, 10).unwrap();
        writer.append_history(&message("new-0")).unwrap();
        writer.append_history(&message("new-1")).unwrap();

        let chain = fixture.chain(newer, 4);

        let texts: Vec<String> = chain
            .to_messages()
            .into_iter()
            .map(|message| message.content)
            .collect();
        // Two from the newest run, then the budget's remainder from its parent.
        assert_eq!(texts, vec!["old-8", "old-9", "new-0", "new-1"]);
        assert!(!chain.is_complete());
    }

    /// A compaction summary already stands in for everything older, so the walk
    /// stops there instead of replaying the ancestors it summarises.
    #[test]
    fn a_compacted_segment_ends_the_walk_without_replaying_its_ancestors() {
        let fixture = Fixture::new();
        let older = RunId::new();
        let newer = RunId::new();

        let writer = fixture.writer(older);
        writer.append_history(&message("pre-compaction")).unwrap();
        let writer = fixture.writer(newer);
        writer.append_resume_link(older, 1).unwrap();
        writer
            .append_history(&HistoryItem::Compacted(rove_core::history::CompactedItem {
                summary: "everything so far".to_string(),
                covered_messages: 1_u32,
            }))
            .unwrap();
        writer.append_history(&message("after")).unwrap();

        let chain = fixture.chain(newer, 100);

        assert_eq!(chain.segments.len(), 1);
        assert!(chain.is_complete());
        let texts: Vec<String> = chain
            .to_messages()
            .into_iter()
            .map(|message| message.content)
            .collect();
        assert_eq!(
            texts,
            vec!["[conversation compacted] everything so far", "after"]
        );
    }

    /// A run killed between dispatching a tool call and recording its result
    /// must still replay into a shape a provider accepts.
    #[test]
    fn an_interrupted_tool_round_is_closed_before_replay() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append_history(&message("thinking")).unwrap();
        writer
            .append_history(&HistoryItem::Message(
                rove_models::Message::assistant_with_tool_calls(
                    "reading the file",
                    vec![rove_models::ToolCallRef {
                        id: "call_1".to_string(),
                        name: "fs_read".to_string(),
                        args: serde_json::json!({"path": "a.rs"}),
                    }],
                ),
            ))
            .unwrap();

        let messages = fixture
            .resolve(HistorySource::Resume(run_id), 100)
            .to_messages();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].role, rove_models::Role::Tool);
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_1"));
        assert!(messages[2].content.contains("interrupted"));
        // The effect is reported as unknown rather than assumed either way.
        assert!(messages[2].content.contains("unknown"));
    }

    /// A tool round that did complete must not be touched.
    #[test]
    fn a_completed_tool_round_is_replayed_unchanged() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer
            .append_history(&HistoryItem::Message(
                rove_models::Message::assistant_with_tool_calls(
                    "reading",
                    vec![rove_models::ToolCallRef {
                        id: "call_1".to_string(),
                        name: "fs_read".to_string(),
                        args: serde_json::json!({}),
                    }],
                ),
            ))
            .unwrap();
        writer
            .append_history(&HistoryItem::Message(rove_models::Message::tool(
                "file body",
                Some("call_1".to_string()),
            )))
            .unwrap();

        let messages = fixture
            .resolve(HistorySource::Resume(run_id), 100)
            .to_messages();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "file body");
    }

    /// A corrupt link pointing at a run already in the chain must not hang
    /// startup.
    #[test]
    fn a_link_cycle_terminates_the_walk_instead_of_hanging() {
        let fixture = Fixture::new();
        let first = RunId::new();
        let second = RunId::new();

        let writer = fixture.writer(first);
        writer.append_resume_link(second, 1).unwrap();
        writer.append_history(&message("a")).unwrap();
        let writer = fixture.writer(second);
        writer.append_resume_link(first, 1).unwrap();
        writer.append_history(&message("b")).unwrap();

        let chain = fixture.chain(second, 100);

        assert_eq!(chain.segments.len(), 2);
        assert!(!chain.is_complete());
    }

    /// Counts the bytes a scan actually pulls, so a bound can be asserted
    /// rather than assumed.
    struct CountingReader {
        inner: std::io::Cursor<Vec<u8>>,
        bytes_read: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl std::io::Read for CountingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let read = self.inner.read(buf)?;
            self.bytes_read.set(self.bytes_read.get() + read);
            Ok(read)
        }
    }

    impl std::io::Seek for CountingReader {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    /// The acceptance criterion for opening a long-running session: cost is set
    /// by how much history is wanted, not by how much the run produced.
    #[test]
    fn a_large_trace_costs_only_the_tail_that_is_actually_wanted() {
        let run_id = RunId::new();
        // ~4 MB of history, far past any chunk or buffer size.
        let mut content = String::new();
        for seq in 1..=4_000u64 {
            let item = HistoryItem::Message(rove_models::Message::assistant("x".repeat(1_000)));
            let line = TraceLine {
                ts: "2026-08-26T00:00:00Z".to_string(),
                seq,
                event: TraceEntry::History(item),
            };
            content.push_str(&serde_json::to_string(&line).unwrap());
            content.push('\n');
        }
        let total_bytes = content.len();
        assert!(total_bytes > 4_000_000, "fixture is not large enough");

        let bytes_read = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let reader = CountingReader {
            inner: std::io::Cursor::new(content.into_bytes()),
            bytes_read: std::rc::Rc::clone(&bytes_read),
        };

        let history = read_history_tail_from(reader, run_id, 3).unwrap();

        assert_eq!(history.items.len(), 3);
        assert!(history.truncated);
        assert_eq!(history.through_seq, 4_000);
        // The bound is what matters: a few tail records must not drag the whole
        // file through memory. One chunk plus a slack allowance for a record
        // straddling the chunk boundary is the honest ceiling.
        let ceiling = super::super::reverse_trace_scanner::READ_CHUNK_SIZE * 2;
        assert!(
            bytes_read.get() <= ceiling,
            "read {} bytes of a {total_bytes}-byte trace for a 3-item tail; \
             the ceiling is {ceiling}",
            bytes_read.get(),
        );
    }

    /// The high-water read must survive a torn final line, and must not
    /// require reading the whole file.
    #[test]
    fn the_high_water_seq_skips_a_torn_tail_and_reads_a_missing_trace_as_zero() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append(&ui_event("one")).unwrap();
        writer.append_history(&message("two")).unwrap();
        let trace_path = fixture.run_dir(run_id).join("trace.jsonl");

        assert_eq!(read_trace_high_water_seq(&trace_path).unwrap(), 2);

        let mut content = std::fs::read_to_string(&trace_path).unwrap();
        content.push_str("{\"ts\":\"2026-08-26T00:00:00Z\",\"seq\":3,\"ev");
        std::fs::write(&trace_path, content).unwrap();
        assert_eq!(read_trace_high_water_seq(&trace_path).unwrap(), 2);

        let missing = fixture.run_dir(RunId::new()).join("trace.jsonl");
        assert_eq!(read_trace_high_water_seq(&missing).unwrap(), 0);
    }

    /// The hand-off point must reflect the source's own sequence space so a
    /// resumed run can record where it took over.
    #[test]
    fn the_handoff_sequence_is_the_highest_one_seen_in_the_source() {
        let fixture = Fixture::new();
        let run_id = RunId::new();
        let writer = fixture.writer(run_id);
        writer.append(&ui_event("one")).unwrap();
        writer.append_history(&message("two")).unwrap();
        writer.append(&ui_event("three")).unwrap();

        let history = fixture.resolve(HistorySource::Resume(run_id), 100);

        let InitialHistory::Resumed(resumed) = &history else {
            panic!("expected a resumed history");
        };
        assert_eq!(resumed.through_seq, 3);
    }
}
