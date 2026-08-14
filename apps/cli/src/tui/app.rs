use std::collections::HashSet;
use std::collections::VecDeque;
use std::error::Error;
use std::io;
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::{FutureExt, Stream, StreamExt};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::cli::runtime::CliRuntime;
use crate::terminal::action::TerminalAction;
use crate::terminal::view::RunViewUpdate;
use crate::tui::action::TuiAction;
use crate::tui::effect::TuiEffect;
use crate::tui::keymap::map_key_event_with_overlay_mode;
use crate::tui::providers::{TuiInteractionKind, TuiInteractionReceiver, TuiInteractionRequest};
use crate::tui::reducer::reduce;
use crate::tui::render::{render, sync_viewport};
use crate::tui::run::{TuiRunContext, drive_tui_run_events};
use crate::tui::state::{
    InteractionKeyMode, InteractionModalKind, InteractionModalView, MAX_VISIBLE_MESSAGES,
    ResumeCandidate, RunLifecycle, SessionPickerError, SessionPickerState, TuiOverlay, TuiState,
};
use crate::tui::terminal::TerminalSession;
use rove_runtime::conversation::{
    MessageDomainService, MessagePageQuery, SendMessageCommand, SessionDeliveryState,
};
use rove_runtime::engine::RunControlHandle;
use rove_runtime::events::StreamEvent;
use rove_runtime::state::index::ResumeJobClaim;
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::types::{
    ApprovalDecision, CallId, JobId, RunId, SessionId, TaskState, TerminationReason,
};

const RUN_UPDATE_CAPACITY: usize = 32;
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const MAX_ARMING_DRAIN_EVENTS: usize = 1024;

#[derive(Debug)]
pub struct TuiApp {
    pub state: TuiState,
    session_id: SessionId,
    active_resume_state: Option<TaskState>,
    resume_claim: Option<ResumeJobClaim>,
    pressed_keys: PressedKeys,
    pending_run_id: Option<RunId>,
    pending_startup_events: Vec<StreamEvent>,
}

