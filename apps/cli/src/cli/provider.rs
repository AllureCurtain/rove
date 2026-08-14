use std::collections::BTreeMap;
use std::path::PathBuf;

use rove_app_bootstrap::{
    ProviderCatalogService, ProviderMigrationOptions, ProviderMigrationSource,
    run_provider_migration,
};

use super::args::ProviderCommand;

pub fn run(
    cwd: Option<PathBuf>,
    trust_project: bool,
    command: ProviderCommand,
) -> anyhow::Result<()> {
    match command {
        ProviderCommand::Migrate {
            apply,
            rewrite_workspace_config,
            product_store,
            rename,
        } => {
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
    }
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
