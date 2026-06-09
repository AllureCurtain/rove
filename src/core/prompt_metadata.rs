use serde::{Deserialize, Serialize};

use crate::core::types::{Message, ToolSchema};
use crate::core::workspace::Workspace;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptBuildMetadata {
    pub prompt_hash: String,
    pub stable_prefix_hash: String,
    pub workspace_fingerprint: String,
    pub tool_signature: String,
    pub token_estimate: usize,
    pub included_history_messages: usize,
    pub dropped_history_messages: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

pub fn stable_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{digest:x}")
}

pub fn prompt_hash(messages: &[Message]) -> String {
    stable_hash(&serde_json::to_string(messages).unwrap_or_default())
}

pub fn tool_signature(tools: &[ToolSchema]) -> String {
    let mut sorted = tools.to_vec();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    stable_hash(&serde_json::to_string(&sorted).unwrap_or_default())
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

pub fn prompt_cache_key(stable_prefix_hash: &str, tool_signature: &str) -> String {
    stable_hash(&format!("{stable_prefix_hash}:{tool_signature}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::ToolSchema;

    #[test]
    fn stable_hash_is_deterministic_and_namespaced() {
        let first = stable_hash("same prompt prefix");
        let second = stable_hash("same prompt prefix");
        let different = stable_hash("different prompt prefix");

        assert_eq!(first, second);
        assert_ne!(first, different);
        assert!(first.starts_with("sha256:"));
    }

    #[test]
    fn tool_signature_is_stable_across_tool_order() {
        let read = ToolSchema {
            name: "fs_read".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
            destructive: false,
            parallel_safe: true,
            capability: None,
        };
        let write = ToolSchema {
            name: "fs_write".to_string(),
            description: "Write a file".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
            destructive: true,
            parallel_safe: false,
            capability: None,
        };

        assert_eq!(
            tool_signature(&[read.clone(), write.clone()]),
            tool_signature(&[write, read])
        );
    }

    #[test]
    fn prompt_cache_key_changes_with_tools() {
        let prefix = stable_hash("system");
        let no_tools = tool_signature(&[]);
        let with_tool = tool_signature(&[ToolSchema {
            name: "echo".to_string(),
            description: "Echo".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
            destructive: false,
            parallel_safe: true,
            capability: None,
        }]);

        assert_ne!(
            prompt_cache_key(&prefix, &no_tools),
            prompt_cache_key(&prefix, &with_tool)
        );
    }

    #[test]
    fn prompt_hash_changes_with_messages() {
        assert_ne!(
            prompt_hash(&[Message::user("a")]),
            prompt_hash(&[Message::user("b")])
        );
    }
}
