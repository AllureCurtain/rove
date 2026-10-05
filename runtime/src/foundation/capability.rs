use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::prompt_metadata::{stable_hash, tool_signature};
use rove_core::{ToolDescriptor, ToolRegistry};

pub const CAPABILITY_SNAPSHOT_SCHEMA_VERSION: u16 = 1;
pub const MAX_PLANNER_CAPABILITY_SUMMARY_BYTES: usize = 24 * 1024;
const MAX_PLANNER_DESCRIPTION_CHARS: usize = 256;
const MAX_PLANNER_SCHEMA_PROPERTIES: usize = 32;
const MAX_PLANNER_ENUM_VALUES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilitySnapshot {
    pub schema_version: u16,
    pub snapshot_id: String,
    pub tool_signature: String,
    pub captured_at: String,
    pub tools: Vec<CapabilityTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    pub description: String,
    pub input_schema: Value,
    pub source: CapabilitySource,
    pub availability: String,
    pub mutation_class: CapabilityMutationClass,
    pub approval_required: bool,
    pub parallel_safe: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    Builtin,
    Mcp,
    Extension,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMutationClass {
    ReadOnly,
    Mutating,
}

impl CapabilitySnapshot {
    pub fn from_registry(registry: &ToolRegistry) -> Self {
        Self::from_descriptors(&registry.descriptors())
    }

    pub fn from_descriptors(descriptors: &[ToolDescriptor]) -> Self {
        let mut descriptors = descriptors.to_vec();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        let tools = descriptors
            .iter()
            .map(|descriptor| CapabilityTool {
                name: descriptor.name.clone(),
                capability_id: descriptor.capability_id.clone(),
                description: descriptor.description.clone(),
                input_schema: descriptor.parameters.clone(),
                source: capability_source(descriptor),
                availability: descriptor
                    .capability
                    .as_ref()
                    .map(|capability| capability.status.clone())
                    .unwrap_or_else(|| "available".to_string()),
                mutation_class: if descriptor.destructive {
                    CapabilityMutationClass::Mutating
                } else {
                    CapabilityMutationClass::ReadOnly
                },
                approval_required: descriptor.destructive,
                parallel_safe: descriptor.parallel_safe,
            })
            .collect::<Vec<_>>();
        let tool_signature = tool_signature(&descriptors);
        let snapshot_content =
            serde_json::to_string(&(CAPABILITY_SNAPSHOT_SCHEMA_VERSION, &tool_signature, &tools))
                .unwrap_or_default();
        let snapshot_id = stable_hash(&format!("capability-snapshot:{snapshot_content}"));
        Self {
            schema_version: CAPABILITY_SNAPSHOT_SCHEMA_VERSION,
            snapshot_id,
            tool_signature,
            captured_at: chrono::Utc::now().to_rfc3339(),
            tools,
        }
    }

    pub fn planner_summary(&self) -> String {
        #[derive(Serialize)]
        struct Summary<'a> {
            snapshot_id: &'a str,
            tool_signature: &'a str,
            total_tools: usize,
            included_tools: usize,
            omitted_tools: usize,
            tools: &'a [PlannerTool],
        }

        let candidates = self.tools.iter().map(PlannerTool::from).collect::<Vec<_>>();
        let mut included = Vec::new();
        for candidate in candidates {
            included.push(candidate);
            let summary = Summary {
                snapshot_id: &self.snapshot_id,
                tool_signature: &self.tool_signature,
                total_tools: self.tools.len(),
                included_tools: included.len(),
                omitted_tools: self.tools.len().saturating_sub(included.len()),
                tools: &included,
            };
            if serde_json::to_vec(&summary)
                .is_ok_and(|encoded| encoded.len() > MAX_PLANNER_CAPABILITY_SUMMARY_BYTES)
            {
                included.pop();
                break;
            }
        }
        serde_json::to_string(&Summary {
            snapshot_id: &self.snapshot_id,
            tool_signature: &self.tool_signature,
            total_tools: self.tools.len(),
            included_tools: included.len(),
            omitted_tools: self.tools.len().saturating_sub(included.len()),
            tools: &included,
        })
        .unwrap_or_else(|_| "{\"tools\":[]}".to_string())
    }
}

