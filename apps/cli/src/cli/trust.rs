use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use rove_app_bootstrap::{
    PROJECT_TRUST_INVALID_INPUT_CODE, PROJECT_TRUST_UNAVAILABLE_CODE, ProjectActivationState,
    ProjectTrustDecision, ProjectTrustRepository, ProjectTrustResolution, capability_digest_map,
    provider_capability_selector_for_workspace,
};
use rove_runtime::workspace::Workspace;
use serde::Serialize;

use crate::cli::args::{CliProjectTrustCapability, TrustCommand};

#[derive(Debug)]
pub struct TrustCommandError {
    pub code: &'static str,
    pub message: String,
}

impl TrustCommandError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for TrustCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TrustCommandError {}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct CliTrustStatus {
    state: ProjectActivationState,
    identity_digest: String,
    invalidated_capabilities: Vec<String>,
    granted_capabilities: Vec<String>,
}

pub fn run(cwd: Option<String>, command: TrustCommand) -> anyhow::Result<()> {
    let cwd = cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd).map_err(|error| {
        TrustCommandError::new(
            PROJECT_TRUST_INVALID_INPUT_CODE,
            format!("workspace is invalid: {error}"),
        )
    })?;
    let repository = ProjectTrustRepository::operator_default().map_err(|error| {
        TrustCommandError::new(
            PROJECT_TRUST_UNAVAILABLE_CODE,
            format!("project trust authority is unavailable: {error}"),
        )
    })?;
    let status = execute(&repository, &workspace, command)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&status).map_err(|error| {
            TrustCommandError::new(
                PROJECT_TRUST_UNAVAILABLE_CODE,
                format!("project trust status could not be encoded: {error}"),
            )
        })?
    );
    Ok(())
}

fn execute(
    repository: &ProjectTrustRepository,
    workspace: &Workspace,
    command: TrustCommand,
) -> Result<CliTrustStatus, TrustCommandError> {
    let provider_selector = provider_capability_selector_for_workspace(&workspace.root);
    let all_digests = capability_digest_map(&workspace.root, None, Some(&provider_selector));
    let (decision, selected) = match command {
        TrustCommand::Query { capability } => {
            let selected = selected_names(&capability)?;
            let resolution = repository
                .resolve(&workspace.root, workspace.kind.clone(), &all_digests)
                .map_err(authority_error)?;
            return Ok(status_from_resolution(resolution, selected));
        }
        TrustCommand::Grant { capability } => {
            (ProjectTrustDecision::Grant, selected_names(&capability)?)
        }
        TrustCommand::Deny { capability } => {
            (ProjectTrustDecision::Deny, selected_names(&capability)?)
        }
        TrustCommand::Revoke { capability } => {
            (ProjectTrustDecision::Revoke, selected_names(&capability)?)
        }
    };
    let selected_digests = if selected.is_empty() {
        match decision {
            ProjectTrustDecision::Grant => all_digests.clone(),
            ProjectTrustDecision::Deny | ProjectTrustDecision::Revoke => BTreeMap::new(),
        }
    } else {
        all_digests
            .iter()
            .filter(|(capability, _)| selected.contains(*capability))
            .map(|(capability, digest)| (capability.clone(), digest.clone()))
            .collect()
    };
    repository
        .decide(
            &workspace.root,
            workspace.kind.clone(),
            decision,
            selected_digests,
        )
        .map_err(authority_error)?;
    let resolution = repository
        .resolve(&workspace.root, workspace.kind.clone(), &all_digests)
        .map_err(authority_error)?;
    Ok(status_from_resolution(resolution, BTreeSet::new()))
}

fn selected_names(
    capabilities: &[CliProjectTrustCapability],
) -> Result<BTreeSet<String>, TrustCommandError> {
    let mut selected = BTreeSet::new();
    for capability in capabilities {
        if !selected.insert(capability.as_str().to_string()) {
            return Err(TrustCommandError::new(
                PROJECT_TRUST_INVALID_INPUT_CODE,
                "project trust capability list contains duplicates",
            ));
        }
    }
    Ok(selected)
}

fn status_from_resolution(
    resolution: ProjectTrustResolution,
    selected: BTreeSet<String>,
) -> CliTrustStatus {
    let include = |capability: &String| selected.is_empty() || selected.contains(capability);
    CliTrustStatus {
        state: resolution.state,
        identity_digest: resolution.identity_digest,
        invalidated_capabilities: resolution
            .invalidated_capabilities
            .into_iter()
            .filter(include)
            .collect(),
        granted_capabilities: resolution
            .granted_capabilities
            .into_iter()
            .filter(include)
            .collect(),
    }
}

fn authority_error(error: anyhow::Error) -> TrustCommandError {
    TrustCommandError::new(
        PROJECT_TRUST_UNAVAILABLE_CODE,
        format!("project trust authority failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_cli_operations_are_capability_scoped_and_revocable() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let workspace = Workspace::detect(&root).unwrap();
        let repository = ProjectTrustRepository::new(temp.path().join("trust.sqlite"));

        let unknown = execute(
            &repository,
            &workspace,
            TrustCommand::Query {
                capability: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(unknown.state, ProjectActivationState::Unknown);

        let granted = execute(
            &repository,
            &workspace,
            TrustCommand::Grant {
                capability: vec![
                    CliProjectTrustCapability::ProjectConfiguration,
                    CliProjectTrustCapability::ProviderCredentials,
                ],
            },
        )
        .unwrap();
        assert_eq!(granted.state, ProjectActivationState::Trusted);
        assert_eq!(
            granted.granted_capabilities,
            vec![
                "project_configuration".to_string(),
                "provider_credentials".to_string()
            ]
        );

        let partially_revoked = execute(
            &repository,
            &workspace,
            TrustCommand::Revoke {
                capability: vec![CliProjectTrustCapability::ProviderCredentials],
            },
        )
        .unwrap();
        assert_eq!(partially_revoked.state, ProjectActivationState::Trusted);
        assert_eq!(
            partially_revoked.granted_capabilities,
            vec!["project_configuration".to_string()]
        );

        let denied = execute(
            &repository,
            &workspace,
            TrustCommand::Deny {
                capability: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(denied.state, ProjectActivationState::Restricted);
        assert!(denied.granted_capabilities.is_empty());
    }

    #[test]
    fn duplicate_cli_capabilities_use_the_shared_invalid_input_code() {
        let error = selected_names(&[
            CliProjectTrustCapability::McpProcesses,
            CliProjectTrustCapability::McpProcesses,
        ])
        .unwrap_err();
        assert_eq!(error.code, PROJECT_TRUST_INVALID_INPUT_CODE);
    }
}
