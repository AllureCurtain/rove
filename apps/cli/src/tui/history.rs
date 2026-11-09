use crate::terminal::view::{MAX_RUN_TIMELINE_ENTRIES, RunViewState, ToolCallStatus};
use rove_runtime::events::StreamEvent;
use rove_runtime::state::index::StateIndex;
use rove_runtime::types::{RunId, SessionId, TerminationReason};

const MAX_HISTORY_EVENTS: usize = 2_048;
const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryView {
    pub run_id: Option<RunId>,
    pub text: String,
    pub notice: String,
    pub message: Option<String>,
    pub scroll: u16,
    pub max_scroll: u16,
}

impl Default for HistoryView {
    fn default() -> Self {
        Self {
            run_id: None,
            text: String::new(),
            notice: "Loading saved history...".into(),
            message: None,
            scroll: 0,
            max_scroll: 0,
        }
    }
}

pub struct SavedRun {
    pub view: RunViewState,
    pub notice: String,
}

pub async fn load_run(
    index: &StateIndex,
    session: SessionId,
    run_id: RunId,
) -> std::io::Result<SavedRun> {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        read_run(index, session, run_id),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "history read timed out"))?
}

async fn read_run(
    index: &StateIndex,
    session: SessionId,
    run_id: RunId,
) -> std::io::Result<SavedRun> {
    let mut view = RunViewState::default();
    let mut after = 0;
    let mut count = 0;
    let mut bytes = 0;
    let mut partial = false;
    let mut status = String::new();
    let mut terminal = None;
    loop {
        let snapshot = index
            .run_event_snapshot_async(run_id, after, 64)
            .await?
            .ok_or_else(|| std::io::Error::other("saved run is unavailable"))?;
        if snapshot.run.session_id != session || snapshot.run.run_id != run_id {
            return Err(std::io::Error::other("saved run identity does not match"));
        }
        status.clone_from(&snapshot.run.status);
        if snapshot.events.is_empty() {
            partial |= snapshot.high_water_seq > after;
            break;
        }
        for record in snapshot.events {
            count += 1;
            bytes += record.event_json.len();
            if count > MAX_HISTORY_EVENTS || bytes > MAX_HISTORY_BYTES {
                partial = true;
                break;
            }
            // Model-history and resume-link trace lines legitimately consume
            // sequence numbers without appearing in the UI event index.
            after = record.seq;
            let Ok(event) = serde_json::from_str::<StreamEvent>(&record.event_json) else {
                partial = true;
                continue;
            };
            if record.event_name != event.event_name() {
                partial = true;
                continue;
            }
            if let StreamEvent::RunStarted {
                run_id: event_run,
                job_id,
                ..
            } = &event
                && (*event_run != run_id || *job_id != snapshot.run.job_id)
            {
                return Err(std::io::Error::other("saved event identity does not match"));
            }
            // Child lifecycle events must not replace the parent's run identity
            // or turn a child's completion into the parent's completion.
            if matches!(event, StreamEvent::SubagentEvent { .. }) {
                partial = true;
                continue;
            }
            if terminal.is_some() {
                partial = true;
            }
            if let StreamEvent::RunCompleted { reason, .. } = &event {
                terminal = Some(reason.clone());
            }
            view.apply_event(&event);
        }
        if count > MAX_HISTORY_EVENTS || bytes > MAX_HISTORY_BYTES {
            break;
        }
        if !snapshot.has_more {
            partial |= after < snapshot.high_water_seq;
            break;
        }
    }
    partial |= view.timeline_high_watermark > MAX_RUN_TIMELINE_ENTRIES as u64;
    partial |= view.run_id.is_none();
    partial |= match (status.as_str(), terminal.as_ref()) {
        (
            "done",
            Some(
                TerminationReason::Final
                | TerminationReason::StepLimit
                | TerminationReason::TokenLimit
                | TerminationReason::TimeLimit,
            ),
        )
        | ("error", Some(TerminationReason::Error))
        | ("cancelled", Some(TerminationReason::Cancelled)) => false,
        ("done" | "error" | "cancelled", _) => true,
        (_, Some(_)) => true,
        _ => false,
    };
    view.run_id = Some(run_id);
    view.pending_approvals.clear();
    view.pending_inputs.clear();
    view.model_status = None;
    view.current_step = None;
    for tool in &mut view.tool_calls {
        if matches!(
            tool.status,
            ToolCallStatus::Started | ToolCallStatus::WaitingApproval
        ) {
            tool.status = ToolCallStatus::Interrupted;
        }
    }
    let status = match status.as_str() {
        "done" => "completed",
        "error" => "failed",
        "cancelled" => "cancelled",
        "interrupted" => "interrupted",
        _ => "unfinished",
    };
    let notice = if partial {
        format!(
            "Saved run ({status}); partial history: some events are missing, invalid, omitted or beyond display limits."
        )
    } else {
        format!("Saved run ({status}). F2 opens history; no recorded approvals can be acted on.")
    };
    view.saved_history_notice = Some(notice.clone());
    Ok(SavedRun { view, notice })
}