#[derive(Serialize)]
struct PlannerTool {
    name: String,
    capability_id: Option<String>,
    description: String,
    source: CapabilitySource,
    availability: String,
    mutation_class: CapabilityMutationClass,
    approval_required: bool,
    parallel_safe: bool,
    input_schema: PlannerInputSchema,
}

impl From<&CapabilityTool> for PlannerTool {
    fn from(tool: &CapabilityTool) -> Self {
        Self {
            name: tool.name.clone(),
            capability_id: tool.capability_id.clone(),
            description: truncate(&tool.description, MAX_PLANNER_DESCRIPTION_CHARS),
            source: tool.source,
            availability: tool.availability.clone(),
            mutation_class: tool.mutation_class,
            approval_required: tool.approval_required,
            parallel_safe: tool.parallel_safe,
            input_schema: PlannerInputSchema::from(&tool.input_schema),
        }
    }
}

#[derive(Serialize)]
struct PlannerInputSchema {
    schema_type: String,
    additional_properties: Option<bool>,
    properties: Vec<PlannerProperty>,
    omitted_properties: usize,
}

impl From<&Value> for PlannerInputSchema {
    fn from(schema: &Value) -> Self {
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let all_properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        let properties = all_properties
            .iter()
            .take(MAX_PLANNER_SCHEMA_PROPERTIES)
            .map(|(name, schema)| PlannerProperty {
                name: (*name).clone(),
                schema_type: schema
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                required: required.contains(name.as_str()),
                enum_values: schema
                    .get("enum")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(MAX_PLANNER_ENUM_VALUES)
                    .cloned()
                    .collect(),
            })
            .collect();
        Self {
            schema_type: schema
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            additional_properties: schema.get("additionalProperties").and_then(Value::as_bool),
            properties,
            omitted_properties: all_properties
                .len()
                .saturating_sub(MAX_PLANNER_SCHEMA_PROPERTIES),
        }
    }
}

#[derive(Serialize)]
struct PlannerProperty {
    name: String,
    schema_type: String,
    required: bool,
    enum_values: Vec<Value>,
}

fn capability_source(descriptor: &ToolDescriptor) -> CapabilitySource {
    match descriptor.capability_id.as_deref() {
        Some(id) if id.starts_with("mcp.") => CapabilitySource::Mcp,
        Some(id)
            if id.starts_with("workspace.")
                || id.starts_with("memory.")
                || id.starts_with("interaction.")
                || id.starts_with("execution.")
                || id.starts_with("testing.") =>
        {
            CapabilitySource::Builtin
        }
        Some(_) | None => CapabilitySource::Extension,
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(name: &str, capability_id: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: "Inspect safely".repeat(100),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "mode": {"type": "string", "enum": ["fast", "full"]}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: true,
            capability_id: Some(capability_id.to_string()),
            capability: None,
        }
    }

    #[test]
    fn snapshot_identity_is_stable_across_descriptor_order() {
        let alpha = descriptor("alpha", "workspace.alpha.read");
        let zeta = descriptor("zeta", "workspace.zeta.read");
        let first = CapabilitySnapshot::from_descriptors(&[alpha.clone(), zeta.clone()]);
        let second = CapabilitySnapshot::from_descriptors(&[zeta, alpha]);

        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(first.tool_signature, second.tool_signature);
        assert_eq!(first.tools[0].name, "alpha");
        assert_ne!(first.captured_at, "");
    }

    #[test]
    fn planner_summary_is_bounded_and_contains_safe_operational_metadata() {
        let descriptors = (0..128)
            .map(|index| descriptor(&format!("tool_{index}"), &format!("test.tool-{index}.read")))
            .collect::<Vec<_>>();
        let snapshot = CapabilitySnapshot::from_descriptors(&descriptors);
        let summary = snapshot.planner_summary();
        let value: Value = serde_json::from_str(&summary).unwrap();

        assert!(summary.len() <= MAX_PLANNER_CAPABILITY_SUMMARY_BYTES);
        assert_eq!(value["snapshot_id"], snapshot.snapshot_id);
        assert!(value["included_tools"].as_u64().unwrap() > 0);
        assert_eq!(
            value["total_tools"].as_u64().unwrap(),
            descriptors.len() as u64
        );
        assert!(summary.contains("approval_required"));
        assert!(!summary.contains("transport"));
    }
}