impl TuiApp {
    pub fn new(active_resume_state: Option<TaskState>) -> Self {
        let session_id = active_resume_state
            .as_ref()
            .map(|state| state.session_id)
            .unwrap_or_default();
        let state = TuiState {
            active_resume: active_resume_state
                .as_ref()
                .map(ResumeCandidate::from_task_state),
            ..TuiState::default()
        };
        Self {
            state,
            session_id,
            active_resume_state,
            resume_claim: None,
            pressed_keys: PressedKeys::default(),
            pending_run_id: None,
            pending_startup_events: Vec::new(),
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

    fn set_active_resume_state(&mut self, active_resume_state: TaskState) {
        self.session_id = active_resume_state.session_id;
        self.state.active_resume = Some(ResumeCandidate::from_task_state(&active_resume_state));
        self.active_resume_state = Some(active_resume_state);
    }

    fn clear_active_resume_state(&mut self) {
        self.state.active_resume = None;
        self.active_resume_state = None;
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

#[derive(Default)]
struct InteractionController {
    waiting_request: Option<TuiInteractionRequest>,
    waiting_view: Option<InteractionModalView>,
    active_request: Option<TuiInteractionRequest>,
    active_armed: bool,
    modal_drawn: bool,
}

impl InteractionController {
    fn offer_request(&mut self, request: TuiInteractionRequest, state: &mut TuiState) {
        if request.responder_is_closed()
            || state.run_lifecycle != RunLifecycle::Running
            || self.active_request.is_some()
            || self.waiting_request.is_some()
        {
            return;
        }

        if let Some(view) = self.waiting_view.take() {
            if request_matches_view(&request, &view) {
                self.activate(request, view, state);
            } else {
                self.waiting_view = Some(view);
            }
        } else {
            self.waiting_request = Some(request);
        }
    }

    fn offer_view(&mut self, view: InteractionModalView, state: &mut TuiState) {
        if state.run_lifecycle != RunLifecycle::Running || self.active_request.is_some() {
            return;
        }

        if let Some(request) = self.waiting_request.take() {
            if request_matches_view(&request, &view) {
                self.activate(request, view, state);
            } else {
                self.waiting_request = Some(request);
            }
        } else if self.waiting_view.is_none() {
            self.waiting_view = Some(view);
        }
    }

    fn activate(
        &mut self,
        request: TuiInteractionRequest,
        view: InteractionModalView,
        state: &mut TuiState,
    ) {
        if state.modal.is_some() || request.responder_is_closed() {
            return;
        }
        self.active_request = Some(request);
        self.active_armed = false;
        self.modal_drawn = false;
        reduce(state, TuiAction::OpenInteraction(view));
    }

    fn observe_update(&mut self, update: &RunViewUpdate, state: &mut TuiState) {
        match update {
            RunViewUpdate::ToolCallApprovalNeeded {
                call_id,
                name,
                args,
                reason,
            } => self.offer_view(
                InteractionModalView::Approval {
                    call_id: *call_id,
                    name: name.clone(),
                    args: args.clone(),
                    reason: reason.clone(),
                },
                state,
            ),
            RunViewUpdate::InputNeeded { input_id, prompt } => self.offer_view(
                InteractionModalView::Input {
                    input_id: *input_id,
                    prompt: prompt.clone(),
                    draft: String::new(),
                },
                state,
            ),
            RunViewUpdate::ToolCallCompleted { call_id, .. }
            | RunViewUpdate::ToolCallFailed { call_id, .. } => {
                self.drop_request(*call_id, state);
            }
            RunViewUpdate::RunCompleted { .. } => self.clear(state),
            _ => {}
        }
    }

    fn resolve(&mut self, action: &TerminalAction) {
        let matches = self.active_armed
            && self
                .active_request
                .as_ref()
                .is_some_and(|request| action_matches_request(action, request));
        if !matches {
            return;
        }

        let request = self
            .active_request
            .take()
            .expect("matching request was checked above");
        self.active_armed = false;
        self.modal_drawn = false;
        match (request, action) {
            (
                TuiInteractionRequest::Approval { respond_to, .. },
                TerminalAction::ApproveTool { .. },
            ) => {
                let _ = respond_to.send(ApprovalDecision::Approve);
            }
            (
                TuiInteractionRequest::Approval { respond_to, .. },
                TerminalAction::RejectTool { .. },
            ) => {
                let _ = respond_to.send(ApprovalDecision::Reject);
            }
            (
                TuiInteractionRequest::Input { respond_to, .. },
                TerminalAction::SubmitInput { answer, .. },
            ) => {
                let _ = respond_to.send(answer.clone());
            }
            _ => unreachable!("request/action type was checked above"),
        }
    }

    fn drop_request(&mut self, request_id: CallId, state: &mut TuiState) {
        if self
            .active_request
            .as_ref()
            .is_some_and(|request| request.request_id() == request_id)
        {
            self.active_request = None;
        }
        if self
            .waiting_request
            .as_ref()
            .is_some_and(|request| request.request_id() == request_id)
        {
            self.waiting_request = None;
        }
        if self
            .waiting_view
            .as_ref()
            .is_some_and(|view| view.request_id() == request_id)
        {
            self.waiting_view = None;
        }
        close_modal(state, request_id);
        if self.active_request.is_none() {
            self.active_armed = false;
            self.modal_drawn = false;
        }
    }

    fn clear(&mut self, state: &mut TuiState) {
        self.active_request = None;
        self.waiting_request = None;
        self.waiting_view = None;
        self.active_armed = false;
        self.modal_drawn = false;
        if let Some(modal) = state.modal.as_ref() {
            let kind = modal.kind();
            let request_id = modal.request_id();
            reduce(state, TuiAction::CloseInteraction { kind, request_id });
        }
    }

    fn needs_arming(&self) -> bool {
        self.active_request.is_some() && !self.active_armed
    }

    fn is_armed(&self) -> bool {
        self.active_armed
    }

    fn after_modal_draw(&mut self, no_keys_down: bool) {
        if !self.needs_arming() {
            return;
        }
        if self.modal_drawn && no_keys_down {
            self.active_armed = true;
        } else {
            self.modal_drawn = true;
        }
    }

    fn discard_closed_active(&mut self, state: &mut TuiState) -> bool {
        let Some(request_id) = self
            .active_request
            .as_ref()
            .filter(|request| request.responder_is_closed())
            .map(TuiInteractionRequest::request_id)
        else {
            return false;
        };
        self.drop_request(request_id, state);
        true
    }
}

#[derive(Debug, Default)]
struct PressedKeys {
    keys: HashSet<KeyCode>,
}

impl PressedKeys {
    fn observe(&mut self, event: &Event) -> bool {
        let Event::Key(key) = event else {
            return false;
        };
        match key.kind {
            KeyEventKind::Press => self.keys.insert(key.code),
            KeyEventKind::Repeat => {
                self.keys.insert(key.code);
                false
            }
            KeyEventKind::Release => {
                self.keys.remove(&key.code);
                false
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

fn request_matches_view(request: &TuiInteractionRequest, view: &InteractionModalView) -> bool {
    interaction_kind(request) == view.kind() && request.request_id() == view.request_id()
}

fn interaction_kind(request: &TuiInteractionRequest) -> InteractionModalKind {
    match request.kind() {
        TuiInteractionKind::Approval => InteractionModalKind::Approval,
        TuiInteractionKind::Input => InteractionModalKind::Input,
    }
}

fn action_matches_request(action: &TerminalAction, request: &TuiInteractionRequest) -> bool {
    match (action, request) {
        (
            TerminalAction::ApproveTool { call_id } | TerminalAction::RejectTool { call_id },
            TuiInteractionRequest::Approval { request, .. },
        ) => *call_id == request.call_id,
        (
            TerminalAction::SubmitInput { input_id, .. },
            TuiInteractionRequest::Input {
                input_id: request_id,
                ..
            },
        ) => input_id == request_id,
        _ => false,
    }
}

fn close_modal(state: &mut TuiState, request_id: CallId) {
    if let Some(modal) = state.modal.as_ref()
        && modal.request_id() == request_id
    {
        let kind = modal.kind();
        reduce(state, TuiAction::CloseInteraction { kind, request_id });
    }
}

fn discard_queued_interactions(interactions: &mut TuiInteractionReceiver) -> usize {
    let mut discarded = 0;
    while let Ok(request) = interactions.try_recv() {
        drop(request);
        discarded += 1;
    }
    discarded
}

fn clear_interactions(
    controller: &mut InteractionController,
    interactions: &mut TuiInteractionReceiver,
    state: &mut TuiState,
) {
    controller.clear(state);
    discard_queued_interactions(interactions);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownSignal {
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

struct ShutdownInput<'a> {
    receiver: &'a mut mpsc::Receiver<ShutdownSignal>,
    open: &'a mut bool,
}

struct ActiveUiControl<'a> {
    cancel: CancellationToken,
    pressed_keys: &'a mut PressedKeys,
    message_service: &'a MessageDomainService,
    control: &'a RunControlHandle,
    session_id: SessionId,
    run_id: RunId,
}

#[cfg(test)]
fn test_message_service() -> &'static MessageDomainService {
    static SERVICE: std::sync::OnceLock<MessageDomainService> = std::sync::OnceLock::new();
    SERVICE.get_or_init(|| {
        MessageDomainService::new(Arc::new(
            rove_runtime::conversation::SqliteMessageRepository::new(
                std::env::temp_dir().join("rove-tui-test-messages.sqlite"),
                100,
            ),
        ))
    })
}

#[cfg(test)]
fn test_control() -> &'static RunControlHandle {
    static CONTROL: std::sync::OnceLock<RunControlHandle> = std::sync::OnceLock::new();
    CONTROL.get_or_init(RunControlHandle::disconnected)
}

pub async fn run(
    runtime: CliRuntime,
    active_resume_state: Option<TaskState>,
    mut interactions: TuiInteractionReceiver,
) -> anyhow::Result<()> {
    if let Some(resume_state) = active_resume_state.as_ref() {
        validate_tui_resume_state(&runtime, resume_state).await?;
    }
    let mut terminal = TerminalSession::enter().context("failed to enter TUI terminal mode")?;
    let interaction_key_mode = terminal.interaction_key_mode();
    let mut events = EventStream::new();
    let (signal_task, mut shutdown) = spawn_shutdown_listener();

    let app_result = run_loop(
        &mut terminal,
        &mut events,
        &mut shutdown,
        &runtime,
        active_resume_state,
        &mut interactions,
        interaction_key_mode,
    )
    .await;
    signal_task.abort();
    discard_queued_interactions(&mut interactions);
    interactions.close();
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

async fn validate_tui_resume_state(
    runtime: &CliRuntime,
    resume_state: &TaskState,
) -> anyhow::Result<()> {
    let Some(job) = runtime
        .state_store
        .index
        .job_record_async(resume_state.job_id)
        .await?
    else {
        anyhow::bail!("cannot resume: indexed job is missing");
    };
    let Some(run) = runtime.state_store.index.run_record(resume_state.run_id)? else {
        anyhow::bail!("cannot resume: indexed run is missing");
    };
    if job.session_id != resume_state.session_id
        || job.run_id != Some(resume_state.run_id)
        || run.session_id != resume_state.session_id
        || run.job_id != resume_state.job_id
        || run.status != job.status
        || !is_terminal_index_status(&job.status)
    {
        anyhow::bail!("cannot resume: selected task state is stale or still active");
    }
    Ok(())
}

fn is_terminal_index_status(status: &str) -> bool {
    matches!(status, "done" | "error" | "cancelled" | "interrupted")
}

async fn run_loop<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: &mut mpsc::Receiver<ShutdownSignal>,
    runtime: &CliRuntime,
    active_resume_state: Option<TaskState>,
    interactions: &mut TuiInteractionReceiver,
    interaction_key_mode: InteractionKeyMode,
) -> anyhow::Result<TuiApp>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let mut app = TuiApp::new(active_resume_state);
    app.state.interaction_key_mode = interaction_key_mode;
    match runtime.selection_for_session(app.session_id) {
        Ok((selection, revision)) => {
            app.state.model_selection = Some(selection);
            app.state.model_selection_revision = revision;
        }
        Err(error) => app.state.model_notice = Some(error.to_string()),
    }
    let mut shutdown_open = true;
    draw_app(terminal, &mut app.state)?;

    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    return Ok(app);
                };
                let event = event?;
                let fresh_press = app.pressed_keys.observe(&event);
                let effects = reduce_event(&mut app.state, event, true, fresh_press);
                let prompt = apply_idle_effects(effects, runtime, &mut app).await?;

                if let Some(message) = prompt {
                    let mut next = Some((
                        message,
                        app.pending_startup_events.drain(..).collect(),
                        app.pending_run_id.take(),
                    ));
                    while let Some((message, startup_events, requested_run_id)) = next.take() {
                        let result = run_prompt(
                            terminal,
                            events,
                            ShutdownInput {
                                receiver: shutdown,
                                open: &mut shutdown_open,
                            },
                            runtime,
                            &mut app,
                            message,
                            startup_events,
                            requested_run_id,
                            interactions,
                        )
                        .await?;
                        tracing::debug!(
                            run_id = %result.run_id,
                            reason = ?result.reason,
                            "TUI run finished"
                        );
                        refresh_tui_messages(runtime, &mut app).await;
                        if result.exit_requested {
                            return Ok(app);
                        }
                        if result.reason == TerminationReason::Final {
                            next = claim_next_tui_successor(runtime, &mut app).await;
                        } else {
                            require_tui_message_attention(
                                runtime,
                                &mut app,
                                &format!("run ended with {}", termination_reason_label(&result.reason)),
                            )
                            .await;
                        }
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

fn current_tui_session_id(app: &TuiApp) -> SessionId {
    app.active_resume_state
        .as_ref()
        .map(|state| state.session_id)
        .unwrap_or(app.session_id)
}

async fn refresh_tui_messages(runtime: &CliRuntime, app: &mut TuiApp) {
    let session_id = current_tui_session_id(app).to_string();
    match runtime
        .message_service
        .list(&session_id, MessagePageQuery::latest(MAX_VISIBLE_MESSAGES))
        .await
    {
        Ok(page) => app.state.replace_messages(page.messages),
        Err(error) => app.state.message_error = Some(error.to_string()),
    }
}

async fn claim_next_tui_successor(
    runtime: &CliRuntime,
    app: &mut TuiApp,
) -> Option<(String, Vec<StreamEvent>, Option<RunId>)> {
    let session_id = current_tui_session_id(app);
    let run_id = RunId::new();
    match runtime
        .message_service
        .claim_successor(&session_id.to_string(), run_id)
        .await
    {
        Ok(Some(message)) => {
            let content = message.content.clone();
            let id = message.id.clone();
            app.state.upsert_message(message);
            Some((
                content,
                vec![StreamEvent::MessageClaimedSuccessor { id }],
                Some(run_id),
            ))
        }
        Ok(None) => None,
        Err(error) => {
            app.state.message_error = Some(error.to_string());
            None
        }
    }
}

async fn require_tui_message_attention(runtime: &CliRuntime, app: &mut TuiApp, reason: &str) {
    let session_id = current_tui_session_id(app).to_string();
    match runtime
        .message_service
        .require_attention(&session_id, reason)
        .await
    {
        Ok(messages) => {
            for message in messages {
                app.state.upsert_message(message);
            }
        }
        Err(error) => app.state.message_error = Some(error.to_string()),
    }
    refresh_tui_messages(runtime, app).await;
}

async fn mark_claimed_successor_start_failed(
    runtime: &CliRuntime,
    app: &mut TuiApp,
    run_id: RunId,
    startup_events: &[StreamEvent],
    reason: &str,
) {
    let session_id = current_tui_session_id(app).to_string();
    if let Some(message_id) = startup_events.iter().find_map(|event| match event {
        StreamEvent::MessageClaimedSuccessor { id } => Some(id.clone()),
        _ => None,
    }) {
        let event = StreamEvent::MessageNeedsAttention {
            id: message_id.clone(),
            reason: reason.to_string(),
        };
        if let Err(error) = runtime
            .message_service
            .observe_event(&session_id, run_id, &event)
            .await
        {
            app.state.message_error = Some(error.to_string());
        } else {
            app.state.message_error = Some(format!("message {message_id} needs attention"));
        }
    }
    require_tui_message_attention(runtime, app, reason).await;
}

fn termination_reason_label(reason: &TerminationReason) -> &'static str {
    match reason {
        TerminationReason::Final => "final",
        TerminationReason::StepLimit => "step limit",
        TerminationReason::TokenLimit => "token limit",
        TerminationReason::TimeLimit => "time limit",
        TerminationReason::Error => "error",
        TerminationReason::Cancelled => "cancelled",
    }
}

async fn apply_idle_effects(
    effects: Vec<TuiEffect>,
    runtime: &CliRuntime,
    app: &mut TuiApp,
) -> anyhow::Result<Option<String>> {
    let mut queue = VecDeque::from(effects);
    let mut prompt = None;
    while let Some(effect) = queue.pop_front() {
        match effect {
            TuiEffect::SendMessage {
                content,
                session_state,
                target_run_id,
            } => {
                let session_id = app
                    .active_resume_state
                    .as_ref()
                    .map(|state| state.session_id)
                    .unwrap_or_else(|| app.session_id);
                let run_id = target_run_id
                    .or_else(|| (session_state == SessionDeliveryState::Idle).then(RunId::new));
                match runtime
                    .message_service
                    .send(
                        &session_id.to_string(),
                        SendMessageCommand {
                            content: content.clone(),
                            idempotency_key: None,
                            session_state,
                            target_run_id: run_id,
                        },
                    )
                    .await
                {
                    Ok(mutation) => {
                        app.state.upsert_message(mutation.message.clone());
                        if session_state == SessionDeliveryState::Idle {
                            if !mutation.replayed {
                                app.pending_startup_events.push(StreamEvent::MessageQueued {
                                    id: mutation.message.id.clone(),
                                    content: mutation.message.content.clone(),
                                });
                            }
                            if let Some(claimed) = mutation.claimed_successor {
                                app.pending_run_id = claimed.target_run_id.or(run_id);
                                app.state.upsert_message(claimed.clone());
                                app.pending_startup_events
                                    .push(StreamEvent::MessageClaimedSuccessor { id: claimed.id });
                                prompt = Some(claimed.content);
                            }
                        } else if state_is_active(app.state.run_lifecycle) {
                            // Active delivery is published by the run's
                            // canonical control ingress below.
                            app.state.upsert_message(mutation.message);
                        }
                    }
                    Err(error) => app.state.message_error = Some(error.to_string()),
                }
            }
            TuiEffect::Dispatch(TerminalAction::SubmitPrompt(message)) => {
                if let Some(resume_state) = app.active_resume_state.as_ref() {
                    match runtime
                        .state_store
                        .index
                        .claim_job_for_resume_async(resume_state.job_id, resume_state.run_id)
                        .await
                    {
                        Ok(Some(claim)) => {
                            app.resume_claim = Some(claim);
                            prompt = Some(message);
                        }
                        Ok(None) => {
                            reject_busy_resume_submission(app, message);
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "Failed to claim resume job for TUI");
                            reject_resume_submission(app, message, SessionPickerError::LoadFailed);
                        }
                    }
                } else {
                    prompt = Some(message);
                }
            }
            TuiEffect::Dispatch(_) | TuiEffect::ExitAfterRun => {}
            TuiEffect::Exit => {
                app.state.should_quit = true;
            }
            TuiEffect::LoadSessions => {
                let action = match runtime
                    .state_store
                    .list_resumable_task_states_limited(crate::tui::state::MAX_SESSION_CANDIDATES)
                    .await
                {
                    Ok(states) => TuiAction::SessionsLoaded {
                        candidates: states
                            .iter()
                            .map(ResumeCandidate::from_task_state)
                            .collect(),
                    },
                    Err(error) => TuiAction::SessionsLoadFailed {
                        error: classify_io_error(&error),
                    },
                };
                queue.extend(reduce(&mut app.state, action));
            }
            TuiEffect::ResolveResume { run_id } => {
                let value = run_id.to_string();
                let candidate = resolving_resume_candidate(&app.state, run_id);
                match resolve_resume_state(&runtime.state_store, Some(&value)).await {
                    Ok(Some(resume_state)) if app.state.can_accept_resume(run_id) => {
                        let identity_matches = candidate.as_ref().is_some_and(|candidate| {
                            candidate.run_id == resume_state.run_id
                                && candidate.job_id == resume_state.job_id
                                && candidate.session_id == resume_state.session_id
                                && resume_state.run_id == run_id
                        });
                        let job = runtime
                            .state_store
                            .index
                            .job_record_async(resume_state.job_id)
                            .await;
                        match job {
                            Ok(Some(job))
                                if identity_matches
                                    && job.run_id == Some(run_id)
                                    && job.session_id == resume_state.session_id
                                    && is_terminal_index_status(&job.status) =>
                            {
                                app.set_active_resume_state(resume_state);
                                queue.extend(reduce(
                                    &mut app.state,
                                    TuiAction::ResumeSelectionSucceeded { run_id },
                                ));
                            }
                            Ok(Some(job)) if !is_terminal_index_status(&job.status) => {
                                queue.extend(reduce(
                                    &mut app.state,
                                    TuiAction::ResumeSelectionFailed {
                                        run_id,
                                        error: SessionPickerError::Busy,
                                    },
                                ));
                            }
                            Ok(_) => queue.extend(reduce(
                                &mut app.state,
                                TuiAction::ResumeSelectionFailed {
                                    run_id,
                                    error: if identity_matches {
                                        SessionPickerError::Stale
                                    } else {
                                        SessionPickerError::Malformed
                                    },
                                },
                            )),
                            Err(error) => queue.extend(reduce(
                                &mut app.state,
                                TuiAction::ResumeSelectionFailed {
                                    run_id,
                                    error: classify_io_error(&error),
                                },
                            )),
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => queue.extend(reduce(
                        &mut app.state,
                        TuiAction::ResumeSelectionFailed {
                            run_id,
                            error: SessionPickerError::Stale,
                        },
                    )),
                    Err(error) => queue.extend(reduce(
                        &mut app.state,
                        TuiAction::ResumeSelectionFailed {
                            run_id,
                            error: classify_anyhow_error(&error),
                        },
                    )),
                }
            }
            TuiEffect::LoadModels { query, auto_select } => {
                let action = load_model_candidates(runtime, &app.state, query, auto_select);
                queue.extend(reduce(&mut app.state, action));
            }
            TuiEffect::PersistModel {
                selection,
                expected_revision,
            } => {
                let action = match runtime.persist_session_selection(
                    app.session_id,
                    expected_revision,
                    selection,
                ) {
                    Ok(saved) => TuiAction::ModelSelectionPersisted {
                        selection: saved.selection,
                        revision: saved.revision,
                    },
                    Err(error) => TuiAction::ModelSelectionFailed {
                        error: classify_model_error(&error),
                    },
                };
                queue.extend(reduce(&mut app.state, action));
            }
            TuiEffect::ResetModel { expected_revision } => {
                let action = match runtime.default_selection().and_then(|selection| {
                    runtime.persist_session_selection(app.session_id, expected_revision, selection)
                }) {
                    Ok(saved) => TuiAction::ModelSelectionPersisted {
                        selection: saved.selection,
                        revision: saved.revision,
                    },
                    Err(error) => TuiAction::ModelSelectionFailed {
                        error: classify_model_error(&error),
                    },
                };
                queue.extend(reduce(&mut app.state, action));
            }
            TuiEffect::LoadMessages => {
                let session_id = app
                    .active_resume_state
                    .as_ref()
                    .map(|state| state.session_id)
                    .unwrap_or(app.session_id);
                match runtime
                    .message_service
                    .list(
                        &session_id.to_string(),
                        MessagePageQuery::latest(MAX_VISIBLE_MESSAGES),
                    )
                    .await
                {
                    Ok(page) => queue.extend(reduce(
                        &mut app.state,
                        TuiAction::MessagesLoaded {
                            messages: page.messages,
                        },
                    )),
                    Err(error) => queue.extend(reduce(
                        &mut app.state,
                        TuiAction::MessageOperationFailed { error },
                    )),
                }
            }
            TuiEffect::PromoteMessage { .. } | TuiEffect::RevokeMessage { .. } => {}
        }
    }
    Ok(prompt)
}

fn load_model_candidates(
    runtime: &CliRuntime,
    state: &TuiState,
    query: String,
    auto_select: bool,
) -> TuiAction {
    let catalog = match runtime.catalog() {
        Ok(catalog) => catalog,
        Err(_) => {
            return TuiAction::ModelsLoadFailed {
                error: crate::tui::state::ModelPickerError::LoadFailed,
            };
        }
    };
    let current = state.model_selection.as_ref();
    let candidates = catalog
        .profiles()
        .into_iter()
        .map(|profile| {
            let credential_ready = profile
                .auth_source
                .ready(&runtime.provider_catalog.paths().root)
                .unwrap_or(false);
            crate::tui::state::ModelCandidate {
                selection: rove_app_bootstrap::ModelSelection {
                    profile_id: profile.id.clone(),
                    model: profile.model.clone(),
                    reasoning: current
                        .map(|selection| selection.reasoning.clone())
                        .unwrap_or_else(|| "default".to_string()),
                    revision: catalog.revision().to_string(),
                },
                label: profile.label,
                provider_type: profile.provider_type,
                credential_ready,
                inventory_fresh: false,
                current: current.is_some_and(|selection| {
                    selection.profile_id == profile.id && selection.model == profile.model
                }),
            }
        })
        .collect();
    TuiAction::ModelsLoaded {
        candidates,
        query,
        auto_select,
    }
}

fn classify_model_error(error: &anyhow::Error) -> crate::tui::state::ModelPickerError {
    let text = error.to_string();
    if text.contains("revision conflict") {
        crate::tui::state::ModelPickerError::CatalogChanged
    } else if text.contains("credential") || text.contains("provider_unavailable") {
        crate::tui::state::ModelPickerError::CredentialUnavailable
    } else if text.contains("busy") {
        crate::tui::state::ModelPickerError::Busy
    } else {
        crate::tui::state::ModelPickerError::LoadFailed
    }
}

fn classify_io_error(error: &io::Error) -> SessionPickerError {
    match error.kind() {
        io::ErrorKind::InvalidData => SessionPickerError::Malformed,
        io::ErrorKind::NotFound => SessionPickerError::Stale,
        _ => SessionPickerError::LoadFailed,
    }
}

fn classify_anyhow_error(error: &anyhow::Error) -> SessionPickerError {
    for cause in error.chain() {
        if let Some(io_error) = cause.downcast_ref::<io::Error>() {
            return classify_io_error(io_error);
        }
    }
    SessionPickerError::LoadFailed
}

fn resolving_resume_candidate(state: &TuiState, run_id: RunId) -> Option<ResumeCandidate> {
    let Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
        candidates,
        resolving: Some(resolving),
        ..
    })) = state.overlay.as_ref()
    else {
        return None;
    };
    (*resolving == run_id)
        .then(|| {
            candidates
                .iter()
                .find(|candidate| candidate.run_id == run_id)
        })
        .flatten()
        .cloned()
}

fn reject_busy_resume_submission(app: &mut TuiApp, message: String) {
    reject_resume_submission(app, message, SessionPickerError::Busy);
}

fn reject_resume_submission(app: &mut TuiApp, message: String, error: SessionPickerError) {
    app.clear_active_resume_state();
    app.state.run_lifecycle = RunLifecycle::Idle;
    app.state.composer = message;
    app.state.overlay = Some(TuiOverlay::SessionPicker(SessionPickerState::Ready {
        candidates: Vec::new(),
        selected: 0,
        error: Some(error),
        resolving: None,
    }));
}

fn state_is_active(lifecycle: RunLifecycle) -> bool {
    matches!(lifecycle, RunLifecycle::Running | RunLifecycle::Cancelling)
}

#[allow(clippy::too_many_arguments)]
async fn run_prompt<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: ShutdownInput<'_>,
    runtime: &CliRuntime,
    app: &mut TuiApp,
    message: String,
    startup_events: Vec<StreamEvent>,
    requested_run_id: Option<RunId>,
    interactions: &mut TuiInteractionReceiver,
) -> anyhow::Result<ActiveRunResult>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    discard_queued_interactions(interactions);
    let assembly = match runtime
        .assemble_run(
            &message,
            app.state.model_selection.as_ref(),
            app.active_resume_state.as_ref(),
            app.state.model_selection_changed,
        )
        .await
    {
        Ok(assembly) => assembly,
        Err(error) => {
            if let Some(claim) = app.resume_claim.take() {
                let _ = runtime
                    .state_store
                    .index
                    .release_job_resume_claim_async(claim)
                    .await;
            }
            app.state.run_lifecycle = RunLifecycle::Idle;
            app.state.composer = message;
            if let Some(run_id) = requested_run_id {
                mark_claimed_successor_start_failed(
                    runtime,
                    app,
                    run_id,
                    &startup_events,
                    "successor run could not be assembled",
                )
                .await;
            }
            return Err(error);
        }
    };
    app.state.model_selection = Some(assembly.selection.clone());
    app.state.model_selection_changed = false;
    run_prompt_with_engine(
        terminal,
        events,
        shutdown,
        TuiEngineRun {
            runtime,
            engine: &assembly.engine,
            app,
            message,
            startup_events,
            requested_run_id,
            interactions,
        },
    )
    .await
}

struct TuiEngineRun<'a> {
    runtime: &'a CliRuntime,
    engine: &'a rove_runtime::engine::Engine,
    app: &'a mut TuiApp,
    message: String,
    startup_events: Vec<StreamEvent>,
    requested_run_id: Option<RunId>,
    interactions: &'a mut TuiInteractionReceiver,
}

async fn run_prompt_with_engine<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: ShutdownInput<'_>,
    input: TuiEngineRun<'_>,
) -> anyhow::Result<ActiveRunResult>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let TuiEngineRun {
        runtime,
        engine,
        app,
        message,
        startup_events,
        requested_run_id,
        interactions,
    } = input;
    discard_queued_interactions(interactions);
    let (session_id, job_id, generated_run_id) = app.next_run_identity();
    let run_id = requested_run_id.unwrap_or(generated_run_id);
    let resume_claim = app.resume_claim.take();
    let run = match runtime.state_store.start_run(session_id, job_id, run_id) {
        Ok(run) => run,
        Err(error) => {
            if let Some(claim) = resume_claim {
                let _ = runtime
                    .state_store
                    .index
                    .release_job_resume_claim_async(claim)
                    .await;
            }
            mark_claimed_successor_start_failed(
                runtime,
                app,
                run_id,
                &startup_events,
                "successor run could not be started",
            )
            .await;
            return Err(error.into());
        }
    };
    let resume_state = app.active_resume_state.clone();
    let request = run.request(message.clone(), resume_state.clone());
    let trace_writer = run.trace_writer.clone();
    let cancel = CancellationToken::new();
    let mut engine_stream = engine.run_with_cancel(request, Some(trace_writer), cancel.clone());
    let runtime_identity = Some(engine_stream.runtime_identity().clone());
    let agent_profile = engine_stream.agent_profile().cloned();
    let run_control = engine_stream.control().clone();
    let (message_events_tx, mut message_events_rx) = mpsc::unbounded_channel();
    let message_service = runtime.message_service.clone();
    let message_session_id = session_id.to_string();
    let message_observer = tokio::spawn(async move {
        let mut last_error = None;
        while let Some(event) = message_events_rx.recv().await {
            if let Err(error) = message_service
                .observe_event(&message_session_id, run_id, &event)
                .await
            {
                tracing::warn!(%error, %run_id, "failed to persist TUI message lifecycle event");
                last_error = Some(error.to_string());
            }
        }
        last_error
    });
    let stream = async_stream::stream! {
        let mut startup = startup_events;
        while let Some(event) = engine_stream.next().await {
            let is_started = matches!(event, StreamEvent::RunStarted { .. });
            if is_message_lifecycle_event(&event) {
                let _ = message_events_tx.send(event.clone());
            }
            yield event;
            if is_started {
                for startup_event in startup.drain(..) {
                    if is_message_lifecycle_event(&startup_event) {
                        let _ = message_events_tx.send(startup_event.clone());
                    }
                    yield startup_event;
                }
            }
        }
    };
    let (updates_tx, updates_rx) = mpsc::channel(RUN_UPDATE_CAPACITY);
    let driver = drive_tui_run_events(
        stream,
        TuiRunContext {
            message,
            run,
            resume_state,
            state_store: &runtime.state_store,
            workspace: &runtime.workspace,
            model_id: engine.model_id(),
            runtime_identity,
            agent_profile,
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
        updates_rx,
        interactions,
        ActiveUiControl {
            cancel,
            pressed_keys: &mut app.pressed_keys,
            message_service: &runtime.message_service,
            control: &run_control,
            session_id,
            run_id,
        },
        &mut app.state,
    );

    let (outcome, ui_result) = tokio::join!(driver, ui);
    discard_queued_interactions(interactions);
    if let Some(error) = message_observer
        .await
        .context("TUI message observer task failed")?
    {
        app.state.message_error = Some(error);
    }
    let ui_result = ui_result?;
    if !matches!(outcome.reason, TerminationReason::Cancelled)
        && let Ok(latest) = runtime.state_store.load_task_state(run_id).await
    {
        app.set_active_resume_state(latest);
    }

    Ok(ActiveRunResult {
        run_id,
        reason: outcome.reason,
        exit_requested: ui_result.exit_requested,
    })
}

