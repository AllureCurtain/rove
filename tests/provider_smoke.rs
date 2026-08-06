use std::collections::BTreeMap;

use futures::StreamExt;
use rove_app_bootstrap::build_model_client;
use rove_app_bootstrap::tool_registry;
use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, ProviderAuthConfig, ProviderProfileConfig, SecretSource,
};
use rove_models::ProviderOptions;
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig};
use rove_runtime::events::StreamEvent;
use rove_runtime::types::ApprovalPolicy;
use rove_runtime::workspace::Workspace;

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

fn smoke_profile(provider_type: &str, model: &str) -> ProviderProfileConfig {
    let (base_url, auth) = match provider_type {
        "openai" | "openai-responses" => (
            std::env::var("ROVE_PROVIDER_SMOKE_API_BASE")
                .or_else(|_| std::env::var("OPENAI_API_BASE"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            ProviderAuthConfig::Bearer {
                secret: SecretSource::Env {
                    env: "OPENAI_API_KEY".to_string(),
                },
            },
        ),
        "anthropic" => (
            std::env::var("ROVE_PROVIDER_SMOKE_ANTHROPIC_BASE")
                .unwrap_or_else(|_| "https://api.anthropic.com".to_string()),
            ProviderAuthConfig::Header {
                header: "x-api-key".to_string(),
                secret: SecretSource::Env {
                    env: "ANTHROPIC_API_KEY".to_string(),
                },
            },
        ),
        "ollama" => (
            std::env::var("ROVE_PROVIDER_SMOKE_OLLAMA_BASE")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            ProviderAuthConfig::None,
        ),
        other => panic!("unsupported smoke provider_type `{other}`"),
    };
    ProviderProfileConfig {
        provider_type: provider_type.to_string(),
        base_url,
        model: model.to_string(),
        auth,
        headers: BTreeMap::new(),
        options: ProviderOptions::default(),
        protocol_options: serde_json::json!({}),
    }
}

async fn run_provider_smoke(
    provider_type: &str,
    model: String,
    message: &str,
) -> ProviderSmokeResult {
    let workspace = Workspace::detect(std::env::current_dir().unwrap().as_path()).unwrap();
    let mut config = AppConfig::load(
        &workspace.root,
        AppConfigOverrides {
            model: Some(model.clone()),
            max_steps: Some(3),
            api_bind_addr: None,
            trust_project: false,
        },
    )
    .unwrap();
    let mut profiles = BTreeMap::new();
    profiles.insert("smoke".to_string(), smoke_profile(provider_type, &model));
    config.provider.active = Some("smoke".to_string());
    config.provider.profiles = profiles;
    config.provider.fallback_profiles.clear();
    config.provider.fallback_models.clear();
    config.provider.model = model.clone();

    let model_client = build_model_client(&config, model);
    let engine = Engine::with_workspace(
        model_client,
        tool_registry(&workspace),
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

async fn assert_provider_smoke(provider_type: &str, model: String) {
    let final_answer = run_provider_smoke(
        provider_type,
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
        provider_type,
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
async fn openai_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OPENAI") {
        return;
    }
    require_env("OPENAI_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OPENAI_MODEL")
        .unwrap_or_else(|_| "gpt-4.1-mini".to_string());
    assert_provider_smoke("openai", model).await;
}

#[tokio::test]
async fn openai_responses_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES") {
        return;
    }
    require_env("OPENAI_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL")
        .unwrap_or_else(|_| "gpt-4.1-mini".to_string());
    assert_provider_smoke("openai-responses", model).await;
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
    let model =
        std::env::var("ROVE_PROVIDER_SMOKE_OLLAMA_MODEL").unwrap_or_else(|_| "llama3".to_string());
    assert_provider_smoke("ollama", model).await;
}
