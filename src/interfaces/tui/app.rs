use std::error::Error;
use std::io;
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{Event, EventStream};
use futures::{Stream, StreamExt};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::core::types::{JobId, RunId, SessionId, TaskState, TerminationReason};
use crate::interfaces::cli::runtime::CliRuntime;
use crate::interfaces::terminal::action::TerminalAction;
use crate::interfaces::terminal::view::RunViewUpdate;
use crate::interfaces::tui::action::TuiAction;
use crate::interfaces::tui::effect::TuiEffect;
use crate::interfaces::tui::keymap::map_key_event;
use crate::interfaces::tui::reducer::reduce;
use crate::interfaces::tui::render::{render, sync_viewport};
use crate::interfaces::tui::run::{TuiRunContext, drive_tui_run_events};
use crate::interfaces::tui::state::TuiState;
use crate::interfaces::tui::terminal::TerminalSession;

const RUN_UPDATE_CAPACITY: usize = 32;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Debug)]
pub struct TuiApp {
    pub state: TuiState,
    session_id: SessionId,
    active_resume_state: Option<TaskState>,
}

impl TuiApp {
    pub fn new(active_resume_state: Option<TaskState>) -> Self {
        let session_id = active_resume_state
            .as_ref()
            .map(|state| state.session_id)
            .unwrap_or_default();
        Self {
            state: TuiState::default(),
            session_id,
            active_resume_state,
        }
    }

    fn next_run_identity(&self) -> (SessionId, JobId, RunId) {
        (
            self.active_resume_state
                .as_ref()
                .map(|state| state.session_id)
                .unwrap_or(self.session_id),
            self.active_resume_state
                .as_ref()
                .map(|state| state.job_id)
                .unwrap_or_default(),
            RunId::new(),
        )
    }
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new(None)
    }
}

#[derive(Debug)]
struct ActiveRunResult {
    run_id: RunId,
    reason: TerminationReason,
    exit_requested: bool,
}

#[derive(Debug)]
struct ActiveUiResult {
    exit_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownSignal {
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

pub async fn run(
    runtime: CliRuntime,
    active_resume_state: Option<TaskState>,
) -> anyhow::Result<()> {
    let mut terminal = TerminalSession::enter().context("failed to enter TUI terminal mode")?;
    let mut events = EventStream::new();
    let (signal_task, mut shutdown) = spawn_shutdown_listener();

    let app_result = run_loop(
        &mut terminal,
        &mut events,
        &mut shutdown,
        &runtime,
        active_resume_state,
    )
    .await;
    signal_task.abort();
    let restore_result = terminal.restore();

    match (app_result, restore_result) {
        (Ok(_), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error).context("failed to restore terminal state"),
        (Err(error), Err(restore_error)) => {
            Err(error.context(format!("terminal restore also failed: {restore_error}")))
        }
    }
}

async fn run_loop<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: &mut mpsc::Receiver<ShutdownSignal>,
    runtime: &CliRuntime,
    active_resume_state: Option<TaskState>,
) -> anyhow::Result<TuiApp>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let mut app = TuiApp::new(active_resume_state);
    let mut shutdown_open = true;
    draw_app(terminal, &mut app.state)?;

    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    return Ok(app);
                };
                let effects = reduce_event(&mut app.state, event?);
                let mut prompt = None;
                for effect in effects {
                    match effect {
                        TuiEffect::Dispatch(TerminalAction::SubmitPrompt(message)) => {
                            prompt = Some(message);
                        }
                        TuiEffect::Exit => return Ok(app),
                        TuiEffect::Dispatch(_) | TuiEffect::ExitAfterRun => {}
                    }
                }

                if let Some(message) = prompt {
                    let result = run_prompt(
                        terminal,
                        events,
                        shutdown,
                        &mut shutdown_open,
                        runtime,
                        &mut app,
                        message,
                    )
                    .await?;
                    tracing::debug!(
                        run_id = %result.run_id,
                        reason = ?result.reason,
                        "TUI run finished"
                    );
                    if result.exit_requested {
                        return Ok(app);
                    }
                }
                if app.state.should_quit {
                    return Ok(app);
                }
                draw_app(terminal, &mut app.state)?;
            }
            signal = shutdown.recv(), if shutdown_open => {
                match signal {
                    Some(ShutdownSignal::Interrupt) => {
                        reduce(
                            &mut app.state,
                            TuiAction::Terminal(TerminalAction::CancelRun),
                        );
                        draw_app(terminal, &mut app.state)?;
                    }
                    #[cfg(unix)]
                    Some(ShutdownSignal::Terminate) => return Ok(app),
                    None => shutdown_open = false,
                }
            }
        }
    }
}

