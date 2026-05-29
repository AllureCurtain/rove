use std::io::Write;

use futures::{Stream, StreamExt};

use crate::core::events::StreamEvent;
use crate::core::types::{TaskState, TerminationReason};
use crate::core::workspace::Workspace;
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::{RunHandle, StateStore};

pub struct CliRunRenderContext<'a> {
    pub message: String,
    pub run: RunHandle,
    pub resume_state: Option<TaskState>,
    pub state_store: &'a StateStore,
    pub workspace: &'a Workspace,
    pub model_id: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct CliRunRenderOptions {
    pub print_done_line: bool,
    pub print_trailing_newline: bool,
}

impl Default for CliRunRenderOptions {
    fn default() -> Self {
        Self {
            print_done_line: true,
            print_trailing_newline: true,
        }
    }
}

pub async fn render_run_events<S>(
    stream: S,
    context: CliRunRenderContext<'_>,
    options: CliRunRenderOptions,
) -> TerminationReason
where
    S: Stream<Item = StreamEvent>,
{
    futures::pin_mut!(stream);
    let CliRunRenderContext {
        message,
        run,
        resume_state,
        state_store,
        workspace,
        model_id,
    } = context;
    let resume_state_for_recorder = resume_state.clone();
    let RunHandle {
        session_id,
        job_id,
        run_id,
        run_dir,
        trace_writer: _,
    } = run;
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        message,
        resume_state_for_recorder.as_ref(),
    );
    let mut terminal_reason = TerminationReason::Error;

    while let Some(event) = stream.next().await {
        recorder.record_event(&event, state_store).await;
        match event {
            StreamEvent::LlmChunk { delta } => {
                print!("{}", delta);
                let _ = std::io::stdout().flush();
            }
            StreamEvent::ToolCallStarted { name, args, .. } => {
                eprintln!("\n  [tool] {}({})", name, args);
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                eprintln!("  [result] {}", truncate(&result.output, 200));
            }
            StreamEvent::ToolCallFailed { error, .. } => {
                eprintln!("  [error] {}", error);
            }
            StreamEvent::PlanCreated { plan: new_plan } => {
                eprintln!("\n  [plan] {} steps", new_plan.steps.len());
            }
            StreamEvent::PlanStepStarted { step, index } => {
                eprintln!("  [step {}] {}", index + 1, step.title);
            }
            StreamEvent::RunCompleted { reason, output } => {
                terminal_reason = reason.clone();
                if let Some(ref text) = output
                    && !matches!(reason, TerminationReason::Final)
                {
                    println!("\n{}", text);
                }
                if options.print_done_line {
                    eprintln!("\n  [done] {:?}", reason);
                }
                break;
            }
            _ => {}
        }
    }

    if options.print_trailing_newline {
        println!();
    }

    recorder
        .finalize(state_store, workspace, model_id, &run_dir)
        .await;
    terminal_reason
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use futures::stream;

    use crate::core::events::StreamEvent;
    use crate::core::types::{JobId, RunId, SessionId, TerminationReason};
    use crate::core::workspace::Workspace;
    use crate::state::store::StateStore;

    use super::{CliRunRenderContext, CliRunRenderOptions, render_run_events};

    #[tokio::test]
    async fn render_events_returns_terminal_reason() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let state_store = StateStore::new(&workspace.state_dir);
        let run = state_store
            .start_run(SessionId::new(), JobId::new(), RunId::new())
            .unwrap();
        let run_id = run.run_id;
        let job_id = run.job_id;
        let events = stream::iter(vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            },
            StreamEvent::LlmChunk {
                delta: "hi".to_string(),
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("hi".to_string()),
            },
        ]);

        let reason = render_run_events(
            events,
            CliRunRenderContext {
                message: "hello".to_string(),
                run,
                resume_state: None,
                state_store: &state_store,
                workspace: &workspace,
                model_id: "fake",
            },
            CliRunRenderOptions::default(),
        )
        .await;

        assert_eq!(reason, TerminationReason::Final);
    }
}
