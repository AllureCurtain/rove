use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;
use rove_core::ToolDescriptor;
use rove_models::Message;

const MESSAGE_OVERHEAD_TOKENS: usize = 4;
const CHARS_PER_TOKEN: usize = 4;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptBuildMetadata {
    pub prompt_hash: String,
    pub stable_prefix_hash: String,
    pub workspace_fingerprint: String,
    pub tool_signature: String,
    pub token_estimate: usize,
    pub included_history_messages: usize,
    pub dropped_history_messages: usize,
    #[serde(default)]
    pub system_prompt_bytes: usize,
    #[serde(default)]
    pub stable_prefix_bytes: usize,
    #[serde(default)]
    pub history_bytes: usize,
    #[serde(default)]
    pub total_bytes: usize,
    #[serde(default)]
    pub referenced_tool_results: usize,
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

pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn estimate_message_tokens(message: &Message) -> usize {
    let tool_call_tokens: usize = message
        .tool_calls
        .iter()
        .map(|tool_call| {
            estimate_text_tokens(&tool_call.id)
                + estimate_text_tokens(&tool_call.name)
                + estimate_text_tokens(&tool_call.args.to_string())
        })
        .sum();
    let tool_call_id_tokens = message
        .tool_call_id
        .as_deref()
        .map(estimate_text_tokens)
        .unwrap_or(0);

    MESSAGE_OVERHEAD_TOKENS
        + estimate_text_tokens(&message.content)
        + tool_call_tokens
        + tool_call_id_tokens
}

pub fn message_bytes(message: &Message) -> usize {
    serde_json::to_vec(message)
        .map(|bytes| bytes.len())
        .unwrap_or_default()
}

fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN).max(1)
}

pub fn tool_signature(tools: &[ToolDescriptor]) -> String {
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
        let read = ToolDescriptor {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
            destructive: false,
            parallel_safe: true,
            capability_id: None,
            capability: None,
        };
        let write = ToolDescriptor {
            name: "write_file".to_string(),
            description: "Write a file".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
            destructive: true,
            parallel_safe: false,
            capability_id: None,
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
        let with_tool = tool_signature(&[ToolDescriptor {
            name: "echo".to_string(),
            description: "Echo".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
            destructive: false,
            parallel_safe: true,
            capability_id: None,
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
