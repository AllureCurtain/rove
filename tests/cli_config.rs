use std::path::PathBuf;

use rove::config::AppConfig;
use rove::interfaces::cli::config::format_effective_config;

#[test]
fn format_effective_config_prints_json_without_secret_value() {
    let config = AppConfig {
        api_base: "https://example.test/v1".to_string(),
        api_key: "secret-token".to_string(),
        model: "model-a".to_string(),
        max_steps: 42,
        system_prompt_path: PathBuf::from("prompts/custom.md"),
        mcp_config_path: PathBuf::from(".rove/custom-mcp.json"),
    };

    let output = format_effective_config(&config);
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["api_base"], "https://example.test/v1");
    assert_eq!(json["api_key_set"], true);
    assert_eq!(json["model"], "model-a");
    assert_eq!(json["max_steps"], 42);
    assert_eq!(json["system_prompt_path"], "prompts/custom.md");
    assert_eq!(json["mcp_config_path"], ".rove/custom-mcp.json");
    assert!(!output.contains("secret-token"));
}