async fn run_prompt<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: &mut mpsc::Receiver<ShutdownSignal>,
    shutdown_open: &mut bool,
    runtime: &CliRuntime,
    app: &mut TuiApp,
    message: String,
) -> anyhow::Result<ActiveRunResult>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let (session_id, job_id, run_id) = app.next_run_identity();
    let run = runtime.state_store.start_run(session_id, job_id, run_id)?;
    let resume_state = app.active_resume_state.clone();
    let request = run.request(message.clone(), resume_state.clone());
    let trace_writer = run.trace_writer.clone();
    let cancel = CancellationToken::new();
    let stream = runtime
        .engine
        .run_with_cancel(request, Some(trace_writer), cancel.clone());
    let (updates_tx, updates_rx) = mpsc::channel(RUN_UPDATE_CAPACITY);
    let driver = drive_tui_run_events(
        stream,
        TuiRunContext {
            message,
            run,
            resume_state,
            state_store: &runtime.state_store,
            workspace: &runtime.workspace,
            model_id: runtime.engine.model_id(),
            runtime_identity: Some(runtime.engine.runtime_identity()),
        },
        move |update| {
            let updates_tx = updates_tx.clone();
            async move {
                let _ = updates_tx.send(update).await;
            }
        },
    );
    let ui = active_ui_loop(
        terminal,
        events,
        shutdown,
        shutdown_open,
        updates_rx,
        cancel,
        &mut app.state,
    );

    let (outcome, ui_result) = tokio::join!(driver, ui);
    let ui_result = ui_result?;
    if !matches!(outcome.reason, TerminationReason::Cancelled)
        && let Ok(latest) = runtime.state_store.load_task_state(run_id).await
    {
        app.session_id = latest.session_id;
        app.active_resume_state = Some(latest);
    }

    Ok(ActiveRunResult {
        run_id,
        reason: outcome.reason,
        exit_requested: ui_result.exit_requested,
    })
}

async fn active_ui_loop<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: &mut mpsc::Receiver<ShutdownSignal>,
    shutdown_open: &mut bool,
    mut updates: mpsc::Receiver<RunViewUpdate>,
    cancel: CancellationToken,
    state: &mut TuiState,
) -> anyhow::Result<ActiveUiResult>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let mut exit_requested = false;
    let mut dirty = true;
    let mut redraw = tokio::time::interval(FRAME_INTERVAL);
    redraw.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = redraw.tick(), if dirty => {
                if let Err(error) = draw_app(terminal, state) {
                    cancel.cancel();
                    return Err(error);
                }
                dirty = false;
            }
            update = updates.recv() => {
                let Some(update) = update else {
                    return Err(anyhow::anyhow!(
                        "runtime event stream ended without a completion update"
                    ));
                };
                let completed = matches!(&update, RunViewUpdate::RunCompleted { .. });
                state.apply_run_update(update);
                dirty = true;
                if completed {
                    draw_app(terminal, state)?;
                    return Ok(ActiveUiResult { exit_requested });
                }
            }
            event = events.next() => {
                let Some(event) = event else {
                    cancel.cancel();
                    return Err(anyhow::anyhow!("terminal input stream closed during an active run"));
                };
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        cancel.cancel();
                        return Err(error.into());
                    }
                };
                for effect in reduce_event(state, event) {
                    match effect {
                        TuiEffect::Dispatch(TerminalAction::CancelRun) => cancel.cancel(),
                        TuiEffect::ExitAfterRun => {
                            exit_requested = true;
                            cancel.cancel();
                        }
                        TuiEffect::Exit => exit_requested = true,
                        TuiEffect::Dispatch(_) => {}
                    }
                }
                dirty = true;
            }
            signal = shutdown.recv(), if *shutdown_open => {
                match signal {
                    Some(ShutdownSignal::Interrupt) => {
                        reduce(state, TuiAction::Terminal(TerminalAction::CancelRun));
                        cancel.cancel();
                        dirty = true;
                    }
                    #[cfg(unix)]
                    Some(ShutdownSignal::Terminate) => {
                        reduce(state, TuiAction::Terminal(TerminalAction::CancelRun));
                        exit_requested = true;
                        cancel.cancel();
                        dirty = true;
                    }
                    None => *shutdown_open = false,
                }
            }
        }
    }
}

