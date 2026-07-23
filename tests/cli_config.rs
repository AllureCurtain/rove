use rove_app_bootstrap::{AppConfig, FallbackProviderConfig, ProviderOptions};
use rove_cli::cli::config::format_effective_config;

#[test]
fn format_effective_config_prints_json_without_secret_value() {
    let mut config = AppConfig::default();
    config.provider.name = "openai".to_string();
    config.provider.api_base = "https://example.test/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.anthropic_api_key = "anthropic-secret".to_string();
    config.provider.model = "model-a".to_string();
    config.provider.fallback_models = vec!["model-b".to_string(), "model-c".to_string()];
    config.provider.fallback_providers = vec![FallbackProviderConfig {
        name: "anthropic".to_string(),
        api_base: "https://fallback.test/v1".to_string(),
        api_key: "fallback-secret".to_string(),
        model: "fallback-provider-model".to_string(),
        options: Some(ProviderOptions {
            max_tokens: Some(512),
            temperature: Some(0.3),
            ..Default::default()
        }),
    }];
    config.provider.options = ProviderOptions {
        max_tokens: Some(2048),
        temperature: Some(0.2),
        top_p: Some(0.8),
        frequency_penalty: Some(0.3),
        presence_penalty: Some(0.4),
    };
    config.routing.failure_threshold = 5;
    config.routing.open_cooldown_ms = 45_000;
    config.routing.retry_max_attempts = 4;
    config.routing.retry_backoff_base_ms = 500;
    config.routing.retry_backoff_max_ms = 8_000;
    config.runtime.max_steps = 42;
    config.runtime.system_prompt_path = "prompts/custom.md".into();
    config.runtime.planner_prompt_path = "prompts/custom-planner.md".into();
    config.runtime.model_compaction_enabled = true;
    config.runtime.compaction_failure_threshold = 2;
    config.tool.mcp_config_path = ".rove/custom-mcp.json".into();
    config.tool.shell.timeout_ms = 1_234;
    config.tool.shell.max_output_bytes = 4_096;
    config.tool.shell.inherit_environment = false;
    config.tool.shell.denylist = vec!["shutdown".to_string()];
    config.memory.session_dir = "custom-memory/sessions".into();
    config.memory.durable_dir = "custom-memory/durable".into();
    config.memory.recall_limit = 4;
    config.source_summary.workspace_root = std::path::PathBuf::from("D:/workspace");

    let output = format_effective_config(&config);
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["provider"]["name"], "openai");
    assert_eq!(json["provider"]["api_base"], "https://example.test/v1");
    assert_eq!(json["provider"]["api_key_set"], true);
    assert_eq!(json["provider"]["anthropic_api_key_set"], true);
    assert_eq!(json["provider"]["model"], "model-a");
    assert_eq!(json["provider"]["options"]["max_tokens"], 2048);
    assert_eq!(json["provider"]["options"]["temperature"], 0.2);
    assert_eq!(json["provider"]["options"]["top_p"], 0.8);
    assert_eq!(json["provider"]["options"]["frequency_penalty"], 0.3);
    assert_eq!(json["provider"]["options"]["presence_penalty"], 0.4);
    assert_eq!(json["provider"]["fallback_models"][0], "model-b");
    assert_eq!(json["provider"]["fallback_models"][1], "model-c");
    assert_eq!(
        json["provider"]["fallback_providers"][0]["name"],
        "anthropic"
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["api_base"],
        "https://fallback.test/v1"
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["api_key_set"],
        true
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["model"],
        "fallback-provider-model"
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["options"]["max_tokens"],
        512
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["options"]["temperature"],
        0.3
    );
    assert_eq!(json["routing"]["failure_threshold"], 5);
    assert_eq!(json["routing"]["open_cooldown_ms"], 45_000);
    assert_eq!(json["routing"]["retry_max_attempts"], 4);
    assert_eq!(json["routing"]["retry_backoff_base_ms"], 500);
    assert_eq!(json["routing"]["retry_backoff_max_ms"], 8_000);
    assert_eq!(json["runtime"]["max_steps"], 42);
    assert_eq!(json["runtime"]["system_prompt_path"], "prompts/custom.md");
    assert_eq!(
        json["runtime"]["planner_prompt_path"],
        "prompts/custom-planner.md"
    );
    assert_eq!(json["runtime"]["model_compaction_enabled"], true);
    assert_eq!(json["runtime"]["compaction_failure_threshold"], 2);
    assert_eq!(json["tool"]["mcp_config_path"], ".rove/custom-mcp.json");
    assert_eq!(json["tool"]["shell"]["timeout_ms"], 1_234);
    assert_eq!(json["tool"]["shell"]["max_output_bytes"], 4_096);
    assert_eq!(json["tool"]["shell"]["inherit_environment"], false);
    assert_eq!(json["tool"]["shell"]["denylist"][0], "shutdown");
    assert_eq!(json["memory"]["session_dir"], "custom-memory/sessions");
    assert_eq!(json["memory"]["durable_dir"], "custom-memory/durable");
    assert_eq!(json["memory"]["recall_limit"], 4);
    assert!(json.get("rag").is_none());
    let resolved_session_dir = json["resolved_paths"]["memory_session_dir"]
        .as_str()
        .unwrap()
        .replace('\\', "/");
    let resolved_durable_dir = json["resolved_paths"]["memory_durable_dir"]
        .as_str()
        .unwrap()
        .replace('\\', "/");
    assert!(resolved_session_dir.ends_with("D:/workspace/custom-memory/sessions"));
    assert!(resolved_durable_dir.ends_with("D:/workspace/custom-memory/durable"));
    assert!(!output.contains("secret-token"));
    assert!(!output.contains("fallback-secret"));
    assert!(!output.contains("anthropic-secret"));
}