impl SavedRun {
    pub fn into_history(self) -> HistoryView {
        let mut lines = Vec::new();
        super::widgets::transcript::push_run_lines(&mut lines, &self.view);
        let text = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        HistoryView {
            run_id: self.view.run_id,
            text: super::sanitize::truncate_display_text(&text, 256 * 1024),
            notice: self.notice,
            message: None,
            scroll: 0,
            max_scroll: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_runtime::state::store::StateStore;
    use rove_runtime::types::{CallId, JobId, TerminationReason};

    #[tokio::test]
    async fn resume_links_do_not_make_complete_ui_history_partial() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::new(root.path());
        let session = SessionId::new();
        let job = JobId::new();
        let run_id = RunId::new();
        let run = store.start_run(session, job, run_id).unwrap();
        run.trace_writer
            .append_resume_link(RunId::new(), 7)
            .unwrap();
        run.trace_writer
            .append(&StreamEvent::RunStarted {
                run_id,
                job_id: job,
                user_message: "continue".into(),
                content_blocks: Vec::new(),
            })
            .unwrap();
        run.trace_writer
            .append(&StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("done".into()),
            })
            .unwrap();
        store
            .index
            .record_report(run_id, &run.run_dir.join("report.json"), "success", "final")
            .unwrap();
        assert!(
            !load_run(&store.index, session, run_id)
                .await
                .unwrap()
                .notice
                .contains("partial")
        );
    }