fn is_message_lifecycle_event(event: &StreamEvent) -> bool {
    matches!(
        event,
        StreamEvent::MessageQueued { .. }
            | StreamEvent::MessageInterventionRequested { .. }
            | StreamEvent::MessageAppliedCurrentRun { .. }
            | StreamEvent::MessageClaimedSuccessor { .. }
            | StreamEvent::MessageNeedsAttention { .. }
            | StreamEvent::MessageRevoked { .. }
    )
}

async fn apply_active_effects(
    effects: Vec<TuiEffect>,
    interaction: &mut InteractionController,
    interactions: &mut TuiInteractionReceiver,
    control: &ActiveUiControl<'_>,
    state: &mut TuiState,
    exit_requested: &mut bool,
) {
    for effect in effects {
        match effect {
            TuiEffect::SendMessage {
                content,
                session_state: SessionDeliveryState::Active,
                ..
            } => {
                let mutation = control
                    .message_service
                    .send(
                        &control.session_id.to_string(),
                        SendMessageCommand {
                            content,
                            idempotency_key: None,
                            session_state: SessionDeliveryState::Active,
                            target_run_id: Some(control.run_id),
                        },
                    )
                    .await;
                match mutation {
                    Ok(mutation) => {
                        state.upsert_message(mutation.message.clone());
                        let accepted =
                            control
                                .control
                                .try_send_message_event(StreamEvent::MessageQueued {
                                    id: mutation.message.id.clone(),
                                    content: mutation.message.content.clone(),
                                });
                        if !accepted {
                            state.message_error =
                                Some("active run message channel is full".to_string());
                        }
                    }
                    Err(error) => state.message_error = Some(error.to_string()),
                }
            }
            TuiEffect::SendMessage { .. } => {}
            TuiEffect::PromoteMessage { message_id } => {
                match control
                    .message_service
                    .promote(&control.session_id.to_string(), &message_id)
                    .await
                {
                    Ok(message) => {
                        state.upsert_message(message.clone());
                        let message_id = message.id.clone();
                        let _ = control.control.try_send_steer(
                            rove_runtime::engine::SteerMessage::for_message(
                                message_id,
                                message.content,
                            ),
                        );
                    }
                    Err(error) => state.message_error = Some(error.to_string()),
                }
            }
            TuiEffect::RevokeMessage { message_id } => {
                match control
                    .message_service
                    .revoke(&control.session_id.to_string(), &message_id)
                    .await
                {
                    Ok(message) => {
                        state.upsert_message(message.clone());
                        let _ = control
                            .control
                            .try_send_message_event(StreamEvent::MessageRevoked { id: message.id });
                    }
                    Err(error) => state.message_error = Some(error.to_string()),
                }
            }
            TuiEffect::Dispatch(
                action @ (TerminalAction::ApproveTool { .. }
                | TerminalAction::RejectTool { .. }
                | TerminalAction::SubmitInput { .. }),
            ) => {
                // Core cannot enqueue the next legitimate request until this
                // responder is released, so only pre-existing extras are drained.
                discard_queued_interactions(interactions);
                interaction.resolve(&action);
            }
            TuiEffect::Dispatch(TerminalAction::CancelRun) => {
                control.cancel.cancel();
                clear_interactions(interaction, interactions, state);
            }
            TuiEffect::ExitAfterRun => {
                *exit_requested = true;
                control.cancel.cancel();
                clear_interactions(interaction, interactions, state);
            }
            TuiEffect::Exit => {
                *exit_requested = true;
                clear_interactions(interaction, interactions, state);
            }
            TuiEffect::Dispatch(_)
            | TuiEffect::LoadSessions
            | TuiEffect::ResolveResume { .. }
            | TuiEffect::LoadModels { .. }
            | TuiEffect::PersistModel { .. }
            | TuiEffect::ResetModel { .. }
            | TuiEffect::LoadMessages => {}
        }
    }
}

