//! Native OS notifications driven by the embedded API's event streams.
//!
//! A background pump subscribes to `GET /product/events` — the durable,
//! `Last-Event-ID`-resumable directory feed — for turn-boundary facts, and
//! follows each running session's `GET /jobs/{job_id}/events` for the
//! approval and input gates that only exist on the run's own stream.
//! Notifications fire only while no Rove window is focused; a user already
//! looking at the console does not need a banner saying so.
//!
//! Wiring is deliberately two lines in `lib.rs`: register the plugin with
//! `.plugin(tauri_plugin_notification::init())` on the builder, then
//! `notifications::spawn(...)` inside `setup`.

use std::collections::HashMap;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::ApiServerState;

/// Durable product directory feed. Frames carry the dotted event kind in
/// `event:`, the durable `seq` in `id:`, and the versioned `ProductEvent`
/// envelope in `data:`.
const PRODUCT_EVENTS_PATH: &str = "/product/events";

/// Reconnect delay after a stream drops or a request fails. Doubles on each
/// consecutive failure up to [`MAX_BACKOFF`]; resets on a successful connect.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A lifecycle fact that warrants a native notification.
///
/// Text is deliberately short and free of secrets, paths, and message content:
/// the wording comes from a fixed vocabulary, never from model output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alert {
    /// A turn finished; the outcome distinguishes success, failure, and an
    /// explicit cancel so the body never claims a success it did not see.
    TaskFinished(Outcome),
    /// The session needs a human: an interrupted turn, a failed launch, or an
    /// ambiguous boundary the supervisor refused to guess at.
    NeedsAttention,
    /// The run is parked on an explicit gate — a destructive tool call, a
    /// drafted plan, or a mid-run question that names the kind of gate.
    ApprovalNeeded(&'static str),
}

/// How a finished turn ended, mirroring the product store's `last_outcome`
/// vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Success,
    Failed,
    Cancelled,
}

/// Per-frame accumulator for the SSE fields the notification decision reads.
/// `event:` names the fact, `data:` carries the envelope, `id:` drives the
/// `Last-Event-ID` resume cursor.
#[derive(Debug, Default)]
struct SseFrame {
    event: Option<String>,
    data: String,
    id: Option<String>,
}

impl SseFrame {
    /// Parse one wire line into this frame. Comments, unknown fields, and
    /// field-only lines are skipped — a malformed line produces an empty frame
    /// the classifier ignores, never a panic that kills the pump.
    fn feed(&mut self, line: &str) {
        if line.starts_with(':') {
            return;
        }
        let Some((field, value)) = line.split_once(':') else {
            return;
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_string()),
            "id" => self.id = Some(value.to_string()),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }
    }
}

/// Everything that ends one connection attempt. Cancel is its own variant so
/// shutdown logs as a quiet stop instead of a stream failure.
enum PumpError {
    Cancelled,
    Http(reqwest::Error),
    Status(reqwest::StatusCode),
}

/// Spawn the notification pump on Tauri's async runtime.
///
/// `bearer_token` is the same loopback credential the Webview injection uses;
/// it travels only inside this process and into the `Authorization` header of
/// a request to `127.0.0.1`, never into a log or a notification. `cancel`
/// should be the token the embedded API shutdown also cancels — the pump
/// stops on whichever of stream end, cancellation, or app exit happens first.
pub fn spawn(
    state: ApiServerState,
    app: AppHandle,
    bearer_token: String,
    cancel: CancellationToken,
) {
    tauri::async_runtime::spawn(run(state, app, bearer_token, cancel));
}

/// Shared context the product pump and every job watcher borrow.
struct Pump {
    client: reqwest::Client,
    base_url: String,
    bearer: String,
    app: AppHandle,
    cancel: CancellationToken,
    /// One open job stream per running session, keyed by product session id so
    /// a `running` event never forks a second watcher onto the same job.
    job_watchers: HashMap<String, tauri::async_runtime::JoinHandle<()>>,
}

