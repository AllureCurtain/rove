use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::environment::EnvironmentError;
use crate::tools::coding::map_environment_error;
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_REPOSITORY_MAP_BYTES: usize = 32 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_MEMBERS: usize = 256;

#[derive(Default)]
pub struct RepositoryMapTool;

#[derive(Serialize)]
struct RepositoryMap {
    source: &'static str,
    manifests: Vec<String>,
    members: Vec<String>,
    digest: String,
    output_bytes: usize,
    truncated: bool,
}

#[async_trait]
impl Tool for RepositoryMapTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "repository_map".to_string(),
            description: "Return a bounded deterministic repository member map derived only from verified Cargo.toml/package.json workspace manifests.".to_string(),
            parameters: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("workspace.repository.map".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, _args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let services = runtime_tool_services(ctx)?;
        let filesystem = services.environment.filesystem();
        let mut manifests = Vec::new();
        let mut members = Vec::new();
        let mut identity = String::new();
        let mut truncated = false;

        match filesystem
            .read_relative_bytes("Cargo.toml", MAX_MANIFEST_BYTES)
            .await
        {
            Ok(read) if !read.truncated => {
                let text =
                    std::str::from_utf8(&read.bytes).map_err(|_| ToolError::InvalidInput {
                        reason: "Cargo.toml is not UTF-8".to_string(),
                    })?;
                let value: toml::Value =
                    toml::from_str(text).map_err(|error| ToolError::InvalidInput {
                        reason: format!("Cargo.toml is invalid: {error}"),
                    })?;
                if let Some(values) = value
                    .get("workspace")
                    .and_then(|value| value.get("members"))
                    .and_then(toml::Value::as_array)
                {
                    manifests.push("Cargo.toml".to_string());
                    let declared = values.iter().filter_map(toml::Value::as_str);
                    for member in declared.clone().take(MAX_MEMBERS) {
                        members.push(member.replace('\\', "/"));
                    }
                    truncated |= declared.count() > MAX_MEMBERS;
                    identity.push_str("Cargo.toml\0");
                    identity.push_str(text);
                    identity.push('\0');
                }
            }
            Ok(_) => {
                return Err(ToolError::InvalidInput {
                    reason: "Cargo.toml exceeds the manifest byte limit".to_string(),
                });
            }
            Err(EnvironmentError::NotFound) => {}
            Err(error) => return Err(map_environment_error(error)),
        }
        match filesystem
            .read_relative_bytes("package.json", MAX_MANIFEST_BYTES)
            .await
        {
            Ok(read) if !read.truncated => {
                let value: serde_json::Value =
                    serde_json::from_slice(&read.bytes).map_err(|error| {
                        ToolError::InvalidInput {
                            reason: format!("package.json is invalid: {error}"),
                        }
                    })?;
                let values = value.get("workspaces").and_then(|workspaces| {
                    workspaces
                        .as_array()
                        .or_else(|| workspaces.get("packages").and_then(Value::as_array))
                });
                if let Some(values) = values {
                    manifests.push("package.json".to_string());
                    let declared = values.iter().filter_map(Value::as_str);
                    members.extend(
                        declared
                            .clone()
                            .take(MAX_MEMBERS)
                            .map(|member| member.replace('\\', "/")),
                    );
                    truncated |= declared.count() > MAX_MEMBERS;
                    identity.push_str("package.json\0");
                    identity.push_str(&String::from_utf8_lossy(&read.bytes));
                    identity.push('\0');
                }
            }
            Ok(_) => {
                return Err(ToolError::InvalidInput {
                    reason: "package.json exceeds the manifest byte limit".to_string(),
                });
            }
            Err(EnvironmentError::NotFound) => {}
            Err(error) => return Err(map_environment_error(error)),
        }
        members.sort();
        members.dedup();
        let digest = crate::context::prompt_metadata::stable_hash(&identity);
        let mut result = RepositoryMap {
            source: "verified_manifests",
            manifests,
            members,
            digest,
            output_bytes: 0,
            truncated,
        };
        let mut encoded =
            serde_json::to_string(&result).map_err(|error| ToolError::ExecutionFailed {
                reason: error.to_string(),
            })?;
        while encoded.len() > MAX_REPOSITORY_MAP_BYTES && !result.members.is_empty() {
            result.members.pop();
            result.truncated = true;
            encoded =
                serde_json::to_string(&result).map_err(|error| ToolError::ExecutionFailed {
                    reason: error.to_string(),
                })?;
        }
        for _ in 0..4 {
            if result.output_bytes == encoded.len() {
                break;
            }
            result.output_bytes = encoded.len();
            encoded =
                serde_json::to_string(&result).map_err(|error| ToolError::ExecutionFailed {
                    reason: error.to_string(),
                })?;
        }
        Ok(ToolOutput::text(encoded))
    }
}