fn drain_ready_unarmed_events<E>(
    events: &mut E,
    state: &mut TuiState,
    pressed_keys: &mut PressedKeys,
) -> anyhow::Result<Vec<TuiEffect>>
where
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let mut effects = Vec::new();
    for _ in 0..MAX_ARMING_DRAIN_EVENTS {
        let Some(event) = events.next().now_or_never() else {
            return Ok(effects);
        };
        let Some(event) = event else {
            return Err(anyhow::anyhow!(
                "terminal input stream closed while arming an interaction"
            ));
        };
        let event = event?;
        let fresh_press = pressed_keys.observe(&event);
        effects.extend(reduce_event(state, event, false, fresh_press));
    }

    Err(anyhow::anyhow!(
        "terminal input backlog exceeded the interaction arming limit"
    ))
}

async fn active_ui_loop<B, E>(
    terminal: &mut Terminal<B>,
    events: &mut E,
    shutdown: ShutdownInput<'_>,
    mut updates: mpsc::Receiver<RunViewUpdate>,
    interactions: &mut TuiInteractionReceiver,
    control: ActiveUiControl<'_>,
    state: &mut TuiState,
) -> anyhow::Result<ActiveUiResult>
where
    B: Backend,
    B::Error: Error + Send + Sync + 'static,
    E: Stream<Item = io::Result<Event>> + Unpin,
{
    let mut exit_requested = false;
    let mut dirty = true;
    let mut interaction = InteractionController::default();
    let mut redraw = tokio::time::interval(FRAME_INTERVAL);
    redraw.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = redraw.tick(), if dirty || interaction.needs_arming() => {
                interaction.discard_closed_active(state);
                if interaction.needs_arming() {
                    let effects = match drain_ready_unarmed_events(
                        events,
                        state,
                        control.pressed_keys,
                    ) {
                        Ok(effects) => effects,
                        Err(error) => {
                            control.cancel.cancel();
                            clear_interactions(&mut interaction, interactions, state);
                            return Err(error);
                        }
                    };
                    apply_active_effects(
                        effects,
                        &mut interaction,
                        interactions,
                        &control,
                        state,
                        &mut exit_requested,
                    ).await;
                }
                if let Err(error) = draw_app(terminal, state) {
                    control.cancel.cancel();
                    clear_interactions(&mut interaction, interactions, state);
                    return Err(error);
                }
                interaction.after_modal_draw(control.pressed_keys.is_empty());
                dirty = false;
            }
            update = updates.recv() => {
                let Some(update) = update else {
                    control.cancel.cancel();
                    clear_interactions(&mut interaction, interactions, state);
                    return Err(anyhow::anyhow!(
                        "runtime event stream ended without a completion update"
                    ));
                };
                let completed = matches!(&update, RunViewUpdate::RunCompleted { .. });
                state.apply_run_update(update.clone());
                interaction.observe_update(&update, state);
                dirty = true;
                if completed {
                    clear_interactions(&mut interaction, interactions, state);
                    draw_app(terminal, state)?;
                    return Ok(ActiveUiResult { exit_requested });
                }
            }
            request = interactions.recv() => {
                let Some(request) = request else {
                    control.cancel.cancel();
                    clear_interactions(&mut interaction, interactions, state);
                    return Err(anyhow::anyhow!("terminal interaction channel closed during an active run"));
                };
                if control.cancel.is_cancelled()
                    || state.run_lifecycle != RunLifecycle::Running
                    || !state.interaction_key_mode.is_available()
                {
                    drop(request);
                } else {
                    interaction.offer_request(request, state);
                }
                dirty = true;
            }
            event = events.next() => {
                let Some(event) = event else {
                    control.cancel.cancel();
                    clear_interactions(&mut interaction, interactions, state);
                    return Err(anyhow::anyhow!("terminal input stream closed during an active run"));
                };
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        control.cancel.cancel();
                        clear_interactions(&mut interaction, interactions, state);
                        return Err(error.into());
                    }
                };
                let fresh_press = control.pressed_keys.observe(&event);
                let effects = reduce_event(state, event, interaction.is_armed(), fresh_press);
                apply_active_effects(
                    effects,
                    &mut interaction,
                    interactions,
                    &control,
                    state,
                    &mut exit_requested,
                ).await;
                dirty = true;
            }
            signal = shutdown.receiver.recv(), if *shutdown.open => {
                match signal {
                    Some(ShutdownSignal::Interrupt) => {
                        reduce(state, TuiAction::Terminal(TerminalAction::CancelRun));
                        control.cancel.cancel();
                        clear_interactions(&mut interaction, interactions, state);
                        dirty = true;
                    }
                    #[cfg(unix)]
                    Some(ShutdownSignal::Terminate) => {
                        reduce(state, TuiAction::Terminal(TerminalAction::CancelRun));
                        exit_requested = true;
                        control.cancel.cancel();
                        clear_interactions(&mut interaction, interactions, state);
                        dirty = true;
                    }
                    None => *shutdown.open = false,
                }
            }
        }
    }
}

