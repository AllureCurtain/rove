use futures::StreamExt;
use rove::config::{AppConfig, AppConfigOverrides};
use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::types::ApprovalPolicy;
use rove::core::workspace::Workspace;
use rove::models::factory::build_model_client;
use rove::tools::default_tool_registry;

const SMOKE_PHRASE: &str = "rove provider smoke ok";
const TOOL_SMOKE_PHRASE: &str = "rove provider tool smoke ok";

fn smoke_enabled(name: &str) -> bool {
    std::env::var(name).ok().as_deref() == Some("1")
}

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must be set when the matching provider smoke gate is enabled")
    })
}

#[derive(Default)]
struct ProviderSmokeResult {
    final_output: String,
    terminal_reason: String,
    saw_tool_call: bool,
    saw_tool_output: bool,
    event_names: Vec<&'static str>,
}

async fn run_provider_smoke(provider: &str, model: String, message: &str) -> ProviderSmokeResult {
    let workspace = Workspace::detect(std::env::current_dir().unwrap().as_path()).unwrap();
    let mut config = AppConfig::load(
        &workspace.root,
        AppConfigOverrides {
            model: Some(model.clone()),
            max_steps: Some(3),
            api_bind_addr: None,
        },
    )
    .unwrap();
    config.provider.name = provider.to_string();
    config.provider.model = model;
    config.provider.fallback_models.clear();
    config.provider.fallback_providers.clear();

    let model = build_model_client(&config, config.provider.model.clone());
    let engine = Engine::with_workspace(
        model,
        default_tool_registry(&workspace),
        ContextManager::new(config.load_system_prompt()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Never,
    );

    let mut stream = engine.ask(message.to_string(), None);
    let mut result = ProviderSmokeResult::default();
    while let Some(event) = stream.next().await {
        result.event_names.push(event.event_name());
        match event {
            StreamEvent::ToolCallStarted { name, .. } if name == "echo" => {
                result.saw_tool_call = true;
            }
            StreamEvent::ToolCallCompleted { result: output, .. }
                if output.output.contains(TOOL_SMOKE_PHRASE) =>
            {
                result.saw_tool_output = true;
            }
            StreamEvent::RunCompleted { reason, output } => {
                result.terminal_reason = format!("{reason:?}");
                result.final_output = output.unwrap_or_default();
                break;
            }
            _ => {}
        }
    }
    result
}

async fn assert_provider_smoke(provider: &str, model: String) {
    let final_answer = run_provider_smoke(
        provider,
        model.clone(),
        "Reply with exactly: rove provider smoke ok",
    )
    .await;
    assert!(
        final_answer
            .final_output
            .to_ascii_lowercase()
            .contains(SMOKE_PHRASE),
        "unexpected provider smoke output: {}",
        final_answer.final_output
    );

    let tool_use = run_provider_smoke(
        provider,
        model,
        "Use the echo tool exactly once with message \"rove provider tool smoke ok\", then reply with exactly: rove provider smoke ok",
    )
    .await;
    assert!(
        tool_use.saw_tool_call,
        "provider smoke did not emit an echo tool call"
    );
    assert!(
        tool_use.saw_tool_output,
        "provider smoke did not complete echo tool output containing {TOOL_SMOKE_PHRASE}"
    );
    assert!(
        matches!(tool_use.terminal_reason.as_str(), "Final" | "StepLimit"),
        "provider smoke tool-use run did not reach an acceptable terminal reason: output={:?}, reason={}, events={:?}, saw_tool_call={}, saw_tool_output={}",
        tool_use.final_output,
        tool_use.terminal_reason,
        tool_use.event_names,
        tool_use.saw_tool_call,
        tool_use.saw_tool_output
    );
}

#[tokio::test]
async fn openai_compatible_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OPENAI") {
        return;
    }
    require_env("OPENAI_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OPENAI_MODEL")
        .unwrap_or_else(|_| "gpt-4.1-mini".to_string());
    assert_provider_smoke("openai-compatible", model).await;
}

#[tokio::test]
async fn anthropic_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_ANTHROPIC") {
        return;
    }
    require_env("ANTHROPIC_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-3-5-haiku-latest".to_string());
    assert_provider_smoke("anthropic", model).await;
}

#[tokio::test]
async fn ollama_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OLLAMA") {
        return;
    }
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OLLAMA_MODEL")
        .unwrap_or_else(|_| "llama3.2".to_string());
    assert_provider_smoke("ollama", model).await;
}
