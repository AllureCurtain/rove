use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Unique identity for one tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallId(pub Ulid);

impl CallId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for CallId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Action normalized from one completed model turn.
#[derive(Debug, Clone)]
pub enum Action {
    Final {
        text: String,
    },
    ToolCall {
        call_id: CallId,
        tool_use_id: Option<String>,
        name: String,
        args: serde_json::Value,
    },
    ToolBatch {
        calls: Vec<ToolCallAction>,
    },
    Malformed {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct ToolCallAction {
    pub call_id: CallId,
    pub tool_use_id: Option<String>,
    pub name: String,
    pub args: serde_json::Value,
}

/// Operational tool metadata owned by the Agent layer.
///
/// `model_schema` projects this descriptor into the provider-neutral wire
/// schema without leaking safety or scheduling fields to model adapters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub destructive: bool,
    pub parallel_safe: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<ToolCapability>,
}

impl ToolDescriptor {
    pub fn model_schema(&self) -> rove_models::ModelToolSchema {
        rove_models::ModelToolSchema {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCapability {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: CallId,
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<ToolMutation>,
    #[serde(default)]
    pub metadata: ToolExecutionMetadata,
    /// Rich result detail, when the tool produced one.
    ///
    /// Additive and skipped when absent, so an artifact written by an older
    /// build still deserializes and a consumer that only reads `output` and
    /// `metadata` is unaffected. Boxed to keep `ToolResult` small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Box<crate::tool_result::ToolOutputEnvelope>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolMutation {
    pub path: String,
    pub operation: ToolMutationOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolMutationOperation {
    Create,
    Update,
    Delete,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    #[default]
    Ok,
    Error,
    Rejected,
    PartialSuccess,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRiskLevel {
    #[default]
    Low,
    High,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionMetadata {
    pub status: ToolExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_event_type: Option<String>,
    pub risk_level: ToolRiskLevel,
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_paths: Vec<String>,
    pub workspace_changed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff_summary: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{ToolCapability, ToolDescriptor};

    #[test]
    fn model_schema_projection_excludes_operational_metadata() {
        let descriptor = ToolDescriptor {
            name: "write".to_string(),
            description: "Write a file".to_string(),
            parameters: serde_json::json!({"type":"object"}),
            destructive: true,
            parallel_safe: false,
            capability_id: Some("workspace.fs.write".to_string()),
            capability: Some(ToolCapability {
                status: "enabled".to_string(),
                feature: None,
                message: None,
            }),
        };

        let value = serde_json::to_value(descriptor.model_schema()).unwrap();

        assert_eq!(value["name"], "write");
        assert_eq!(value["parameters"]["type"], "object");
        assert!(value.get("destructive").is_none());
        assert!(value.get("parallel_safe").is_none());
        assert!(value.get("capability_id").is_none());
        assert!(value.get("capability").is_none());
    }
}