fn reduce_event(state: &mut TuiState, event: Event) -> Vec<TuiEffect> {
    match event {
        Event::Key(key) => map_key_event(key).map_or_else(Vec::new, |action| reduce(state, action)),
        Event::Resize(width, height) => reduce(state, TuiAction::Resize { width, height }),
        Event::Paste(text) => text
            .chars()
            .flat_map(|ch| reduce(state, TuiAction::InsertChar(ch)))
            .collect(),
        Event::FocusGained | Event::FocusLost | Event::Mouse(_) => Vec::new(),
    }
}

fn draw_app<B>(terminal: &mut Terminal<B>, state: &mut TuiState) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
{
    let size = terminal.backend().size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    sync_viewport(state, area);
    terminal.draw(|frame| render(frame, state))?;
    Ok(())
}

fn spawn_shutdown_listener() -> (tokio::task::JoinHandle<()>, mpsc::Receiver<ShutdownSignal>) {
    let (sender, receiver) = mpsc::channel(1);
    let task = tokio::spawn(listen_for_shutdown_signals(sender));
    (task, receiver)
}

#[cfg(unix)]
async fn listen_for_shutdown_signals(sender: mpsc::Sender<ShutdownSignal>) {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(terminate) => terminate,
        Err(error) => {
            tracing::warn!("failed to install SIGTERM handler: {error}");
            listen_for_interrupts(sender).await;
            return;
        }
    };

    loop {
        let shutdown = tokio::select! {
            interrupt = tokio::signal::ctrl_c() => {
                match interrupt {
                    Ok(()) => ShutdownSignal::Interrupt,
                    Err(error) => {
                        tracing::warn!("failed to listen for Ctrl+C: {error}");
                        return;
                    }
                }
            }
            terminated = terminate.recv() => {
                if terminated.is_none() {
                    tracing::warn!("SIGTERM listener closed unexpectedly");
                    return;
                }
                ShutdownSignal::Terminate
            }
        };

        if sender.send(shutdown).await.is_err() {
            return;
        }
    }
}

#[cfg(not(unix))]
async fn listen_for_shutdown_signals(sender: mpsc::Sender<ShutdownSignal>) {
    listen_for_interrupts(sender).await;
}

