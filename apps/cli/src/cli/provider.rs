use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEventKind, read};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use rove_app_bootstrap::{
    OnboardingCredential, ProviderCatalogService, ProviderMigrationOptions,
    ProviderMigrationSource, ProviderOnboardingRequest, ProviderOnboardingService,
    ProviderProfileId, SecretSource, run_provider_migration,
};

use super::args::ProviderCommand;

pub async fn run(
    cwd: Option<PathBuf>,
    trust_project: bool,
    command: ProviderCommand,
) -> anyhow::Result<()> {
    match command {
        ProviderCommand::Add {
            profile,
            label,
            provider,
            base_url,
            model,
            secret_env,
            secret_file,
            no_credential,
            no_use,
            expected_revision,
        } => {
            let profile_id = ProviderProfileId::new(profile)?;
            let credential = match (secret_env, secret_file, no_credential) {
                (Some(env), None, false) => {
                    OnboardingCredential::Reference(SecretSource::Env { env })
                }
                (None, Some(file), false) => {
                    OnboardingCredential::Reference(SecretSource::File { file })
                }
                (None, None, true) => OnboardingCredential::None,
                (None, None, false) => {
                    let secret = tokio::task::spawn_blocking(|| {
                        read_secret("Provider API key (stored in OS keyring): ")
                    })
                    .await??;
                    OnboardingCredential::Secret(secret)
                }
                _ => anyhow::bail!("provider_add_invalid_credential_source"),
            };
            let receipt = ProviderOnboardingService::discover()
                .onboard(ProviderOnboardingRequest {
                    profile_id: profile_id.clone(),
                    label: label.unwrap_or_else(|| profile_id.to_string()),
                    provider_type: provider,
                    base_url,
                    model,
                    credential,
                    make_default: !no_use,
                    expected_revision,
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(())
        }
        ProviderCommand::Test { profile, model } => {
            let profile_id = ProviderProfileId::new(profile)?;
            let receipt = ProviderOnboardingService::discover()
                .probe(&profile_id, model.as_deref())
                .await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(())
        }
        ProviderCommand::Use {
            profile,
            model,
            expected_revision,
        } => {
            let selection = ProviderOnboardingService::discover().use_profile(
                &ProviderProfileId::new(profile)?,
                model.as_deref(),
                expected_revision.as_deref(),
            )?;
            println!("{}", serde_json::to_string_pretty(&selection)?);
            Ok(())
        }
        ProviderCommand::List => {
            let catalog = ProviderCatalogService::discover().load()?;
            let profiles = catalog
                .profiles()
                .into_iter()
                .map(|profile| {
                    let credential_source = match profile.auth_source {
                        rove_app_bootstrap::CredentialReference::Env { .. } => "environment",
                        rove_app_bootstrap::CredentialReference::File { .. } => "file",
                        rove_app_bootstrap::CredentialReference::Keyring { .. } => "keyring",
                        rove_app_bootstrap::CredentialReference::None => "none",
                    };
                    serde_json::json!({
                        "id": profile.id,
                        "label": profile.label,
                        "provider_type": profile.provider_type,
                        "base_url": profile.base_url,
                        "model": profile.model,
                        "credential_source": credential_source,
                        "fallback": profile.fallback,
                    })
                })
                .collect::<Vec<_>>();
            let output = serde_json::json!({
                "revision": catalog.revision(),
                "default": catalog.default_selection().ok(),
                "profiles": profiles,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
            Ok(())
        }
        ProviderCommand::Migrate {
            apply,
            rewrite_workspace_config,
            product_store,
            rename,
        } => run_migration(
            cwd,
            trust_project,
            apply,
            rewrite_workspace_config,
            product_store,
            rename,
        ),
    }
}

fn run_migration(
    cwd: Option<PathBuf>,
    trust_project: bool,
    apply: bool,
    rewrite_workspace_config: bool,
    product_store: Option<PathBuf>,
    rename: Vec<String>,
) -> anyhow::Result<()> {
    let workspace_root = cwd
        .unwrap_or(std::env::current_dir()?)
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("migration workspace is unavailable: {error}"))?;
    let renames = parse_renames(rename)?;
    let report = run_provider_migration(
        &ProviderCatalogService::discover(),
        ProviderMigrationOptions {
            workspace_root: workspace_root.clone(),
            trusted_workspace: trust_project,
            product_store_path: product_store
                .or_else(|| Some(workspace_root.join(".rove/product.sqlite"))),
            renames,
            apply,
            rewrite_workspace_config,
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.conflicts.is_empty() {
        anyhow::bail!(
            "provider_migration_conflict: rerun with --rename SOURCE:PROFILE=NEW_PROFILE"
        );
    }
    Ok(())
}

fn read_secret(prompt: &str) -> anyhow::Result<String> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(prompt.as_bytes())?;
    stderr.flush()?;
    enable_raw_mode()?;
    let result = read_secret_raw();
    let restore = disable_raw_mode();
    stderr.write_all(b"\r\n")?;
    stderr.flush()?;
    restore?;
    result
}

fn read_secret_raw() -> anyhow::Result<String> {
    let mut secret = String::new();
    loop {
        let Event::Key(key) = read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Enter => break,
            KeyCode::Esc => anyhow::bail!("provider_add_cancelled"),
            KeyCode::Backspace => {
                secret.pop();
            }
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT,
                ) && secret.len().saturating_add(ch.len_utf8()) <= 16 * 1024 =>
            {
                secret.push(ch);
            }
            _ => {}
        }
    }
    if secret.trim().is_empty() {
        anyhow::bail!("provider_add_empty_credential");
    }
    Ok(secret)
}

fn parse_renames(values: Vec<String>) -> anyhow::Result<BTreeMap<String, String>> {
    let mut renames = BTreeMap::new();
    for value in values {
        let (source_id, target) = value.split_once('=').ok_or_else(|| {
            anyhow::anyhow!(
                "provider_migration_invalid_rename: expected SOURCE:PROFILE=NEW_PROFILE"
            )
        })?;
        let (source, profile) = source_id.split_once(':').ok_or_else(|| {
            anyhow::anyhow!(
                "provider_migration_invalid_rename: expected SOURCE:PROFILE=NEW_PROFILE"
            )
        })?;
        let source = ProviderMigrationSource::parse(source).ok_or_else(|| {
            anyhow::anyhow!(
                "provider_migration_invalid_rename: source must be workspace, environment, or product_store"
            )
        })?;
        if profile.is_empty() || target.is_empty() {
            anyhow::bail!("provider_migration_invalid_rename: profile ids must not be empty");
        }
        let key = format!("{}:{profile}", source.as_str());
        if renames.insert(key, target.to_string()).is_some() {
            anyhow::bail!("provider_migration_invalid_rename: duplicate source mapping");
        }
    }
    Ok(renames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_parser_is_typed_and_rejects_unknown_sources() {
        let parsed = parse_renames(vec!["workspace:old=new".to_string()]).unwrap();
        assert_eq!(parsed["workspace:old"], "new");
        assert!(parse_renames(vec!["other:old=new".to_string()]).is_err());
        assert!(parse_renames(vec!["workspace:old".to_string()]).is_err());
    }
}
