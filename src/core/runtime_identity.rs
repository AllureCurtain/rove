use serde::{Deserialize, Serialize};

use crate::core::prompt_metadata::{stable_hash, tool_signature};
use crate::core::types::{ApprovalPolicy, ToolSchema};
use crate::core::workspace::{Workspace, WorkspaceKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub cwd: String,
    pub workspace_kind: WorkspaceKind,
    pub model_id: String,
    pub provider_target: String,
    pub approval_policy: ApprovalPolicy,
    pub max_steps: u32,
    pub plan_enabled: bool,
    pub system_prompt_hash: String,
    pub planner_prompt_hash: String,
    pub workspace_fingerprint: String,
    pub tool_signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentityEvaluation {
    pub status: RuntimeIdentityStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mismatch_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeIdentityStatus {
    FullValid,
    RuntimeMismatch,
    Missing,
}

impl Default for RuntimeIdentityStatus {
    fn default() -> Self {
        Self::Missing
    }
}

pub fn workspace_fingerprint(workspace: &Workspace) -> String {
    stable_hash(
        &serde_json::json!({
            "root": workspace.root.display().to_string(),
            "kind": workspace.kind,
        })
        .to_string(),
    )
}

pub fn build_runtime_identity(
    workspace: &Workspace,
    model_id: &str,
    provider_target: &str,
    approval_policy: ApprovalPolicy,
    max_steps: u32,
    plan_enabled: bool,
    system_prompt: &str,
    planner_prompt: &str,
    tools: &[ToolSchema],
) -> RuntimeIdentity {
    RuntimeIdentity {
        cwd: workspace.root.display().to_string(),
        workspace_kind: workspace.kind.clone(),
        model_id: model_id.to_string(),
        provider_target: provider_target.to_string(),
        approval_policy,
        max_steps,
        plan_enabled,
        system_prompt_hash: stable_hash(system_prompt),
        planner_prompt_hash: stable_hash(planner_prompt),
        workspace_fingerprint: workspace_fingerprint(workspace),
        tool_signature: tool_signature(tools),
    }
}

pub fn evaluate_runtime_identity(
    saved: Option<&RuntimeIdentity>,
    current: &RuntimeIdentity,
) -> RuntimeIdentityEvaluation {
    let Some(saved) = saved else {
        return RuntimeIdentityEvaluation {
            status: RuntimeIdentityStatus::Missing,
            mismatch_fields: Vec::new(),
        };
    };

    let mut mismatch_fields = Vec::new();
    if saved.cwd != current.cwd {
        mismatch_fields.push("cwd".to_string());
    }
    if saved.workspace_kind != current.workspace_kind {
        mismatch_fields.push("workspace_kind".to_string());
    }
    if saved.model_id != current.model_id {
        mismatch_fields.push("model_id".to_string());
    }
    if saved.provider_target != current.provider_target {
        mismatch_fields.push("provider_target".to_string());
    }
    if saved.approval_policy != current.approval_policy {
        mismatch_fields.push("approval_policy".to_string());
    }
    if saved.max_steps != current.max_steps {
        mismatch_fields.push("max_steps".to_string());
    }
    if saved.plan_enabled != current.plan_enabled {
        mismatch_fields.push("plan_enabled".to_string());
    }
    if saved.system_prompt_hash != current.system_prompt_hash {
        mismatch_fields.push("system_prompt_hash".to_string());
    }
    if saved.planner_prompt_hash != current.planner_prompt_hash {
        mismatch_fields.push("planner_prompt_hash".to_string());
    }
    if saved.workspace_fingerprint != current.workspace_fingerprint {
        mismatch_fields.push("workspace_fingerprint".to_string());
    }
    if saved.tool_signature != current.tool_signature {
        mismatch_fields.push("tool_signature".to_string());
    }

    RuntimeIdentityEvaluation {
        status: if mismatch_fields.is_empty() {
            RuntimeIdentityStatus::FullValid
        } else {
            RuntimeIdentityStatus::RuntimeMismatch
        },
        mismatch_fields,
    }
}

#[cfg(test)]
mod tests {
    use crate::core::prompt_metadata::tool_signature;
    use crate::core::types::{ApprovalPolicy, ToolSchema};
    use crate::core::workspace::{Workspace, WorkspaceKind};

    use super::{RuntimeIdentityStatus, build_runtime_identity, evaluate_runtime_identity};

    fn workspace() -> Workspace {
        let root = std::env::current_dir().unwrap();
        Workspace {
            root: root.clone(),
            kind: WorkspaceKind::Repo,
            state_dir: root.join(".rove"),
        }
    }

    fn tools() -> Vec<ToolSchema> {
        vec![ToolSchema {
            name: "fs_read".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            destructive: false,
            parallel_safe: true,
            capability: None,
        }]
    }

    #[test]
    fn build_runtime_identity_records_execution_contract() {
        let workspace = workspace();
        let tools = tools();

        let identity = build_runtime_identity(
            &workspace,
            "gpt-4.1-mini",
            "openai-responses:https://api.openai.com/v1:gpt-4.1-mini",
            ApprovalPolicy::Auto,
            12,
            true,
            "system prompt",
            "planner prompt",
            &tools,
        );

        assert_eq!(identity.cwd, workspace.root.display().to_string());
        assert_eq!(identity.workspace_kind, WorkspaceKind::Repo);
        assert_eq!(identity.model_id, "gpt-4.1-mini");
        assert_eq!(
            identity.provider_target,
            "openai-responses:https://api.openai.com/v1:gpt-4.1-mini"
        );
        assert_eq!(identity.approval_policy, ApprovalPolicy::Auto);
        assert_eq!(identity.max_steps, 12);
        assert!(identity.plan_enabled);
        assert!(identity.system_prompt_hash.starts_with("sha256:"));
        assert!(identity.planner_prompt_hash.starts_with("sha256:"));
        assert!(identity.workspace_fingerprint.starts_with("sha256:"));
        assert_eq!(identity.tool_signature, tool_signature(&tools));
    }

    #[test]
    fn evaluate_runtime_identity_reports_mismatch_fields() {
        let workspace = workspace();
        let tools = tools();
        let saved = build_runtime_identity(
            &workspace,
            "gpt-4.1-mini",
            "openai-responses:https://api.openai.com/v1:gpt-4.1-mini",
            ApprovalPolicy::Auto,
            12,
            true,
            "system prompt",
            "planner prompt",
            &tools,
        );
        let current = build_runtime_identity(
            &workspace,
            "gpt-4.1",
            "openai-compatible:https://api.openai.com/v1:gpt-4.1",
            ApprovalPolicy::Never,
            8,
            false,
            "changed system prompt",
            "planner prompt",
            &[],
        );

        let evaluation = evaluate_runtime_identity(Some(&saved), &current);

        assert_eq!(evaluation.status, RuntimeIdentityStatus::RuntimeMismatch);
        assert!(evaluation.mismatch_fields.contains(&"model_id".to_string()));
        assert!(
            evaluation
                .mismatch_fields
                .contains(&"provider_target".to_string())
        );
        assert!(
            evaluation
                .mismatch_fields
                .contains(&"approval_policy".to_string())
        );
        assert!(
            evaluation
                .mismatch_fields
                .contains(&"system_prompt_hash".to_string())
        );
        assert!(
            evaluation
                .mismatch_fields
                .contains(&"tool_signature".to_string())
        );
    }

    #[test]
    fn evaluate_runtime_identity_treats_missing_saved_identity_as_missing() {
        let workspace = workspace();
        let current = build_runtime_identity(
            &workspace,
            "fake",
            "fake:local:fake",
            ApprovalPolicy::Auto,
            20,
            false,
            "system",
            "planner",
            &[],
        );

        let evaluation = evaluate_runtime_identity(None, &current);

        assert_eq!(evaluation.status, RuntimeIdentityStatus::Missing);
        assert!(evaluation.mismatch_fields.is_empty());
    }
}
