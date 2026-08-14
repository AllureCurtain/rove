use std::path::PathBuf;

use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, ProviderAuthConfig, ProviderHeaderValue, ProviderProfileConfig,
    SecretSource,
};
use rove_runtime::workspace::Workspace;

pub fn run(cwd: Option<PathBuf>, overrides: AppConfigOverrides) -> anyhow::Result<()> {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    let config = AppConfig::load(&workspace.root, overrides)?;
    println!("{}", format_effective_config(&config));
    Ok(())
}

pub fn format_effective_config(config: &AppConfig) -> String {
    let profiles = config
        .provider
        .profiles
        .iter()
        .map(|(name, profile)| (name.clone(), profile_summary(profile)))
        .collect::<serde_json::Map<_, _>>();
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
            "active": config.provider.active,
            "profiles": profiles,
            "fallback_profiles": config.provider.fallback_profiles,
            "model": config.provider.model,
            "options": config.provider.options,
            "fallback_models": config.provider.fallback_models,
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
        "sources": {
            "workspace_root": config.source_summary.workspace_root.to_string_lossy(),
            "user_config_path": config.source_summary.user_config_path.to_string_lossy(),
            "user_config_present": config.source_summary.user_config_present,
            "user_config_loaded": config.source_summary.user_config_loaded,
            "user_config_revision": config.source_summary.user_config_revision,
            "project_config_path": config.source_summary.project_config_path.to_string_lossy(),
            "project_config_present": config.source_summary.project_config_present,
            "project_config_loaded": config.source_summary.project_config_loaded,
            "project_activation": config.source_summary.project_activation,
            "project_activation_source": config.source_summary.project_activation_source,
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

fn profile_summary(profile: &ProviderProfileConfig) -> serde_json::Value {
    let headers = profile
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), header_source_summary(value)))
        .collect::<serde_json::Map<_, _>>();
    let mut protocol_option_keys = profile
        .protocol_options
        .as_object()
        .map(|options| options.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    protocol_option_keys.sort();
    serde_json::json!({
        "provider_type": profile.provider_type,
        "base_url": profile.base_url,
        "model": profile.model,
        "auth": auth_summary(&profile.auth),
        "headers": headers,
        "options": profile.options,
        "protocol_option_keys": protocol_option_keys,
    })
}

fn auth_summary(auth: &ProviderAuthConfig) -> serde_json::Value {
    match auth {
        ProviderAuthConfig::None => serde_json::json!({ "style": "none" }),
        ProviderAuthConfig::Bearer { secret } => serde_json::json!({
            "style": "bearer",
            "secret": secret_source_summary(secret),
        }),
        ProviderAuthConfig::Header { header, secret } => serde_json::json!({
            "style": "header",
            "header": header,
            "secret": secret_source_summary(secret),
        }),
    }
}

fn header_source_summary(value: &ProviderHeaderValue) -> serde_json::Value {
    match value {
        ProviderHeaderValue::Literal(_) => {
            serde_json::json!({ "source": "literal", "value_set": true })
        }
        ProviderHeaderValue::Env { env } => {
            serde_json::json!({ "source": "env", "name": env })
        }
        ProviderHeaderValue::File { file } => serde_json::json!({
            "source": "file",
            "path": file.to_string_lossy(),
        }),
        ProviderHeaderValue::Keyring { keyring } => serde_json::json!({
            "source": "keyring",
            "service": keyring.service,
            "account": keyring.account,
        }),
    }
}

fn secret_source_summary(source: &SecretSource) -> serde_json::Value {
    match source {
        SecretSource::Env { env } => {
            serde_json::json!({ "source": "env", "name": env })
        }
        SecretSource::File { file } => serde_json::json!({
            "source": "file",
            "path": file.to_string_lossy(),
        }),
        SecretSource::Keyring { keyring } => serde_json::json!({
            "source": "keyring",
            "service": keyring.service,
            "account": keyring.account,
        }),
        SecretSource::Literal(_) => {
            serde_json::json!({ "source": "literal", "value_set": true })
        }
    }
}
