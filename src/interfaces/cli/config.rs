use std::path::PathBuf;

use crate::config::{AppConfig, AppConfigOverrides};
use crate::core::workspace::Workspace;

pub fn run(cwd: Option<PathBuf>, overrides: AppConfigOverrides) -> anyhow::Result<()> {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    let config = AppConfig::load(&workspace.root, overrides)?;
    println!("{}", format_effective_config(&config));
    Ok(())
}

pub fn format_effective_config(config: &AppConfig) -> String {
    let fallback_providers: Vec<_> = config
        .provider
        .fallback_providers
        .iter()
        .map(|provider| {
            serde_json::json!({
                "api_base": provider.api_base,
                "api_key_set": !provider.api_key.is_empty(),
                "model": provider.model,
            })
        })
        .collect();
    let value = serde_json::json!({
        "runtime": {
            "max_steps": config.runtime.max_steps,
            "system_prompt_path": config.runtime.system_prompt_path.to_string_lossy(),
            "context_soft_limit_tokens": config.runtime.context_soft_limit_tokens,
            "context_hard_limit_tokens": config.runtime.context_hard_limit_tokens,
            "context_reserved_tokens": config.runtime.context_reserved_tokens,
        },
        "provider": {
            "name": config.provider.name,
            "api_base": config.provider.api_base,
            "api_key_set": !config.provider.api_key.is_empty(),
            "anthropic_api_key_set": !config.provider.anthropic_api_key.is_empty(),
            "model": config.provider.model,
            "fallback_models": config.provider.fallback_models,
            "fallback_providers": fallback_providers,
        },
        "tool": {
            "mcp_config_path": config.tool.mcp_config_path.to_string_lossy(),
        },
        "memory": {
            "session_dir": config.memory.session_dir.to_string_lossy(),
            "durable_dir": config.memory.durable_dir.to_string_lossy(),
            "recall_limit": config.memory.recall_limit,
        },
        "state": {
            "state_dir": config.state.state_dir.to_string_lossy(),
            "sqlite_path": config.state.sqlite_path.to_string_lossy(),
            "lazy_migration": config.state.lazy_migration,
            "sqlite_busy_timeout_ms": config.state.sqlite_busy_timeout_ms,
            "allow_external_paths": config.state.allow_external_paths,
        },
        "api": {
            "bind_addr": config.api.bind_addr,
            "token_auth_set": config.api.token_auth.as_deref().is_some_and(|token| !token.is_empty()),
            "unsafe_remote_without_auth": config.api.unsafe_remote_without_auth,
            "cors_origins": config.api.cors_origins,
            "rate_limit_per_minute": config.api.rate_limit_per_minute,
        },
        "web": {
            "api_base": config.web.api_base,
        },
        "routing": {
            "failure_threshold": config.routing.failure_threshold,
            "open_cooldown_ms": config.routing.open_cooldown_ms,
        },
        "sources": {
            "workspace_root": config.source_summary.workspace_root.to_string_lossy(),
            "project_config_path": config.source_summary.project_config_path.to_string_lossy(),
            "project_config_loaded": config.source_summary.project_config_loaded,
            "env_keys": config.source_summary.env_keys,
            "cli_keys": config.source_summary.cli_keys,
        },
        "resolved_paths": {
            "system_prompt_path": config.resolve_path(&config.runtime.system_prompt_path).to_string_lossy(),
            "mcp_config_path": config.resolve_path(&config.tool.mcp_config_path).to_string_lossy(),
            "state_dir": config.state_dir().to_string_lossy(),
            "sqlite_path": config.sqlite_path().to_string_lossy(),
            "memory_session_dir": config.resolve_path(&config.memory.session_dir).to_string_lossy(),
            "memory_durable_dir": config.resolve_path(&config.memory.durable_dir).to_string_lossy(),
        },
    });
    serde_json::to_string_pretty(&value).expect("effective config snapshot should serialize")
}