fn reduce_event(
    state: &mut TuiState,
    event: Event,
    interaction_armed: bool,
    fresh_press: bool,
) -> Vec<TuiEffect> {
    match event {
        Event::Key(key)
            if state.modal.is_none() || interaction_armed || is_global_key_event(key) =>
        {
            let action = map_key_event_with_overlay_mode(
                key,
                state.modal.as_ref(),
                state.overlay.as_ref(),
                state.interaction_key_mode,
            );
            match action {
                Some(action) if fresh_press || !is_interaction_decision(&action) => {
                    reduce(state, action)
                }
                Some(_) | None => Vec::new(),
            }
        }
        Event::Key(_) => Vec::new(),
        Event::Resize(width, height) => reduce(state, TuiAction::Resize { width, height }),
        Event::Paste(_) if state.modal.is_some() && !interaction_armed => Vec::new(),
        Event::Paste(text) => text
            .chars()
            .flat_map(|ch| reduce(state, TuiAction::InsertChar(ch)))
            .collect(),
        Event::FocusGained | Event::FocusLost | Event::Mouse(_) => Vec::new(),
    }
}

fn is_interaction_decision(action: &TuiAction) -> bool {
    matches!(
        action,
        TuiAction::PrepareApproval { .. }
            | TuiAction::ApproveInteraction { .. }
            | TuiAction::RejectInteraction { .. }
            | TuiAction::SubmitInteraction { .. }
    )
}

