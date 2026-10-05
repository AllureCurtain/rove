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
    /// How many oversized tool payloads the compaction probe's request elided.
    ///
    /// Bounded pruning facts of one turn's shrink request. They are `None`-free
    /// and omitted while zero/empty, so a build that pruned nothing serializes
    /// exactly the bytes it serialized before this field existed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pruned_tool_results: usize,
    /// Serialized bytes the pruned messages occupied before the projection, so
    /// `pruned_payload_bytes - pruned_excerpt_bytes` is the whole loss: the
    /// elided payload text, the blocks that repeated it, and the framing.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pruned_payload_bytes: usize,
    /// Serialized bytes those messages occupy as the request carries them, so
    /// the elision is measurable rather than implied.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pruned_excerpt_bytes: usize,
    /// Older messages left out so the request itself stayed within its byte and
    /// message bounds. Zero when the whole dropped prefix fit.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pruned_omitted_messages: usize,
    /// Serialized bytes those messages occupied, and therefore the material the
    /// request did not carry. Recorded beside the count because "three messages"
    /// and "three kilobytes" are different answers to "how much was left out".
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pruned_omitted_bytes: usize,
    /// Bounded list (newest first, capped by the compaction policy) of
    /// `result id + content digest` for the elided payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned_payload_digests: Vec<String>,
    /// Which pruning rule produced the excerpt above, so a trace reader can tell
    /// two policies apart. Absent when nothing was pruned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruning_policy: Option<String>,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl PromptBuildMetadata {
    /// Record what one bounded compaction request elided.
    ///
    /// Called on the turn's `PromptBuilt` metadata after the probe ran, so the
    /// pruning fact travels on the canonical event that already says what the
    /// turn's context contained. No new event and no second persistence path:
    /// `trace.jsonl` records the fact, and the state snapshot is untouched
    /// because pruning never rewrites stored history.
    pub fn apply_pruning(&mut self, facts: &crate::compaction::PruningFacts) {
        if facts.is_empty() {
            return;
        }
        self.pruned_tool_results = facts.pruned_tool_results;
        self.pruned_payload_bytes = facts.pruned_payload_bytes;
        self.pruned_excerpt_bytes = facts.pruned_excerpt_bytes;
        self.pruned_omitted_messages = facts.omitted_older_messages;
        self.pruned_omitted_bytes = facts.omitted_older_bytes;
        self.pruned_payload_digests = facts.pruned_payload_digests.clone();
        self.pruning_policy = Some(crate::compaction::PRUNING_POLICY_VERSION.to_string());
    }
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

    /// Every recorded fact reaches the prompt-build metadata, counts and bytes
    /// alike: a fact the request measured but the event dropped would be a number
    /// no reader can see, and the downstream wiring would look for a key that
    /// never appears.
    #[test]
    fn apply_pruning_carries_every_recorded_fact() {
        let facts = crate::compaction::PruningFacts {
            pruned_tool_results: 2,
            pruned_payload_bytes: 40_000,
            pruned_excerpt_bytes: 8_000,
            omitted_older_messages: 3,
            omitted_older_bytes: 9_000,
            pruned_payload_digests: vec!["call-a:sha256:aa".to_string()],
        };

        let mut metadata = PromptBuildMetadata::default();
        metadata.apply_pruning(&facts);

        assert_eq!(metadata.pruned_tool_results, 2);
        assert_eq!(metadata.pruned_payload_bytes, 40_000);
        assert_eq!(metadata.pruned_excerpt_bytes, 8_000);
        assert_eq!(metadata.pruned_omitted_messages, 3);
        assert_eq!(
            metadata.pruned_omitted_bytes, 9_000,
            "the omitted byte total is a fact of the build, not only its count"
        );
        assert_eq!(
            metadata.pruned_payload_digests,
            facts.pruned_payload_digests
        );
        assert_eq!(
            metadata.pruning_policy.as_deref(),
            Some(crate::compaction::PRUNING_POLICY_VERSION)
        );
    }

    /// A build that pruned nothing stays byte-identical, so an existing trace and
    /// the Web parser see exactly what they saw before this field existed.
    #[test]
    fn apply_pruning_leaves_an_untouched_build_alone() {
        let mut metadata = PromptBuildMetadata::default();
        metadata.apply_pruning(&crate::compaction::PruningFacts::default());

        assert_eq!(metadata.pruned_tool_results, 0);
        assert_eq!(metadata.pruned_omitted_bytes, 0);
        assert!(metadata.pruning_policy.is_none());
    }

    #[test]
    fn prompt_hash_changes_with_messages() {
        assert_ne!(
            prompt_hash(&[Message::user("a")]),
            prompt_hash(&[Message::user("b")])
        );
    }
}
