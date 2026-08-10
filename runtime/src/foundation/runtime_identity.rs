use serde::{Deserialize, Serialize};

use crate::environment::{ExecutionCapabilities, ExecutionEnvironmentIdentity};
use crate::execution::ExecutionPolicy;
use crate::prompt_metadata::{stable_hash, tool_signature};
use crate::types::ApprovalPolicy;
use crate::workspace::{Workspace, WorkspaceKind};
use rove_core::ToolDescriptor;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_prompt_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalizer_prompt_hash: Option<String>,
    pub workspace_fingerprint: String,
    pub tool_signature: String,
    /// Fully resolved execution policy. Absent in older snapshots, which fall
    /// back to the `max_steps` / `plan_enabled` projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_policy: Option<ExecutionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_environment: Option<ExecutionEnvironmentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_capabilities: Option<ExecutionCapabilities>,
    /// Content-free identity of the immutable Agent snapshot used by the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<crate::agents::AgentProfileIdentity>,
}

impl RuntimeIdentity {
    /// Project sugar fields into the typed policy without changing wire schema.
    pub fn to_execution_policy(&self) -> ExecutionPolicy {
        self.execution_policy.clone().unwrap_or_else(|| {
            ExecutionPolicy::from_max_steps_and_plan_flag(self.max_steps, self.plan_enabled)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentityEvaluation {
    pub status: RuntimeIdentityStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mismatch_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeIdentityStatus {
    FullValid,
    RuntimeMismatch,
    #[default]
    Missing,
}

pub struct RuntimeIdentityInput<'a> {
    pub workspace: &'a Workspace,
    pub model_id: &'a str,
    pub provider_target: &'a str,
    pub approval_policy: ApprovalPolicy,
    pub max_steps: u32,
    pub plan_enabled: bool,
    pub system_prompt: &'a str,
    pub planner_prompt: &'a str,
    /// Bounded evaluator prompt. Recorded so a resumed run can detect that the
    /// text driving ambiguity decisions changed underneath it.
    pub evaluator_prompt: &'a str,
    /// Independent finalizer prompt, recorded for the same reason.
    pub finalizer_prompt: &'a str,
    /// Fully resolved execution policy for this run.
    pub execution_policy: ExecutionPolicy,
    pub tools: &'a [ToolDescriptor],
    pub capability_snapshot_id: Option<&'a str>,
    pub execution_environment: Option<&'a ExecutionEnvironmentIdentity>,
    pub execution_capabilities: Option<&'a ExecutionCapabilities>,
    pub agent: Option<&'a crate::agents::AgentProfileIdentity>,
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

pub fn build_runtime_identity(input: RuntimeIdentityInput<'_>) -> RuntimeIdentity {
    RuntimeIdentity {
        cwd: input.workspace.root.display().to_string(),
        workspace_kind: input.workspace.kind.clone(),
        model_id: input.model_id.to_string(),
        provider_target: input.provider_target.to_string(),
        approval_policy: input.approval_policy,
        max_steps: input.max_steps,
        plan_enabled: input.plan_enabled,
        system_prompt_hash: stable_hash(input.system_prompt),
        planner_prompt_hash: stable_hash(input.planner_prompt),
        evaluator_prompt_hash: Some(stable_hash(input.evaluator_prompt)),
        finalizer_prompt_hash: Some(stable_hash(input.finalizer_prompt)),
        workspace_fingerprint: workspace_fingerprint(input.workspace),
        tool_signature: tool_signature(input.tools),
        execution_policy: Some(input.execution_policy),
        capability_snapshot_id: input.capability_snapshot_id.map(str::to_string),
        execution_environment: input.execution_environment.cloned(),
        execution_capabilities: input.execution_capabilities.copied(),
        agent: input.agent.cloned(),
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
    if saved.evaluator_prompt_hash.is_some()
        && saved.evaluator_prompt_hash != current.evaluator_prompt_hash
    {
        mismatch_fields.push("evaluator_prompt_hash".to_string());
    }
    if saved.finalizer_prompt_hash.is_some()
        && saved.finalizer_prompt_hash != current.finalizer_prompt_hash
    {
        mismatch_fields.push("finalizer_prompt_hash".to_string());
    }
    if saved.workspace_fingerprint != current.workspace_fingerprint {
        mismatch_fields.push("workspace_fingerprint".to_string());
    }
    if saved.tool_signature != current.tool_signature {
        mismatch_fields.push("tool_signature".to_string());
    }
    if saved.execution_policy.is_some() && saved.execution_policy != current.execution_policy {
        mismatch_fields.push("execution_policy".to_string());
    }
    if saved.capability_snapshot_id.is_some()
        && saved.capability_snapshot_id != current.capability_snapshot_id
    {
        mismatch_fields.push("capability_snapshot_id".to_string());
    }
    // Missing fields identify pre-environment artifacts and remain compatible.
    // Once persisted, the adapter identity and capability set are part of the
    // resume contract and must match exactly.
    if saved.execution_environment.is_some()
        && saved.execution_environment != current.execution_environment
    {
        mismatch_fields.push("execution_environment".to_string());
    }
    if saved.execution_capabilities.is_some()
        && saved.execution_capabilities != current.execution_capabilities
    {
        mismatch_fields.push("execution_capabilities".to_string());
    }
    if saved.agent.is_some() && saved.agent != current.agent {
        mismatch_fields.push("agent".to_string());
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
    use crate::execution::ExecutionPolicy;
    use crate::prompt_metadata::tool_signature;
    use crate::types::ApprovalPolicy;
    use crate::workspace::{Workspace, WorkspaceKind};
    use rove_core::ToolDescriptor;

    use super::{
        RuntimeIdentity, RuntimeIdentityInput, RuntimeIdentityStatus, build_runtime_identity,
        evaluate_runtime_identity, workspace_fingerprint,
    };

    fn workspace() -> Workspace {
        let root = std::env::current_dir().unwrap();
        Workspace {
            root: root.clone(),
            kind: WorkspaceKind::Repo,
            state_dir: root.join(".rove"),
        }
    }

    fn tools() -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            destructive: false,
            parallel_safe: true,
            capability_id: None,
            capability: None,
        }]
    }

    #[test]
    fn build_runtime_identity_records_execution_contract() {
        let workspace = workspace();
        let tools = tools();

        let identity = build_runtime_identity(RuntimeIdentityInput {
            workspace: &workspace,
            model_id: "gpt-4.1-mini",
            provider_target: "openai-responses:https://api.openai.com/v1:gpt-4.1-mini",
            approval_policy: ApprovalPolicy::Auto,
            max_steps: 12,
            plan_enabled: true,
            system_prompt: "system prompt",
            planner_prompt: "planner prompt",
            evaluator_prompt: "evaluator prompt",
            finalizer_prompt: "finalizer prompt",
            execution_policy: ExecutionPolicy::from_max_steps_and_plan_flag(12, true),
            tools: &tools,
            capability_snapshot_id: Some("sha256:capabilities"),
            execution_environment: None,
            execution_capabilities: None,
            agent: None,
        });

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
        assert_eq!(
            identity.to_execution_policy().strategy,
            crate::execution::ExecutionStrategy::PlanReact
        );
        assert_eq!(
            identity.to_execution_policy().budgets.max_step_attempts,
            Some(12)
        );
        assert!(identity.system_prompt_hash.starts_with("sha256:"));
        assert!(identity.planner_prompt_hash.starts_with("sha256:"));
        assert!(identity.workspace_fingerprint.starts_with("sha256:"));
        assert_eq!(identity.tool_signature, tool_signature(&tools));
        assert_eq!(
            identity.capability_snapshot_id.as_deref(),
            Some("sha256:capabilities")
        );
    }

    #[test]
    fn evaluate_runtime_identity_reports_mismatch_fields() {
        let workspace = workspace();
        let tools = tools();
        let saved = build_runtime_identity(RuntimeIdentityInput {
            workspace: &workspace,
            model_id: "gpt-4.1-mini",
            provider_target: "openai-responses:https://api.openai.com/v1:gpt-4.1-mini",
            approval_policy: ApprovalPolicy::Auto,
            max_steps: 12,
            plan_enabled: true,
            system_prompt: "system prompt",
            planner_prompt: "planner prompt",
            evaluator_prompt: "evaluator prompt",
            finalizer_prompt: "finalizer prompt",
            execution_policy: ExecutionPolicy::from_max_steps_and_plan_flag(12, true),
            tools: &tools,
            capability_snapshot_id: Some("sha256:saved-capabilities"),
            execution_environment: None,
            execution_capabilities: None,
            agent: None,
        });
        let current = build_runtime_identity(RuntimeIdentityInput {
            workspace: &workspace,
            model_id: "gpt-4.1",
            provider_target: "openai:https://api.openai.com/v1:gpt-4.1",
            approval_policy: ApprovalPolicy::Never,
            max_steps: 8,
            plan_enabled: false,
            system_prompt: "changed system prompt",
            planner_prompt: "planner prompt",
            evaluator_prompt: "evaluator prompt",
            finalizer_prompt: "finalizer prompt",
            execution_policy: ExecutionPolicy::from_max_steps_and_plan_flag(12, true),
            tools: &[],
            capability_snapshot_id: Some("sha256:current-capabilities"),
            execution_environment: None,
            execution_capabilities: None,
            agent: None,
        });

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
        assert!(
            evaluation
                .mismatch_fields
                .contains(&"capability_snapshot_id".to_string())
        );
    }

    #[test]
    fn evaluate_runtime_identity_treats_missing_saved_identity_as_missing() {
        let workspace = workspace();
        let current = build_runtime_identity(RuntimeIdentityInput {
            workspace: &workspace,
            model_id: "fake",
            provider_target: "fake:local:fake",
            approval_policy: ApprovalPolicy::Auto,
            max_steps: 20,
            plan_enabled: false,
            system_prompt: "system",
            planner_prompt: "planner",
            evaluator_prompt: "evaluator",
            finalizer_prompt: "finalizer",
            execution_policy: ExecutionPolicy::from_max_steps_and_plan_flag(4, true),
            tools: &[],
            capability_snapshot_id: None,
            execution_environment: None,
            execution_capabilities: None,
            agent: None,
        });

        let evaluation = evaluate_runtime_identity(None, &current);

        assert_eq!(evaluation.status, RuntimeIdentityStatus::Missing);
        assert!(evaluation.mismatch_fields.is_empty());
    }

    #[test]
    fn legacy_identity_without_environment_fields_remains_compatible() {
        let workspace = workspace();
        let current = build_runtime_identity(RuntimeIdentityInput {
            workspace: &workspace,
            model_id: "fake",
            provider_target: "fake:local:fake",
            approval_policy: ApprovalPolicy::Auto,
            max_steps: 20,
            plan_enabled: false,
            system_prompt: "system",
            planner_prompt: "planner",
            evaluator_prompt: "evaluator",
            finalizer_prompt: "finalizer",
            execution_policy: ExecutionPolicy::from_max_steps_and_plan_flag(4, true),
            tools: &[],
            capability_snapshot_id: Some("sha256:current-capabilities"),
            execution_environment: Some(&crate::environment::ExecutionEnvironmentIdentity {
                adapter: "local".to_string(),
                workspace_kind: workspace.kind.clone(),
                workspace_digest: workspace_fingerprint(&workspace),
            }),
            execution_capabilities: Some(&crate::environment::ExecutionCapabilities {
                filesystem_read: true,
                filesystem_write: true,
                process_run: true,
                process_stdio: true,
                observations: true,
                process_background: true,
                process_pty: false,
                workspace_checkpoints: true,
                artifact_projection: true,
            }),
            agent: None,
        });
        let mut legacy_value = serde_json::to_value(&current).unwrap();
        legacy_value
            .as_object_mut()
            .unwrap()
            .remove("execution_environment");
        legacy_value
            .as_object_mut()
            .unwrap()
            .remove("execution_capabilities");
        let legacy: RuntimeIdentity = serde_json::from_value(legacy_value).unwrap();

        let evaluation = evaluate_runtime_identity(Some(&legacy), &current);

        assert_eq!(evaluation.status, RuntimeIdentityStatus::FullValid);
        assert!(evaluation.mismatch_fields.is_empty());
    }

    fn lifecycle_identity(
        evaluator_prompt: &str,
        finalizer_prompt: &str,
        policy: ExecutionPolicy,
        workspace: &Workspace,
    ) -> RuntimeIdentity {
        build_runtime_identity(RuntimeIdentityInput {
            workspace,
            model_id: "fake",
            provider_target: "fake:local:fake",
            approval_policy: ApprovalPolicy::Auto,
            max_steps: 20,
            plan_enabled: true,
            system_prompt: "system",
            planner_prompt: "planner",
            evaluator_prompt,
            finalizer_prompt,
            execution_policy: policy,
            tools: &[],
            capability_snapshot_id: None,
            execution_environment: None,
            execution_capabilities: None,
            agent: None,
        })
    }

    /// A resumed run must notice that the text or policy driving lifecycle
    /// decisions changed underneath it, otherwise the recorded identity is
    /// decorative.
    #[test]
    fn changed_lifecycle_prompts_and_policy_are_reported_as_mismatches() {
        let workspace = workspace();
        let baseline_policy = ExecutionPolicy::from_max_steps_and_plan_flag(20, true);
        let saved = lifecycle_identity(
            "evaluator v1",
            "finalizer v1",
            baseline_policy.clone(),
            &workspace,
        );

        let changed_evaluator = lifecycle_identity(
            "evaluator v2",
            "finalizer v1",
            baseline_policy.clone(),
            &workspace,
        );
        assert!(
            evaluate_runtime_identity(Some(&saved), &changed_evaluator)
                .mismatch_fields
                .contains(&"evaluator_prompt_hash".to_string())
        );

        let changed_finalizer = lifecycle_identity(
            "evaluator v1",
            "finalizer v2",
            baseline_policy.clone(),
            &workspace,
        );
        assert!(
            evaluate_runtime_identity(Some(&saved), &changed_finalizer)
                .mismatch_fields
                .contains(&"finalizer_prompt_hash".to_string())
        );

        let mut tightened = baseline_policy.clone();
        tightened.budgets.max_tool_calls = Some(3);
        let changed_policy =
            lifecycle_identity("evaluator v1", "finalizer v1", tightened, &workspace);
        assert!(
            evaluate_runtime_identity(Some(&saved), &changed_policy)
                .mismatch_fields
                .contains(&"execution_policy".to_string())
        );

        // An identical lifecycle contract must not be reported as drift.
        let same = lifecycle_identity("evaluator v1", "finalizer v1", baseline_policy, &workspace);
        assert!(
            evaluate_runtime_identity(Some(&saved), &same)
                .mismatch_fields
                .is_empty()
        );
    }

    /// Older snapshots predate these fields entirely and must stay resumable.
    #[test]
    fn an_identity_without_lifecycle_fields_is_not_treated_as_drift() {
        let workspace = workspace();
        let current = lifecycle_identity(
            "evaluator v1",
            "finalizer v1",
            ExecutionPolicy::from_max_steps_and_plan_flag(20, true),
            &workspace,
        );
        let mut legacy_value = serde_json::to_value(&current).unwrap();
        let object = legacy_value.as_object_mut().unwrap();
        object.remove("evaluator_prompt_hash");
        object.remove("finalizer_prompt_hash");
        object.remove("execution_policy");
        let legacy: RuntimeIdentity = serde_json::from_value(legacy_value).unwrap();

        let evaluation = evaluate_runtime_identity(Some(&legacy), &current);

        assert!(
            evaluation.mismatch_fields.is_empty(),
            "absent lifecycle identity is unknown, not changed: {:?}",
            evaluation.mismatch_fields
        );
        // The projection still resolves a usable policy for an old snapshot.
        assert_eq!(
            legacy.to_execution_policy(),
            ExecutionPolicy::from_max_steps_and_plan_flag(20, true)
        );
    }
}