fn is_global_key_event(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(ch) if ch.eq_ignore_ascii_case(&'c') || ch.eq_ignore_ascii_case(&'q'))
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
    use std::sync::Arc;
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use futures::stream::BoxStream;
    use futures::{StreamExt, stream};
    use ratatui::Terminal;
    use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
    use ratatui::buffer::Cell;
    use ratatui::layout::{Position, Size};
    use tokio::sync::{Notify, mpsc, oneshot};
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_util::sync::CancellationToken;

    use crate::cli::args::CliApprovalPolicy;
    use crate::cli::runtime::{CliRuntimeInteraction, CliRuntimeOptions, build_cli_runtime};
    use crate::terminal::action::TerminalAction;
    use crate::terminal::interaction::{
        TerminalInteractionProviders, TerminalInteractionRequest, bounded_interaction_channel,
    };
    use crate::terminal::view::RunViewUpdate;
    use crate::tui::action::TuiAction;
    use crate::tui::reducer::reduce;
    use crate::tui::state::{InteractionKeyMode, InteractionModalView, RunLifecycle, TuiState};
    use rove_app_bootstrap::tool_registry;
    use rove_models::ModelError;
    use rove_models::fake::{FakeModelClient, FakeTurn};
    use rove_models::{ModelClient, ModelClientId, ModelEvent};
    use rove_runtime::context::ContextManager;
    use rove_runtime::conversation::{
        MessagePageQuery, MessageStatus, SendMessageCommand, SessionDeliveryState,
    };
    use rove_runtime::engine::{Engine, EngineConfig};
    use rove_runtime::events::StreamEvent;
    use rove_runtime::types::{
        ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, ModelToolSchema, RunId,
        RunRequest, SessionId, TaskState, TerminationReason, ToolApprovalRequest, Usage,
        UserInputRequest,
    };
    use rove_runtime::workspace::Workspace;

    use super::{
        ActiveUiControl, InteractionController, PressedKeys, ShutdownInput, ShutdownSignal, TuiApp,
        TuiEngineRun, active_ui_loop, apply_idle_effects, claim_next_tui_successor,
        discard_queued_interactions, run_loop, run_prompt_with_engine, test_control,
        test_message_service,
    };

    struct FailingDrawBackend {
        inner: TestBackend,
    }

    struct GatedModelClient {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    impl ModelClient for GatedModelClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            let entered = Arc::clone(&self.entered);
            let release = Arc::clone(&self.release);
            Box::pin(
                stream::once(async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(ModelEvent::TextDelta {
                        text: "TUI_GATED_RESPONSE".to_string(),
                    })
                })
                .chain(stream::iter([Ok(ModelEvent::Usage {
                    usage: Usage::default(),
                })])),
            )
        }

        fn model_id(&self) -> &str {
            "gated-test"
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::new("fake", "local", self.model_id())
        }
    }

    impl FailingDrawBackend {
        fn new(width: u16, height: u16) -> Self {
            Self {
                inner: TestBackend::new(width, height),
            }
        }
    }

    impl Backend for FailingDrawBackend {
        type Error = io::Error;

        fn draw<'a, I>(&mut self, _content: I) -> Result<(), Self::Error>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            Err(io::Error::other("injected draw failure"))
        }

        fn hide_cursor(&mut self) -> Result<(), Self::Error> {
            self.inner.hide_cursor().map_err(|never| match never {})
        }

        fn show_cursor(&mut self) -> Result<(), Self::Error> {
            self.inner.show_cursor().map_err(|never| match never {})
        }

        fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
            self.inner
                .get_cursor_position()
                .map_err(|never| match never {})
        }

        fn set_cursor_position<P: Into<Position>>(
            &mut self,
            position: P,
        ) -> Result<(), Self::Error> {
            self.inner
                .set_cursor_position(position)
                .map_err(|never| match never {})
        }

        fn clear(&mut self) -> Result<(), Self::Error> {
            self.inner.clear().map_err(|never| match never {})
        }

        fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
            self.inner
                .clear_region(clear_type)
                .map_err(|never| match never {})
        }

        fn size(&self) -> Result<Size, Self::Error> {
            self.inner.size().map_err(|never| match never {})
        }

        fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
            self.inner.window_size().map_err(|never| match never {})
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            self.inner.flush().map_err(|never| match never {})
        }
    }

    async fn send_and_wait_for_receive<T>(sender: &mpsc::Sender<T>, value: T) {
        sender.send(value).await.unwrap();
        let permit = sender.reserve().await.unwrap();
        drop(permit);
    }

    async fn wait_until_interaction_dequeued(providers: &TerminalInteractionProviders) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match providers
                    .input_provider
                    .begin_input(
                        CallId::new(),
                        UserInputRequest {
                            prompt: "capacity probe".to_string(),
                        },
                    )
                    .await
                {
                    Ok(probe) => {
                        drop(probe);
                        return;
                    }
                    Err(error) if error.to_string().contains("full") => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("interaction probe failed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("active loop must dequeue the live interaction");
        tokio::task::yield_now().await;
    }

    fn approval_request(
        call_id: CallId,
    ) -> (
        TerminalInteractionRequest,
        oneshot::Receiver<ApprovalDecision>,
    ) {
        let (respond_to, response) = oneshot::channel();
        (
            TerminalInteractionRequest::Approval {
                request: ToolApprovalRequest {
                    call_id,
                    name: "write_file".to_string(),
                    args: serde_json::json!({"path":"out.txt"}),
                    reason: "writes a file".to_string(),
                },
                respond_to,
            },
            response,
        )
    }

    fn input_request(input_id: CallId) -> (TerminalInteractionRequest, oneshot::Receiver<String>) {
        let (respond_to, response) = oneshot::channel();
        (
            TerminalInteractionRequest::Input {
                input_id,
                request: UserInputRequest {
                    prompt: "Which branch?".to_string(),
                },
                respond_to,
            },
            response,
        )
    }

    fn approval_view(call_id: CallId) -> InteractionModalView {
        InteractionModalView::Approval {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        }
    }

    fn input_view(input_id: CallId) -> InteractionModalView {
        InteractionModalView::Input {
            input_id,
            prompt: "Which branch?".to_string(),
            draft: String::new(),
        }
    }

    fn arm_controller(controller: &mut InteractionController) {
        controller.after_modal_draw(true);
        assert!(!controller.is_armed());
        controller.after_modal_draw(true);
        assert!(controller.is_armed());
    }

    #[test]
    fn controller_requires_both_halves_and_supports_either_arrival_order() {
        let approval_id = CallId::new();
        let (approval, mut approval_response) = approval_request(approval_id);
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut controller = InteractionController::default();

        controller.offer_request(approval, &mut state);
        assert!(state.modal.is_none());
        controller.offer_view(approval_view(approval_id), &mut state);
        assert!(matches!(
            state.modal,
            Some(InteractionModalView::Approval { call_id, .. }) if call_id == approval_id
        ));
        controller.resolve(&TerminalAction::ApproveTool {
            call_id: approval_id,
        });
        assert!(matches!(
            approval_response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        arm_controller(&mut controller);

        let wrong_id = CallId::new();
        controller.resolve(&TerminalAction::ApproveTool { call_id: wrong_id });
        assert!(matches!(
            approval_response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        let effect = reduce(
            &mut state,
            TuiAction::ApproveInteraction {
                call_id: approval_id,
            },
        )
        .pop()
        .unwrap();
        let crate::tui::effect::TuiEffect::Dispatch(action) = effect else {
            panic!("approval must produce a dispatch effect");
        };
        controller.resolve(&action);
        assert_eq!(
            approval_response.try_recv().unwrap(),
            ApprovalDecision::Approve
        );
        controller.resolve(&action);

        let input_id = CallId::new();
        let (input, mut input_response) = input_request(input_id);
        controller.offer_view(input_view(input_id), &mut state);
        assert!(state.modal.is_none());
        controller.offer_request(input, &mut state);
        assert!(matches!(
            state.modal,
            Some(InteractionModalView::Input { input_id: current, .. }) if current == input_id
        ));
        arm_controller(&mut controller);
        reduce(&mut state, TuiAction::InsertChar(' '));
        reduce(&mut state, TuiAction::InsertChar(' '));
        let effect = reduce(&mut state, TuiAction::SubmitInteraction { input_id })
            .pop()
            .unwrap();
        let crate::tui::effect::TuiEffect::Dispatch(action) = effect else {
            panic!("input must produce a dispatch effect");
        };
        controller.resolve(&action);
        assert_eq!(input_response.try_recv().unwrap(), "  ");

        let empty_id = CallId::new();
        let (empty_input, mut empty_response) = input_request(empty_id);
        controller.offer_request(empty_input, &mut state);
        controller.offer_view(input_view(empty_id), &mut state);
        arm_controller(&mut controller);
        let effect = reduce(
            &mut state,
            TuiAction::SubmitInteraction { input_id: empty_id },
        )
        .pop()
        .unwrap();
        let crate::tui::effect::TuiEffect::Dispatch(action) = effect else {
            panic!("empty input must still produce a dispatch effect");
        };
        controller.resolve(&action);
        assert_eq!(empty_response.try_recv().unwrap(), "");
    }

    #[test]
    fn controller_fails_closed_for_closed_mismatched_and_extra_requests() {
        let first_id = CallId::new();
        let (closed, closed_response) = approval_request(first_id);
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut controller = InteractionController::default();
        controller.offer_request(closed, &mut state);
        drop(closed_response);
        controller.offer_view(approval_view(first_id), &mut state);
        assert!(state.modal.is_none());
        assert!(controller.active_request.is_none());

        let mismatch_id = CallId::new();
        let (mismatched, mut mismatched_response) = approval_request(mismatch_id);
        controller.offer_request(mismatched, &mut state);
        controller.offer_view(input_view(mismatch_id), &mut state);
        assert!(state.modal.is_none());
        assert!(matches!(
            mismatched_response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        controller.offer_view(approval_view(mismatch_id), &mut state);
        assert!(matches!(
            state.modal,
            Some(InteractionModalView::Approval { call_id, .. }) if call_id == mismatch_id
        ));

        let extra_id = CallId::new();
        let (extra, mut extra_response) = input_request(extra_id);
        controller.offer_request(extra, &mut state);
        assert!(matches!(
            extra_response.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(matches!(
            state.modal,
            Some(InteractionModalView::Approval { call_id, .. }) if call_id == mismatch_id
        ));

        controller.clear(&mut state);
        assert!(matches!(
            mismatched_response.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));

        let wrong_view_id = CallId::new();
        let (request, mut response) = approval_request(extra_id);
        controller.offer_request(request, &mut state);
        controller.offer_view(approval_view(wrong_view_id), &mut state);
        assert!(state.modal.is_none());
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        controller.offer_view(approval_view(extra_id), &mut state);
        assert!(state.modal.is_some());
    }

    #[tokio::test]
    async fn discarding_queued_requests_releases_waiters_before_the_next_run() {
        let (providers, mut interactions) = bounded_interaction_channel(2);
        let pending = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: CallId::new(),
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"old.txt"}),
                reason: "old run".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(discard_queued_interactions(&mut interactions), 1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), pending.resolve())
                .await
                .unwrap(),
            ApprovalDecision::Reject
        );
        assert!(interactions.is_empty());
        assert_eq!(interactions.capacity(), interactions.max_capacity());
    }

    #[tokio::test]
    async fn rejected_tui_approval_never_runs_the_destructive_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let blocked_path = workspace.root.join("blocked.txt");
        let (providers, mut interactions) = bounded_interaction_channel(2);
        let engine = Engine::with_workspace(
            Box::new(FakeModelClient::with_turns(
                "finished".to_string(),
                vec![
                    FakeTurn::ToolUse {
                        id: "write-1".to_string(),
                        name: "write_file".to_string(),
                        args: serde_json::json!({
                            "path": "blocked.txt",
                            "content": "must not be written"
                        }),
                    },
                    FakeTurn::Text("finished".to_string()),
                ],
            )),
            tool_registry(&workspace),
            ContextManager::new("You are a test agent.".to_string()),
            EngineConfig::new(3, false),
            workspace,
            ApprovalPolicy::Ask,
        )
        .with_approval_provider(providers.approval_provider.clone());
        let cancel = CancellationToken::new();
        let stream = engine.run_with_cancel(
            RunRequest {
                session_id: SessionId::new(),
                job_id: JobId::new(),
                run_id: RunId::new(),
                user_message: "write a blocked file".to_string(),
                resume_state: None,
            },
            None,
            cancel,
        );
        futures::pin_mut!(stream);
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut controller = InteractionController::default();
        let mut saw_canonical_approval = false;

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                tokio::select! {
                    request = interactions.recv() => {
                        controller.offer_request(request.unwrap(), &mut state);
                    }
                    event = stream.next() => {
                        let event = event.expect("engine must emit RunCompleted");
                        let canonical_approval =
                            matches!(event, StreamEvent::ToolCallApprovalNeeded { .. });
                        if canonical_approval {
                            saw_canonical_approval = true;
                        }
                        let completed = matches!(event, StreamEvent::RunCompleted { .. });
                        let update = RunViewUpdate::from(&event);
                        state.apply_run_update(update.clone());
                        controller.observe_update(&update, &mut state);
                        if canonical_approval {
                            assert!(
                                controller.waiting_request.is_some()
                                    || controller.active_request.is_some()
                                    || !interactions.is_empty(),
                                "canonical approval must not precede responder registration"
                            );
                        }
                        if completed {
                            break;
                        }
                    }
                }

                if let Some(InteractionModalView::Approval { call_id, .. }) = state.modal.as_ref() {
                    let call_id = *call_id;
                    arm_controller(&mut controller);
                    let effect = reduce(&mut state, TuiAction::RejectInteraction { call_id })
                        .pop()
                        .unwrap();
                    let crate::tui::effect::TuiEffect::Dispatch(action) = effect else {
                        panic!("rejection must produce a dispatch effect");
                    };
                    controller.resolve(&action);
                }
            }
        })
        .await
        .expect("rejected destructive run must finish promptly");

        assert!(saw_canonical_approval);
        assert!(!blocked_path.exists());
        assert!(state.modal.is_none());
    }

    #[tokio::test]
    async fn active_loop_approval_requires_a_matching_real_key_press() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        let _event_keepalive = event_tx.clone();
        let mut events = ReceiverStream::new(event_rx);
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        let (providers, mut interactions) = bounded_interaction_channel(1);
        let _provider_keepalive = providers.clone();
        let call_id = CallId::new();
        let pending = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id,
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"approved.txt"}),
                reason: "writes a file".to_string(),
            })
            .await
            .unwrap();
        let decision = tokio::spawn(pending.resolve());
        let cancel = CancellationToken::new();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut pressed_keys = PressedKeys::default();

        let producer = async move {
            send_and_wait_for_receive(
                &updates_tx,
                RunViewUpdate::RunStarted {
                    run_id: RunId::new(),
                    job_id: JobId::new(),
                    user_message: "approve safely".to_string(),
                },
            )
            .await;
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                ))),
            )
            .await;
            send_and_wait_for_receive(
                &updates_tx,
                RunViewUpdate::ToolCallApprovalNeeded {
                    call_id,
                    name: "write_file".to_string(),
                    args: serde_json::json!({"path":"approved.txt"}),
                    reason: "writes a file".to_string(),
                },
            )
            .await;
            wait_until_interaction_dequeued(&providers).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(!decision.is_finished());

            for event in [
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::Paste("y".to_string()),
            ] {
                send_and_wait_for_receive(&event_tx, Ok(event)).await;
            }
            tokio::task::yield_now().await;
            assert!(!decision.is_finished());
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(!decision.is_finished());

            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                ))),
            )
            .await;
            let decision = tokio::time::timeout(Duration::from_secs(1), decision)
                .await
                .unwrap()
                .unwrap();
            updates_tx
                .send(RunViewUpdate::RunCompleted {
                    reason: TerminationReason::Final,
                    output: Some("approved".to_string()),
                })
                .await
                .unwrap();
            decision
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            updates_rx,
            &mut interactions,
            ActiveUiControl {
                cancel,
                pressed_keys: &mut pressed_keys,
                message_service: test_message_service(),
                control: test_control(),
                session_id: SessionId::new(),
                run_id: RunId::new(),
            },
            &mut state,
        );

        let (decision, ui_result) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(producer, ui) })
                .await
                .unwrap();

        assert_eq!(decision, ApprovalDecision::Approve);
        ui_result.unwrap();
        assert!(state.modal.is_none());
    }

    #[tokio::test]
    async fn active_loop_function_key_mode_requires_fresh_y_then_f8() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        let _event_keepalive = event_tx.clone();
        let mut events = ReceiverStream::new(event_rx);
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        let (providers, mut interactions) = bounded_interaction_channel(1);
        let _provider_keepalive = providers.clone();
        let call_id = CallId::new();
        let pending = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id,
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"approved.txt"}),
                reason: "writes a file".to_string(),
            })
            .await
            .unwrap();
        let decision = tokio::spawn(pending.resolve());
        let cancel = CancellationToken::new();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            interaction_key_mode: InteractionKeyMode::ConfirmWithFunctionKey,
            ..TuiState::default()
        };
        let mut pressed_keys = PressedKeys::default();

        let producer = async move {
            send_and_wait_for_receive(
                &updates_tx,
                RunViewUpdate::ToolCallApprovalNeeded {
                    call_id,
                    name: "write_file".to_string(),
                    args: serde_json::json!({"path":"approved.txt"}),
                    reason: "writes a file".to_string(),
                },
            )
            .await;
            wait_until_interaction_dequeued(&providers).await;
            tokio::time::sleep(Duration::from_millis(100)).await;

            for event in [
                Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE)),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::F(8),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::Paste("y".to_string()),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::F(8),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::F(8),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
            ] {
                send_and_wait_for_receive(&event_tx, Ok(event)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(!decision.is_finished());

            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                ))),
            )
            .await;
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                ))),
            )
            .await;
            tokio::task::yield_now().await;
            assert!(!decision.is_finished());

            for event in [
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::Paste("f8".to_string()),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::F(8),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::F(8),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
            ] {
                send_and_wait_for_receive(&event_tx, Ok(event)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(!decision.is_finished());

            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE))),
            )
            .await;
            let decision = tokio::time::timeout(Duration::from_secs(1), decision)
                .await
                .unwrap()
                .unwrap();
            updates_tx
                .send(RunViewUpdate::RunCompleted {
                    reason: TerminationReason::Final,
                    output: Some("approved".to_string()),
                })
                .await
                .unwrap();
            decision
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            updates_rx,
            &mut interactions,
            ActiveUiControl {
                cancel,
                pressed_keys: &mut pressed_keys,
                message_service: test_message_service(),
                control: test_control(),
                session_id: SessionId::new(),
                run_id: RunId::new(),
            },
            &mut state,
        );

        let (decision, ui_result) =
            tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(producer, ui) })
                .await
                .unwrap();

        assert_eq!(decision, ApprovalDecision::Approve);
        ui_result.unwrap();
        assert!(state.modal.is_none());
        assert_eq!(state.approval_confirmation, None);
    }

    #[tokio::test]
    async fn active_loop_input_submits_pasted_whitespace_without_normalizing_it() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        let _event_keepalive = event_tx.clone();
        let mut events = ReceiverStream::new(event_rx);
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        let (providers, mut interactions) = bounded_interaction_channel(1);
        let _provider_keepalive = providers.clone();
        let input_id = CallId::new();
        let pending = providers
            .input_provider
            .begin_input(
                input_id,
                UserInputRequest {
                    prompt: "Exact answer?".to_string(),
                },
            )
            .await
            .unwrap();
        let answer = tokio::spawn(pending.resolve());
        let cancel = CancellationToken::new();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut pressed_keys = PressedKeys::default();

        let producer = async move {
            send_and_wait_for_receive(
                &updates_tx,
                RunViewUpdate::InputNeeded {
                    input_id,
                    prompt: "Exact answer?".to_string(),
                },
            )
            .await;
            wait_until_interaction_dequeued(&providers).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            send_and_wait_for_receive(&event_tx, Ok(Event::Paste("  \t ".to_string()))).await;
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                ))),
            )
            .await;
            let answer = tokio::time::timeout(Duration::from_secs(1), answer)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            updates_tx
                .send(RunViewUpdate::RunCompleted {
                    reason: TerminationReason::Final,
                    output: Some("answered".to_string()),
                })
                .await
                .unwrap();
            answer
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            updates_rx,
            &mut interactions,
            ActiveUiControl {
                cancel,
                pressed_keys: &mut pressed_keys,
                message_service: test_message_service(),
                control: test_control(),
                session_id: SessionId::new(),
                run_id: RunId::new(),
            },
            &mut state,
        );

        let (answer, ui_result) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(producer, ui) })
                .await
                .unwrap();

        assert_eq!(answer, "  \t ");
        ui_result.unwrap();
        assert!(state.modal.is_none());
    }

    #[tokio::test]
    async fn active_loop_rejects_interactions_when_key_event_types_are_unreliable() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut events = stream::pending::<io::Result<Event>>();
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        let (providers, mut interactions) = bounded_interaction_channel(2);
        let approval_id = CallId::new();
        let input_id = CallId::new();
        let approval = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: approval_id,
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"blocked.txt"}),
                reason: "writes a file".to_string(),
            })
            .await
            .unwrap();
        let input = providers
            .input_provider
            .begin_input(
                input_id,
                UserInputRequest {
                    prompt: "Unsafe terminal?".to_string(),
                },
            )
            .await
            .unwrap();
        let approval = tokio::spawn(approval.resolve());
        let input = tokio::spawn(input.resolve());
        let cancel = CancellationToken::new();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            interaction_key_mode: InteractionKeyMode::Unavailable,
            ..TuiState::default()
        };
        let mut pressed_keys = PressedKeys::default();

        let producer = async move {
            updates_tx
                .send(RunViewUpdate::ToolCallApprovalNeeded {
                    call_id: approval_id,
                    name: "write_file".to_string(),
                    args: serde_json::json!({"path":"blocked.txt"}),
                    reason: "writes a file".to_string(),
                })
                .await
                .unwrap();
            updates_tx
                .send(RunViewUpdate::InputNeeded {
                    input_id,
                    prompt: "Unsafe terminal?".to_string(),
                })
                .await
                .unwrap();
            let approval = approval.await.unwrap();
            let input = input.await.unwrap().unwrap_err();
            updates_tx
                .send(RunViewUpdate::RunCompleted {
                    reason: TerminationReason::Final,
                    output: None,
                })
                .await
                .unwrap();
            (approval, input)
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            updates_rx,
            &mut interactions,
            ActiveUiControl {
                cancel,
                pressed_keys: &mut pressed_keys,
                message_service: test_message_service(),
                control: test_control(),
                session_id: SessionId::new(),
                run_id: RunId::new(),
            },
            &mut state,
        );

        let ((approval, input), ui_result) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(producer, ui) })
                .await
                .unwrap();

        assert_eq!(approval, ApprovalDecision::Reject);
        assert!(input.to_string().contains("response was dropped"));
        ui_result.unwrap();
        assert!(state.modal.is_none());
    }

    #[tokio::test]
    async fn run_prompt_rejects_stale_interaction_before_polling_next_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        workspace.ensure_state_dir().unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let engine = Engine::with_workspace(
            Box::new(GatedModelClient {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
            tool_registry(&workspace),
            ContextManager::new("You are a test agent.".to_string()),
            EngineConfig::new(1, false),
            workspace.clone(),
            ApprovalPolicy::Never,
        );
        let runtime =
            crate::cli::runtime::build_cli_runtime(crate::cli::runtime::CliRuntimeOptions {
                cwd: Some(workspace.root.clone()),
                model: Some("fake".to_string()),
                max_steps: Some(1),
                agent: None,
                trust_project: false,
                approval: crate::cli::args::CliApprovalPolicy::Never,
                task_workspace: None,
                task_base: None,
                initial_fake_response: Some("unused".to_string()),
                interaction: Default::default(),
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
        let (interaction_providers, mut interactions) = bounded_interaction_channel(8);
        let stale_approval = interaction_providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: CallId::new(),
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"stale.txt"}),
                reason: "belongs to an earlier run".to_string(),
            })
            .await
            .unwrap();

        let run = Box::pin(run_prompt_with_engine(
            &mut terminal,
            &mut events,
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            TuiEngineRun {
                runtime: &runtime,
                engine: &engine,
                app: &mut app,
                message: "hello from tui".to_string(),
                startup_events: Vec::new(),
                requested_run_id: None,
                interactions: &mut interactions,
            },
        ));
        let stale_before_completion = async {
            entered.notified().await;
            let result =
                tokio::time::timeout(Duration::from_secs(1), stale_approval.resolve()).await;
            release.notify_one();
            result
        };
        let (result, stale_decision) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(run, stale_before_completion)
        })
        .await
        .unwrap();
        let result = result.unwrap();

        assert_eq!(result.reason, TerminationReason::Final);
        assert_eq!(
            stale_decision
                .expect("stale interaction must be rejected before the model is released"),
            ApprovalDecision::Reject
        );
        assert!(interactions.is_empty());
        assert_eq!(interactions.capacity(), interactions.max_capacity());
        assert_eq!(app.state.run_lifecycle, RunLifecycle::Completed);
        assert!(app.state.run.assistant_text.contains("TUI_GATED_RESPONSE"));
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
        let (event_tx, event_rx) = mpsc::channel(1);
        let _event_keepalive = event_tx.clone();
        let mut events = ReceiverStream::new(event_rx);
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (updates_tx, updates_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let cancel_for_producer = cancel.clone();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut pressed_keys = PressedKeys::default();
        let (interaction_providers, mut interactions) = bounded_interaction_channel(8);
        let _provider_keepalive = interaction_providers.clone();
        let call_id = CallId::new();
        let pending_approval = interaction_providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id,
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"cancelled.txt"}),
                reason: "writes a file".to_string(),
            })
            .await
            .unwrap();
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
            send_and_wait_for_receive(
                &updates_tx,
                RunViewUpdate::ToolCallApprovalNeeded {
                    call_id,
                    name: "write_file".to_string(),
                    args: serde_json::json!({"path":"cancelled.txt"}),
                    reason: "writes a file".to_string(),
                },
            )
            .await;
            wait_until_interaction_dequeued(&interaction_providers).await;
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ))),
            )
            .await;
            send_and_wait_for_receive(
                &event_tx,
                Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL,
                ))),
            )
            .await;
            cancel_for_producer.cancelled().await;
            let decision = tokio::time::timeout(Duration::from_secs(1), pending_approval.resolve())
                .await
                .unwrap();
            updates_tx
                .send(RunViewUpdate::RunCompleted {
                    reason: TerminationReason::Cancelled,
                    output: None,
                })
                .await
                .unwrap();
            decision
        };
        let ui = active_ui_loop(
            &mut terminal,
            &mut events,
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            updates_rx,
            &mut interactions,
            ActiveUiControl {
                cancel: cancel.clone(),
                pressed_keys: &mut pressed_keys,
                message_service: test_message_service(),
                control: test_control(),
                session_id: SessionId::new(),
                run_id: RunId::new(),
            },
            &mut state,
        );

        let (decision, result) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(producer, ui) })
                .await
                .unwrap();
        result.unwrap();

        assert!(cancel.is_cancelled());
        assert_eq!(state.run_lifecycle, RunLifecycle::Completed);
        assert!(matches!(
            state.run.completed.as_ref().map(|view| &view.reason),
            Some(TerminationReason::Cancelled)
        ));
        assert!(state.modal.is_none());
        assert_eq!(decision, ApprovalDecision::Reject);
    }

    #[tokio::test]
    async fn idle_loop_keeps_receiving_interrupts_after_the_first_one() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: None,
            agent: None,
            trust_project: false,
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
        let (_interaction_providers, mut interactions) = bounded_interaction_channel(8);

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
        let app = run_loop(
            &mut terminal,
            &mut events,
            &mut shutdown,
            &runtime,
            None,
            &mut interactions,
            InteractionKeyMode::Direct,
        );

        let ((), result) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(producer, app)
        })
        .await
        .expect("the second interrupt must still be consumed");

        assert!(result.unwrap().state.composer.is_empty());
    }

    #[tokio::test]
    async fn run_loop_propagates_terminal_interaction_mode() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: None,
            agent: None,
            trust_project: false,
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
        let mut events = stream::empty::<io::Result<Event>>();
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let (_interaction_providers, mut interactions) = bounded_interaction_channel(1);

        let app = run_loop(
            &mut terminal,
            &mut events,
            &mut shutdown,
            &runtime,
            None,
            &mut interactions,
            InteractionKeyMode::ConfirmWithFunctionKey,
        )
        .await
        .unwrap();

        assert_eq!(
            app.state.interaction_key_mode,
            InteractionKeyMode::ConfirmWithFunctionKey
        );
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
        let mut pressed_keys = PressedKeys::default();
        let (interaction_providers, mut interactions) = bounded_interaction_channel(8);
        let pending_input = interaction_providers
            .input_provider
            .begin_input(
                CallId::new(),
                UserInputRequest {
                    prompt: "must be released".to_string(),
                },
            )
            .await
            .unwrap();
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
            ShutdownInput {
                receiver: &mut shutdown,
                open: &mut shutdown_open,
            },
            updates_rx,
            &mut interactions,
            ActiveUiControl {
                cancel: cancel.clone(),
                pressed_keys: &mut pressed_keys,
                message_service: test_message_service(),
                control: test_control(),
                session_id: SessionId::new(),
                run_id: RunId::new(),
            },
            &mut state,
        );

        let ((), ui_result) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(producer, ui) })
                .await
                .expect("closed UI receiver must unblock the bounded producer");

        assert!(ui_result.is_err());
        assert!(cancel.is_cancelled());
        let error = tokio::time::timeout(Duration::from_secs(1), pending_input.resolve())
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("response was dropped"));
    }

    #[tokio::test]
    async fn draw_failure_releases_a_pending_approval_before_returning() {
        let backend = FailingDrawBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut events = stream::pending::<io::Result<Event>>();
        let (_shutdown_tx, mut shutdown) = mpsc::channel(1);
        let mut shutdown_open = true;
        let (_updates_tx, updates_rx) = mpsc::channel(1);
        let (providers, mut interactions) = bounded_interaction_channel(1);
        let pending = providers
            .approval_provider
            .begin_approval(ToolApprovalRequest {
                call_id: CallId::new(),
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"must-not-exist.txt"}),
                reason: "writes a file".to_string(),
            })
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let mut state = TuiState {
            run_lifecycle: RunLifecycle::Running,
            ..TuiState::default()
        };
        let mut pressed_keys = PressedKeys::default();

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            active_ui_loop(
                &mut terminal,
                &mut events,
                ShutdownInput {
                    receiver: &mut shutdown,
                    open: &mut shutdown_open,
                },
                updates_rx,
                &mut interactions,
                ActiveUiControl {
                    cancel: cancel.clone(),
                    pressed_keys: &mut pressed_keys,
                    message_service: test_message_service(),
                    control: test_control(),
                    session_id: SessionId::new(),
                    run_id: RunId::new(),
                },
                &mut state,
            ),
        )
        .await
        .unwrap();

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("injected draw failure")
        );
        assert!(cancel.is_cancelled());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), pending.resolve())
                .await
                .unwrap(),
            ApprovalDecision::Reject
        );
        assert!(state.modal.is_none());
    }

    #[tokio::test]
    async fn idle_session_effects_list_newest_first_and_reject_stale_resume() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: Some(1),
            agent: None,
            trust_project: false,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("ready".to_string()),
            interaction: CliRuntimeInteraction::Providers {
                input_provider: None,
                approval_provider: None,
            },
        })
        .await
        .unwrap();
        let older = TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "older session".to_string(),
            step: 1,
            history: Vec::new(),
            summary: None,
            checkpoint: None,
            plan: None,
            runtime_identity: None,
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        };
        let newer = TaskState {
            goal: "newer session".to_string(),
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            ..older.clone()
        };
        runtime.state_store.write_task_state(&older).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        runtime.state_store.write_task_state(&newer).await.unwrap();
        for state in [&older, &newer] {
            runtime
                .state_store
                .index
                .record_report(
                    state.run_id,
                    &runtime
                        .state_store
                        .run_store
                        .run_dir(&state.run_id)
                        .join("report.json"),
                    "success",
                    "final",
                )
                .unwrap();
        }

        let mut app = TuiApp::default();
        let effects = reduce(&mut app.state, TuiAction::OpenSessionPicker);
        apply_idle_effects(effects, &runtime, &mut app)
            .await
            .unwrap();
        let Some(crate::tui::state::TuiOverlay::SessionPicker(picker)) = app.state.overlay.as_ref()
        else {
            panic!("expected session picker");
        };
        assert_eq!(picker.candidates()[0].run_id, newer.run_id);

        let effects = reduce(&mut app.state, TuiAction::ConfirmOverlay);
        apply_idle_effects(effects, &runtime, &mut app)
            .await
            .unwrap();
        assert_eq!(
            app.active_resume_state.as_ref().map(|state| state.run_id),
            Some(newer.run_id)
        );

        let mut stale_app = TuiApp::default();
        let effects = reduce(&mut stale_app.state, TuiAction::OpenSessionPicker);
        apply_idle_effects(effects, &runtime, &mut stale_app)
            .await
            .unwrap();
        let stale_path = runtime
            .state_store
            .run_store
            .run_dir(&newer.run_id)
            .join("task_state.json");
        tokio::fs::remove_file(stale_path).await.unwrap();
        let effects = reduce(&mut stale_app.state, TuiAction::ConfirmOverlay);
        apply_idle_effects(effects, &runtime, &mut stale_app)
            .await
            .unwrap();
        assert!(stale_app.active_resume_state.is_none());
        assert!(matches!(
            stale_app.state.overlay,
            Some(crate::tui::state::TuiOverlay::SessionPicker(_))
        ));
    }

    #[tokio::test]
    async fn completed_tui_turn_claims_queued_successors_in_fifo_order() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: Some(1),
            agent: None,
            trust_project: false,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("ready".to_string()),
            interaction: CliRuntimeInteraction::Providers {
                input_provider: None,
                approval_provider: None,
            },
        })
        .await
        .unwrap();
        let mut app = TuiApp::default();
        let session_id = app.session_id.to_string();
        let first = runtime
            .message_service
            .send(
                &session_id,
                SendMessageCommand {
                    content: "first queued".to_string(),
                    idempotency_key: Some("first-queued".to_string()),
                    session_state: SessionDeliveryState::Active,
                    target_run_id: None,
                },
            )
            .await
            .unwrap();
        let second = runtime
            .message_service
            .send(
                &session_id,
                SendMessageCommand {
                    content: "second queued".to_string(),
                    idempotency_key: Some("second-queued".to_string()),
                    session_state: SessionDeliveryState::Active,
                    target_run_id: None,
                },
            )
            .await
            .unwrap();

        let (first_content, first_events, first_run_id) =
            claim_next_tui_successor(&runtime, &mut app)
                .await
                .expect("the completed turn must start its FIFO successor");
        assert_eq!(first_content, first.message.content);
        assert!(first_run_id.is_some());
        assert!(matches!(
            first_events.as_slice(),
            [StreamEvent::MessageClaimedSuccessor { id }] if id == &first.message.id
        ));

        let (second_content, second_events, _) = claim_next_tui_successor(&runtime, &mut app)
            .await
            .expect("the second queued message must remain available");
        assert_eq!(second_content, second.message.content);
        assert!(matches!(
            second_events.as_slice(),
            [StreamEvent::MessageClaimedSuccessor { id }] if id == &second.message.id
        ));
        assert!(claim_next_tui_successor(&runtime, &mut app).await.is_none());

        let page = runtime
            .message_service
            .list(&session_id, MessagePageQuery::latest(2))
            .await
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .map(|message| message.status)
                .collect::<Vec<_>>(),
            vec![
                MessageStatus::ClaimedSuccessor,
                MessageStatus::ClaimedSuccessor
            ]
        );
    }
}
