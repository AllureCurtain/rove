use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::types::{JobId, Message, RunId, RunRequest, SessionId, ToolSchema, Usage};
use rove::errors::ModelError;
use rove::models::traits::{ModelClient, StreamChunk};
use rove::tools::echo::EchoTool;
use rove::tools::registry::ToolRegistry;

/// A fake model client that returns predetermined responses.
struct FakeModelClient {
    responses: Vec<String>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl FakeModelClient {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelClient for FakeModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "No more responses configured".to_string());

        Box::pin(futures::stream::once(async move {
            Ok(StreamChunk {
                delta: response,
                usage: Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                }),
            })
        }))
    }

    fn model_id(&self) -> &str {
        "fake-model"
    }
}

fn build_test_engine(responses: Vec<String>) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let context_manager = ContextManager::new("You are a test agent.".to_string());
    let config = EngineConfig { max_steps: 5 };
    Engine::new(model, registry, context_manager, config)
}

/// Collect all events from a run into a Vec.
async fn collect_events(engine: &Engine, message: &str) -> Vec<StreamEvent> {
    let stream = engine.ask(message.to_string(), None);
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

async fn collect_events_with_request(engine: &Engine, req: RunRequest) -> Vec<StreamEvent> {
    let stream = engine.run(req, None);
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn run_started_event_uses_request_ids() {
    let engine = build_test_engine(vec!["done".to_string()]);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "say hello".to_string(),
    };

    let events = collect_events_with_request(&engine, req.clone()).await;

    match &events[0] {
        StreamEvent::RunStarted { run_id, job_id, .. } => {
            assert_eq!(*run_id, req.run_id);
            assert_eq!(*job_id, req.job_id);
        }
        other => panic!("Expected RunStarted, got {:?}", other),
    }
}

#[tokio::test]
async fn engine_produces_final_answer() {
    let engine = build_test_engine(vec!["Hello from the agent!".to_string()]);
    let events = collect_events(&engine, "say hello").await;

    // Should have: RunStarted, LlmChunk, LlmMessage, RunCompleted
    assert!(events.len() >= 3);

    // First event is RunStarted
    assert!(matches!(&events[0], StreamEvent::RunStarted { .. }));

    // Last event is RunCompleted with Final reason
    let last = events.last().unwrap();
    match last {
        StreamEvent::RunCompleted { reason, output } => {
            assert_eq!(*reason, rove::core::types::TerminationReason::Final);
            assert!(output.is_some());
            assert_eq!(output.as_deref().unwrap(), "Hello from the agent!");
        }
        _ => panic!("Expected RunCompleted, got {:?}", last),
    }
}

#[tokio::test]
async fn engine_handles_tool_call() {
    // First response: tool call JSON. Second response: final answer.
    let engine = build_test_engine(vec![
        r#"{"tool": "echo", "args": {"message": "ping"}}"#.to_string(),
        "The echo returned: ping".to_string(),
    ]);
    let events = collect_events(&engine, "echo ping").await;

    // Should contain ToolCallStarted and ToolCallCompleted
    let has_tool_start = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallStarted { name, .. } if name == "echo"));
    let has_tool_complete = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallCompleted { .. }));

    assert!(has_tool_start, "Missing ToolCallStarted event");
    assert!(has_tool_complete, "Missing ToolCallCompleted event");

    // Should end with RunCompleted::Final
    let last = events.last().unwrap();
    assert!(matches!(
        last,
        StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            ..
        }
    ));
}

#[tokio::test]
async fn engine_respects_step_limit() {
    // Model always returns tool calls — should hit step limit
    let responses: Vec<String> = (0..10)
        .map(|_| r#"{"tool": "echo", "args": {"message": "loop"}}"#.to_string())
        .collect();
    let engine = build_test_engine(responses);
    let events = collect_events(&engine, "keep going").await;

    let last = events.last().unwrap();
    assert!(matches!(
        last,
        StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::StepLimit,
            ..
        }
    ));
}

#[tokio::test]
async fn engine_handles_unknown_tool() {
    let engine = build_test_engine(vec![
        r#"{"tool": "nonexistent", "args": {}}"#.to_string(),
        "Sorry, that tool doesn't exist.".to_string(),
    ]);
    let events = collect_events(&engine, "use bad tool").await;

    let has_tool_failed = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallFailed { .. }));
    assert!(has_tool_failed, "Missing ToolCallFailed event");
}

#[tokio::test]
async fn trace_writer_records_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let trace_path = tmp.path().join("trace.jsonl");
    let trace_writer = rove::state::trace::TraceWriter::new(tmp.path()).unwrap();

    let engine = build_test_engine(vec!["done".to_string()]);
    let stream = engine.ask("test".to_string(), Some(trace_writer));
    futures::pin_mut!(stream);
    while stream.next().await.is_some() {}

    // Verify trace file was written
    let content = std::fs::read_to_string(&trace_path).unwrap();
    assert!(!content.is_empty());

    // Each line should be valid JSON
    for line in content.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(parsed.get("type").is_some());
    }
}
