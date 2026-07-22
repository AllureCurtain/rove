use std::path::PathBuf;

use rove_app_bootstrap::{AppConfig, AppConfigOverrides};
use rove_runtime::workspace::Workspace;

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
                "name": provider.name,
                "api_base": provider.api_base,
                "api_key_set": !provider.api_key.is_empty(),
                "model": provider.model,
                "options": provider.options,
            })
        })
        .collect();
    let value = serde_json::json!({
        "runtime": {
            "max_steps": config.runtime.max_steps,
            "system_prompt_path": config.runtime.system_prompt_path.to_string_lossy(),
            "planner_prompt_path": config.runtime.planner_prompt_path.to_string_lossy(),
            "model_compaction_enabled": config.runtime.model_compaction_enabled,
            "compaction_failure_threshold": config.runtime.compaction_failure_threshold,
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
            "options": config.provider.options,
            "fallback_models": config.provider.fallback_models,
            "fallback_providers": fallback_providers,
        },
        "tool": {
            "mcp_config_path": config.tool.mcp_config_path.to_string_lossy(),
            "shell": {
                "timeout_ms": config.tool.shell.timeout_ms,
                "max_output_bytes": config.tool.shell.max_output_bytes,
                "inherit_environment": config.tool.shell.inherit_environment,
                "denylist": config.tool.shell.denylist,
            },
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
            "retry_max_attempts": config.routing.retry_max_attempts,
            "retry_backoff_base_ms": config.routing.retry_backoff_base_ms,
            "retry_backoff_max_ms": config.routing.retry_backoff_max_ms,
        },
        "rag": {
            "deterministic": config.rag.deterministic,
            "embedding_provider": config.rag.embedding_provider,
            "embedding_model": config.rag.embedding_model,
            "embedding_api_base": config.rag.embedding_api_base,
            "embedding_api_key_set": !config.rag.embedding_api_key.is_empty(),
            "rerank_provider": config.rag.rerank_provider,
            "rerank_model": config.rag.rerank_model,
            "rerank_api_key_set": config.rag.rerank_api_key.as_deref().is_some_and(|key| !key.is_empty()),
            "timeout_ms": config.rag.timeout_ms,
            "fallback_to_deterministic": config.rag.fallback_to_deterministic,
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
            "planner_prompt_path": config.resolve_path(&config.runtime.planner_prompt_path).to_string_lossy(),
            "mcp_config_path": config.resolve_path(&config.tool.mcp_config_path).to_string_lossy(),
            "state_dir": config.state_dir().to_string_lossy(),
            "sqlite_path": config.sqlite_path().to_string_lossy(),
            "memory_session_dir": config.resolve_path(&config.memory.session_dir).to_string_lossy(),
            "memory_durable_dir": config.resolve_path(&config.memory.durable_dir).to_string_lossy(),
        },
    });
    serde_json::to_string_pretty(&value).expect("effective config snapshot should serialize")
}