async fn run(state: ApiServerState, app: AppHandle, bearer: String, cancel: CancellationToken) {
    let Some(base_url) = state.base_url.clone() else {
        debug!("desktop notifications disabled: embedded API has no base URL");
        return;
    };
    let mut pump = Pump {
        client: reqwest::Client::new(),
        base_url,
        bearer,
        app,
        cancel,
        job_watchers: HashMap::new(),
    };
    // `None` follows from the newest retained event, matching the web client's
    // first connect; a `Some(seq)` resumes exactly where the last connection
    // left off via `Last-Event-ID`.
    let mut cursor: Option<i64> = None;
    let mut backoff = INITIAL_BACKOFF;

    info!("desktop notification pump subscribed to {PRODUCT_EVENTS_PATH}");
    loop {
        match product_stream(&mut pump, &mut cursor).await {
            Ok(()) => backoff = INITIAL_BACKOFF,
            Err(PumpError::Cancelled) => break,
            Err(PumpError::Http(error)) => {
                warn!("product event stream failed: {error}");
            }
            Err(PumpError::Status(status)) => {
                // A 409 means the cursor fell outside the retained window or
                // the log was reset; dropping it resynchronizes the stream.
                if status == reqwest::StatusCode::CONFLICT {
                    debug!("product event cursor expired; resubscribing without it");
                    cursor = None;
                } else {
                    warn!("product event stream returned status {status}");
                }
            }
        }

        let wait = backoff;
        backoff = (backoff * 2).min(MAX_BACKOFF);
        tokio::select! {
            _ = pump.cancel.cancelled() => break,
            _ = tokio::time::sleep(wait) => {}
        }
    }
    info!("desktop notification pump stopped");
}

/// One subscription pass over the product directory stream: connect, read
/// frames until the stream ends or the token cancels, dispatching each frame
/// to notifications and job watchers. `cursor` is updated in place so a
/// reconnect resumes instead of replaying.
async fn product_stream(pump: &mut Pump, cursor: &mut Option<i64>) -> Result<(), PumpError> {
    let url = format!("{}{PRODUCT_EVENTS_PATH}", pump.base_url);
    let mut request = pump
        .client
        .get(&url)
        .bearer_auth(&pump.bearer)
        .header(reqwest::header::ACCEPT, "text/event-stream");
    if let Some(seq) = *cursor {
        request = request.header("Last-Event-ID", seq.to_string());
    }
    let mut response = tokio::select! {
        _ = pump.cancel.cancelled() => return Err(PumpError::Cancelled),
        response = request.send() => response.map_err(PumpError::Http)?,
    };
    if !response.status().is_success() {
        return Err(PumpError::Status(response.status()));
    }

    let mut frame = SseFrame::default();
    let mut buffer = String::new();
    loop {
        let chunk = tokio::select! {
            _ = pump.cancel.cancelled() => return Err(PumpError::Cancelled),
            chunk = response.chunk() => chunk.map_err(PumpError::Http)?,
        };
        let Some(chunk) = chunk else {
            return Ok(());
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer.drain(..=line_end);
            if line.is_empty() {
                dispatch_product_frame(pump, &mut frame, cursor).await;
            } else {
                frame.feed(&line);
            }
        }
    }
}