async fn listen_for_interrupts(sender: mpsc::Sender<ShutdownSignal>) {
    loop {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!("failed to listen for Ctrl+C: {error}");
            return;
        }
        if sender.send(ShutdownSignal::Interrupt).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use futures::{StreamExt, stream};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_util::sync::CancellationToken;

    use crate::core::types::{JobId, RunId, TerminationReason};
    use crate::interfaces::cli::args::CliApprovalPolicy;
    use crate::interfaces::cli::runtime::{
        CliRuntimeInteraction, CliRuntimeOptions, build_cli_runtime,
    };
    use crate::interfaces::terminal::view::RunViewUpdate;
    use crate::interfaces::tui::state::{RunLifecycle, TuiState};

    use super::{ShutdownSignal, TuiApp, active_ui_loop, run_loop, run_prompt};

    async fn send_and_wait_for_receive<T>(sender: &mpsc::Sender<T>, value: T) {
        sender.send(value).await.unwrap();
        let permit = sender.reserve().await.unwrap();
        drop(permit);
    }

    #[tokio::test]
    async fn fake_prompt_reaches_shared_engine_and_finalizes_artifacts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: None,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("TUI_FAKE_RESPONSE".to_string()),
            interaction: CliRuntimeInteraction::Providers {
                input_provider: None,
                approval_provider: None,
            },
        })
        .await
        .unwrap();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut events = stream::pending::<io::Result<Event>>();
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let mut app = TuiApp::default();
        app.state.run_lifecycle = RunLifecycle::Running;

        let result = run_prompt(
            &mut terminal,
            &mut events,
            &mut shutdown,
            &mut shutdown_open,
            &runtime,
            &mut app,
            "hello from tui".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(result.reason, TerminationReason::Final);
        assert_eq!(app.state.run_lifecycle, RunLifecycle::Completed);
        assert!(app.state.run.assistant_text.contains("TUI_FAKE_RESPONSE"));
        let run_dir = runtime.state_store.run_store.run_dir(&result.run_id);
        assert!(run_dir.join("trace.jsonl").exists());
        assert!(run_dir.join("task_state.json").exists());
        assert!(run_dir.join("report.json").exists());
        assert!(
            runtime
                .state_store
                .load_task_state(result.run_id)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn ctrl_c_cancels_an_active_run_and_waits_for_canonical_completion() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut events = stream::iter(vec![Ok(Event::Key(key))]).chain(stream::pending());
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let cancel_for_producer = cancel.clone();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let run_id = RunId::new();
        let producer = async move {
            updates_tx
                .send(RunViewUpdate::RunStarted {
                    run_id,
                    job_id: JobId::new(),
                    user_message: "cancel me".to_string(),
                })
                .await
                .unwrap();
            cancel_for_producer.cancelled().await;
            updates_tx
                .send(RunViewUpdate::RunCompleted {
                    reason: TerminationReason::Cancelled,
                    output: None,
                })
                .await
                .unwrap();
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            &mut shutdown,
            &mut shutdown_open,
            updates_rx,
            cancel.clone(),
            &mut state,
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            let ((), result) = tokio::join!(producer, ui);
            result.unwrap();
        })
        .await
        .unwrap();

        assert!(cancel.is_cancelled());
        assert_eq!(state.run_lifecycle, RunLifecycle::Completed);
        assert!(matches!(
            state.run.completed.as_ref().map(|view| &view.reason),
            Some(TerminationReason::Cancelled)
        ));
    }

    #[tokio::test]
    async fn idle_loop_keeps_receiving_interrupts_after_the_first_one() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: None,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("unused".to_string()),
            interaction: CliRuntimeInteraction::Providers {
                input_provider: None,
                approval_provider: None,
            },
        })
        .await
        .unwrap();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        let mut events = ReceiverStream::new(event_rx);
        let (shutdown_tx, mut shutdown) = mpsc::channel(1);

        let producer = async move {
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('x'),
                    KeyModifiers::NONE,
                ))),
            )
            .await;
            send_and_wait_for_receive(&shutdown_tx, ShutdownSignal::Interrupt).await;
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                ))),
            )
            .await;
            send_and_wait_for_receive(&shutdown_tx, ShutdownSignal::Interrupt).await;
            event_tx
                .send(Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::CONTROL,
                ))))
                .await
                .unwrap();
        };
        let app = run_loop(&mut terminal, &mut events, &mut shutdown, &runtime, None);

        let ((), result) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(producer, app)
        })
        .await
        .expect("the second interrupt must still be consumed");

        assert!(result.unwrap().state.composer.is_empty());
    }

    #[tokio::test]
    async fn closing_terminal_input_drops_a_full_update_receiver_without_deadlock() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut events = stream::empty::<io::Result<Event>>();
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        updates_tx
            .send(RunViewUpdate::RunStarted {
                run_id: RunId::new(),
                job_id: JobId::new(),
                user_message: "fill the queue".to_string(),
            })
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let producer = async move {
            loop {
                if updates_tx
                    .send(RunViewUpdate::AssistantDelta {
                        delta: "chunk".to_string(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            &mut shutdown,
            &mut shutdown_open,
            updates_rx,
            cancel.clone(),
            &mut state,
        );

        let ((), ui_result) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(producer, ui) })
                .await
                .expect("closed UI receiver must unblock the bounded producer");

        assert!(ui_result.is_err());
        assert!(cancel.is_cancelled());
    }
}