    #[tokio::test]
    async fn conflicting_terminal_status_and_events_after_completion_are_partial() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::new(root.path());
        let session = SessionId::new();
        let job = JobId::new();
        let run_id = RunId::new();
        let run = store.start_run(session, job, run_id).unwrap();
        run.trace_writer
            .append(&StreamEvent::RunStarted {
                run_id,
                job_id: job,
                user_message: "start".into(),
                content_blocks: Vec::new(),
            })
            .unwrap();
        run.trace_writer
            .append(&StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("success".into()),
            })
            .unwrap();
        store
            .index
            .record_report(
                run_id,
                &run.run_dir.join("report.json"),
                "cancelled",
                "cancelled",
            )
            .unwrap();
        assert!(
            load_run(&store.index, session, run_id)
                .await
                .unwrap()
                .notice
                .contains("partial")
        );
        store
            .index
            .record_report(run_id, &run.run_dir.join("report.json"), "success", "final")
            .unwrap();
        assert!(
            !load_run(&store.index, session, run_id)
                .await
                .unwrap()
                .notice
                .contains("partial")
        );
        run.trace_writer
            .append(&StreamEvent::LlmChunk {
                delta: "too late".into(),
            })
            .unwrap();
        assert!(
            load_run(&store.index, session, run_id)
                .await
                .unwrap()
                .notice
                .contains("partial")
        );
    }

    #[tokio::test]
    async fn saved_history_projects_events_without_restoring_controls() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::new(root.path());
        let session = SessionId::new();
        let job = JobId::new();
        let run_id = RunId::new();
        let run = store.start_run(session, job, run_id).unwrap();
        for event in [
            StreamEvent::RunStarted {
                run_id,
                job_id: job,
                user_message: "read this file".into(),
                content_blocks: Vec::new(),
            },
            StreamEvent::LlmChunk {
                delta: "visible answer".into(),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id: CallId::new(),
                name: "write_file".into(),
                args: serde_json::json!({"path":"test.txt"}),
                reason: "approval required".into(),
            },
            StreamEvent::InputNeeded {
                input_id: CallId::new(),
                prompt: "pick a value".into(),
            },
        ] {
            run.trace_writer.append(&event).unwrap();
        }
        let saved = load_run(&store.index, session, run_id).await.unwrap();
        assert_eq!(saved.view.assistant_text, "visible answer");
        assert!(saved.view.pending_approvals.is_empty());
        assert!(saved.view.pending_inputs.is_empty());
        assert!(saved.view.tool_calls.iter().all(|tool| !matches!(
            tool.status,
            ToolCallStatus::Started | ToolCallStatus::WaitingApproval
        )));
        assert!(saved.view.completed.is_none());
        assert!(saved.notice.contains("unfinished"));
        let history = saved.into_history();
        assert!(history.text.contains("visible answer"));
        assert!(history.text.contains("read this file"));
        assert!(
            load_run(&store.index, SessionId::new(), run_id)
                .await
                .is_err()
        );
        assert_eq!(
            store.index.job_record(job).unwrap().unwrap().status,
            "running"
        );
    }

    #[tokio::test]
    async fn corrupt_missing_and_nested_events_are_explicitly_partial() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::new(root.path());
        let session = SessionId::new();
        let job = JobId::new();
        let run_id = RunId::new();
        let run = store.start_run(session, job, run_id).unwrap();
        run.trace_writer
            .append(&StreamEvent::RunStarted {
                run_id,
                job_id: job,
                user_message: "parent".into(),
                content_blocks: Vec::new(),
            })
            .unwrap();
        store
            .index
            .append_event(
                run_id,
                3,
                &StreamEvent::LlmChunk {
                    delta: String::new(),
                },
                "broken",
            )
            .unwrap();
        let nested = StreamEvent::SubagentEvent {
            parent_call_id: CallId::new(),
            event: Box::new(StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("child done".into()),
            }),
        };
        store
            .index
            .append_event(run_id, 4, &nested, &serde_json::to_string(&nested).unwrap())
            .unwrap();
        let saved = load_run(&store.index, session, run_id).await.unwrap();
        assert!(saved.notice.contains("partial history"));
        assert_eq!(saved.view.run_id, Some(run_id));
        assert!(saved.view.completed.is_none());
        store.index.advance_event_seq(run_id, 5).unwrap();
        assert!(
            load_run(&store.index, session, run_id)
                .await
                .unwrap()
                .notice
                .contains("partial history")
        );
    }

    #[tokio::test]
    async fn event_budget_is_bounded_and_empty_or_foreign_history_is_not_complete() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::new(root.path());
        let session = SessionId::new();
        let job = JobId::new();
        let run_id = RunId::new();
        let run = store.start_run(session, job, run_id).unwrap();
        assert!(
            load_run(&store.index, session, run_id)
                .await
                .unwrap()
                .notice
                .contains("partial")
        );
        run.trace_writer
            .append(&StreamEvent::RunStarted {
                run_id,
                job_id: job,
                user_message: "start".into(),
                content_blocks: Vec::new(),
            })
            .unwrap();
        for _ in 0..MAX_HISTORY_EVENTS + 8 {
            run.trace_writer
                .append(&StreamEvent::LlmChunk { delta: "x".into() })
                .unwrap();
        }
        let saved = load_run(&store.index, session, run_id).await.unwrap();
        assert!(saved.notice.contains("partial"));
        assert!(saved.view.assistant_text.len() < MAX_HISTORY_EVENTS);
        let wrong = StreamEvent::RunStarted {
            run_id: RunId::new(),
            job_id: job,
            user_message: "foreign".into(),
            content_blocks: Vec::new(),
        };
        let corrupt_run = RunId::new();
        store.start_run(session, JobId::new(), corrupt_run).unwrap();
        store
            .index
            .append_event(
                corrupt_run,
                1,
                &wrong,
                &serde_json::to_string(&wrong).unwrap(),
            )
            .unwrap();
        assert!(load_run(&store.index, session, corrupt_run).await.is_err());
    }
}