/// End-of-frame marker for the product stream: update the resume cursor, then
/// act on the fact. The cursor advances on every frame — alert or not — so a
/// reconnect never replays events already seen.
async fn dispatch_product_frame(pump: &mut Pump, frame: &mut SseFrame, cursor: &mut Option<i64>) {
    if let Some(seq) = frame.id.as_deref().and_then(|id| id.parse::<i64>().ok()) {
        *cursor = Some(seq);
    }
    if frame.event.as_deref() == Some("session.status_changed") {
        let body: serde_json::Value = serde_json::from_str(&frame.data).unwrap_or_default();
        let summary = body
            .get("summary")
            .and_then(|summary| summary.as_str())
            .and_then(|summary| serde_json::from_str::<serde_json::Value>(summary).ok());
        let status = summary
            .as_ref()
            .and_then(|summary| summary.get("status"))
            .and_then(|status| status.as_str());
        match status {
            Some("idle") => {
                let outcome = summary
                    .as_ref()
                    .and_then(|summary| summary.get("last_outcome"))
                    .and_then(|outcome| outcome.as_str());
                notify_if_unfocused(
                    &pump.app,
                    Alert::TaskFinished(match outcome {
                        Some("failed") => Outcome::Failed,
                        Some("cancelled") => Outcome::Cancelled,
                        _ => Outcome::Success,
                    }),
                );
            }
            // Both mean the turn stopped needing human recovery; the user
            // response is identical, so they share one alert.
            Some("needs_attention") | Some("error") => {
                notify_if_unfocused(&pump.app, Alert::NeedsAttention);
            }
            // A turn just started: open the run's own stream so approval and
            // input gates — which never reach the directory feed — still
            // surface while the window is unfocused.
            Some("running") => {
                let session_id = body
                    .get("session_id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string);
                let workspace_id = body
                    .get("workspace_id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string);
                if let (Some(session_id), Some(workspace_id)) = (session_id, workspace_id) {
                    pump.start_job_watcher(session_id, workspace_id);
                }
            }
            _ => {}
        }
    }
    *frame = SseFrame::default();
}

impl Pump {
    /// Fork one watcher onto the running session's job stream. If the session
    /// already has a watcher — a resume re-fires `running` on connect — the
    /// existing handle is kept rather than opening a duplicate stream.
    fn start_job_watcher(&mut self, session_id: String, workspace_id: String) {
        if self
            .job_watchers
            .get(&session_id)
            .is_some_and(|handle| !handle.inner().is_finished())
        {
            return;
        }
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let bearer = self.bearer.clone();
        let app = self.app.clone();
        let cancel = self.cancel.clone();
        let key = session_id.clone();
        self.job_watchers.insert(
            key,
            tauri::async_runtime::spawn(watch_job(
                client,
                base_url,
                bearer,
                session_id,
                workspace_id,
                app,
                cancel,
            )),
        );
    }
}

/// Resolve the session's current job from its runtime binding, then follow the
/// job's own event stream for approval and input gates. Runs the watcher to
/// the run's terminal event; a session that goes `running` again later spawns
/// a fresh watcher for the new job.
async fn watch_job(
    client: reqwest::Client,
    base_url: String,
    bearer: String,
    session_id: String,
    workspace_id: String,
    app: AppHandle,
    cancel: CancellationToken,
) {
    let Some(job_id) = latest_job_id(&client, &base_url, &bearer, &workspace_id, &session_id).await
    else {
        return;
    };
    // The product event that spawned this watcher already named the turn's
    // start, so resume from `0` and let the classifier skip the kinds it does
    // not alert on; approval gates that predated the watcher are still pending
    // in the replay and surface correctly.
    let mut cursor = 0u64;
    let mut backoff = INITIAL_BACKOFF;
    loop {
        match job_stream(
            &client,
            &base_url,
            &bearer,
            &job_id,
            &mut cursor,
            &app,
            &cancel,
        )
        .await
        {
            // The job stream ends at the run's terminal event; the product
            // stream's own status_changed frame delivers the outcome alert.
            Ok(()) | Err(PumpError::Cancelled) => return,
            Err(PumpError::Http(error)) => {
                warn!("job {job_id} event stream failed: {error}");
            }
            Err(PumpError::Status(status)) => {
                // A terminal job replays from the store and still ends after
                // its last frame; a 404 means the job outlived its record.
                debug!("job {job_id} event stream returned status {status}");
                return;
            }
        }
        let wait = backoff;
        backoff = (backoff * 2).min(MAX_BACKOFF);
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// The one directory lookup a job watcher needs: the session list is the only
/// surface that exposes `runtime_binding.latest_job_id`. Scoped by the
/// workspace the event named and paged through `next_cursor` until the session
/// appears, so a workspace with more than one page of sessions still resolves.
async fn latest_job_id(
    client: &reqwest::Client,
    base_url: &str,
    bearer: &str,
    workspace_id: &str,
    session_id: &str,
) -> Option<String> {
    let mut page_cursor: Option<String> = None;
    loop {
        let mut url = format!("{base_url}/product/sessions?workspace_id={workspace_id}&limit=200");
        if let Some(cursor) = page_cursor.as_deref() {
            // The cursor is an opaque server token that may carry `+`, `/`,
            // or `=`, so it goes through form encoding rather than a raw
            // concat that would corrupt the query string.
            url.push_str("&cursor=");
            url.push_str(
                &url::form_urlencoded::byte_serialize(cursor.as_bytes()).collect::<String>(),
            );
        }
        let page: serde_json::Value = client
            .get(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        if let Some(job_id) = page
            .get("sessions")?
            .as_array()?
            .iter()
            .find(|session| session.get("id").and_then(|id| id.as_str()) == Some(session_id))
            .and_then(|session| session.get("runtime_binding"))
            .and_then(|binding| binding.get("latest_job_id"))
            .and_then(|id| id.as_str())
        {
            return Some(job_id.to_string());
        }
        match page.get("next_cursor").and_then(|c| c.as_str()) {
            Some(next) => page_cursor = Some(next.to_string()),
            None => return None,
        }
    }
}

/// One subscription pass over a job's event stream, reading frames until the
/// run's terminal event or cancellation. `cursor` is a `u64` event seq — the
/// job stream's own `Last-Event-ID` domain, distinct from the product `i64`.
async fn job_stream(
    client: &reqwest::Client,
    base_url: &str,
    bearer: &str,
    job_id: &str,
    cursor: &mut u64,
    app: &AppHandle,
    cancel: &CancellationToken,
) -> Result<(), PumpError> {
    let url = format!("{base_url}/jobs/{job_id}/events");
    let mut request = client
        .get(&url)
        .bearer_auth(bearer)
        .header(reqwest::header::ACCEPT, "text/event-stream");
    if *cursor > 0 {
        request = request.header("Last-Event-ID", cursor.to_string());
    }
    let mut response = tokio::select! {
        _ = cancel.cancelled() => return Err(PumpError::Cancelled),
        response = request.send() => response.map_err(PumpError::Http)?,
    };
    if !response.status().is_success() {
        return Err(PumpError::Status(response.status()));
    }

    let mut frame = SseFrame::default();
    let mut buffer = String::new();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(PumpError::Cancelled),
            chunk = response.chunk() => chunk.map_err(PumpError::Http)?,
        };
        let Some(chunk) = chunk else {
            return Ok(());
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer.drain(..=line_end);
            if line.is_empty() {
                if let Some(alert) = classify_job_frame(&frame) {
                    notify_if_unfocused(app, alert);
                }
                if let Some(seq) = frame.id.as_deref().and_then(|id| id.parse::<u64>().ok()) {
                    *cursor = seq;
                }
                // A terminal frame ends this watcher; the product stream owns
                // the outcome notification, so nothing here needs the reason.
                if is_terminal_job_frame(&frame) {
                    return Ok(());
                }
                frame = SseFrame::default();
            } else {
                frame.feed(&line);
            }
        }
    }
}

/// Job-stream `event:` names that mean the run is blocked on a human. These
/// are the canonical `StreamEvent` kind strings, shared with trace and replay.
fn classify_job_frame(frame: &SseFrame) -> Option<Alert> {
    match frame.event.as_deref()? {
        // A destructive tool call is parked until the user answers.
        "tool_call_approval_needed" => {
            Some(Alert::ApprovalNeeded("A tool call is waiting for approval"))
        }
        // A drafted plan waits for approval before its first step runs.
        "plan_approval_needed" => Some(Alert::ApprovalNeeded("A plan is waiting for approval")),
        // The run asked a direct question mid-turn.
        "input_needed" => Some(Alert::ApprovalNeeded("A run is waiting for input")),
        _ => None,
    }
}

/// `run_completed` is the job stream's only terminal kind, so the watcher
/// ends on it rather than reading a closed run's replay to its end.
fn is_terminal_job_frame(frame: &SseFrame) -> bool {
    frame.event.as_deref() == Some("run_completed")
}

/// Fire the native notification unless any Rove window currently holds focus.
/// Checking at send time — rather than tracking focus events — means a window
/// that just gained focus suppresses a queued alert correctly.
fn notify_if_unfocused(app: &AppHandle, alert: Alert) {
    if app
        .webview_windows()
        .values()
        .any(|window| window.is_focused().unwrap_or(false))
    {
        return;
    }
    let (title, body) = match alert {
        Alert::TaskFinished(Outcome::Success) => ("Task finished", "Run completed successfully"),
        Alert::TaskFinished(Outcome::Cancelled) => ("Task finished", "Run was cancelled"),
        Alert::TaskFinished(Outcome::Failed) => {
            ("Task finished", "Run ended without a final answer")
        }
        Alert::NeedsAttention => ("Task needs attention", "A run is waiting for your input"),
        Alert::ApprovalNeeded(body) => ("Approval needed", body),
    };
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        // Permission is requested lazily by the plugin on the first show, so a
        // denial lands here as an error rather than a panic; the pump keeps
        // running because a later grant should not need a restart.
        debug!("native notification could not be shown: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event: &str, data: &str) -> SseFrame {
        SseFrame {
            event: Some(event.to_string()),
            data: data.to_string(),
            id: None,
        }
    }

    #[test]
    fn job_gate_events_alert() {
        assert_eq!(
            classify_job_frame(&frame("tool_call_approval_needed", "{}")),
            Some(Alert::ApprovalNeeded("A tool call is waiting for approval"))
        );
        assert_eq!(
            classify_job_frame(&frame("plan_approval_needed", "{}")),
            Some(Alert::ApprovalNeeded("A plan is waiting for approval"))
        );
        assert_eq!(
            classify_job_frame(&frame("input_needed", "{}")),
            Some(Alert::ApprovalNeeded("A run is waiting for input"))
        );
    }

    #[test]
    fn job_stream_other_kinds_stay_quiet() {
        for kind in [
            "run_started",
            "llm_message",
            "tool_call_started",
            "tool_call_completed",
            "plan_created",
            "run_completed",
            "steer_applied",
            "message_queued",
            "subagent_event",
        ] {
            assert_eq!(classify_job_frame(&frame(kind, "{}")), None, "{kind}");
        }
    }

    #[test]
    fn run_completed_ends_the_watcher() {
        assert!(is_terminal_job_frame(&frame("run_completed", "{}")));
        assert!(!is_terminal_job_frame(&frame(
            "tool_call_approval_needed",
            "{}"
        )));
        assert!(!is_terminal_job_frame(&SseFrame::default()));
    }

    #[test]
    fn sse_lines_accumulate_multiline_data() {
        let mut frame = SseFrame::default();
        frame.feed("event: session.status_changed");
        frame.feed("id: 41");
        frame.feed("data: {\"v\":1,");
        frame.feed("data: \"summary\":\"{}\"}");
        assert_eq!(frame.event.as_deref(), Some("session.status_changed"));
        assert_eq!(frame.id.as_deref(), Some("41"));
        assert_eq!(frame.data, "{\"v\":1,\n\"summary\":\"{}\"}");
    }

    #[test]
    fn sse_lines_skip_comments_and_junk() {
        let mut frame = SseFrame::default();
        frame.feed(": keep-alive");
        frame.feed("retry: 3000");
        frame.feed("bogus-line-without-colon");
        frame.feed("event: input_needed");
        frame.feed("data:{}");
        assert_eq!(frame.event.as_deref(), Some("input_needed"));
        assert_eq!(frame.id, None);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut backoff = INITIAL_BACKOFF;
        let mut delays = Vec::new();
        for _ in 0..8 {
            delays.push(backoff);
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
        assert_eq!(
            delays,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                MAX_BACKOFF,
                MAX_BACKOFF,
                MAX_BACKOFF,
            ]
        );
    }
}
