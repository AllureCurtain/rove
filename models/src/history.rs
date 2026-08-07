use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    ContentBlock, InternalCallId, MAX_TOOL_ID_BYTES, Message, ModelMessage, Role, ToolCall,
    ToolCallRef, ToolResult, ToolResultStatus, WireCallReference,
};

/// Bounded policy for converting canonical history into a target request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryProjectionPolicy {
    /// Accept a legacy tool result with no native ID by assigning a stable
    /// compatibility identity.  This is enabled only for old artifacts.
    pub allow_legacy_tool_results: bool,
    /// Close an unresolved call with an explicit unknown-effect result.  New
    /// canonical sessions keep this disabled so malformed history fails closed.
    pub synthesize_missing_results: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionDiagnostic {
    LegacyToolResultId { index: usize, id: String },
    SynthesizedUnknownResult { id: String },
    RichContentDowngraded { index: usize },
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum HistoryProjectionError {
    #[error("message {index} is invalid: {reason}")]
    InvalidMessage { index: usize, reason: String },
    #[error("tool result at message {index} has no matching call `{id}`")]
    OrphanResult { index: usize, id: String },
    #[error("tool result at message {index} duplicates call `{id}`")]
    DuplicateResult { index: usize, id: String },
    #[error("tool result at message {index} names `{actual}` but call `{expected}` was requested")]
    ResultNameMismatch {
        index: usize,
        expected: String,
        actual: String,
    },
    #[error("tool call `{id}` has no result before history ended")]
    MissingResult { id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedHistory {
    pub messages: Vec<Message>,
    pub wire_ids: BTreeMap<InternalCallId, String>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Target-aware, provider-neutral history projection.
#[derive(Debug, Clone)]
pub struct HistoryProjector {
    target_protocol: String,
    policy: HistoryProjectionPolicy,
    preserve_source_wire_ids: bool,
}

impl HistoryProjector {
    pub fn new(protocol: impl Into<String>) -> Self {
        Self {
            target_protocol: protocol.into(),
            policy: HistoryProjectionPolicy::default(),
            preserve_source_wire_ids: false,
        }
    }

    /// Build the derived legacy-artifact projection. This is never used for a
    /// provider request: it retains the source wire ID only so older readers
    /// observe the same `Message` shape while canonical identity remains in
    /// the typed session.
    pub fn compatibility_artifact() -> Self {
        Self {
            target_protocol: "compatibility-artifact".to_string(),
            policy: HistoryProjectionPolicy::default(),
            preserve_source_wire_ids: true,
        }
    }

    pub fn with_policy(mut self, policy: HistoryProjectionPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn project(
        &self,
        source: &[ModelMessage],
    ) -> Result<ProjectedHistory, HistoryProjectionError> {
        let mut output = Vec::with_capacity(source.len());
        let mut aliases = AliasMap::new(&self.target_protocol, self.preserve_source_wire_ids);
        let mut pending = BTreeMap::<InternalCallId, String>::new();
        let mut completed = BTreeSet::<InternalCallId>::new();
        let mut diagnostics = Vec::new();

        for (index, message) in source.iter().enumerate() {
            message
                .validate()
                .map_err(|error| HistoryProjectionError::InvalidMessage {
                    index,
                    reason: error.to_string(),
                })?;

            if !message.tool_calls.is_empty() {
                let mut refs = Vec::with_capacity(message.tool_calls.len());
                for call in &message.tool_calls {
                    if pending.contains_key(&call.internal_call_id)
                        || completed.contains(&call.internal_call_id)
                    {
                        return Err(HistoryProjectionError::InvalidMessage {
                            index,
                            reason: format!(
                                "duplicate canonical tool call id `{}`",
                                call.internal_call_id
                            ),
                        });
                    }
                    let wire_id = aliases.alias(call)?;
                    pending.insert(call.internal_call_id.clone(), call.name.clone());
                    refs.push(ToolCallRef {
                        id: wire_id,
                        name: call.name.clone(),
                        args: call.arguments.clone(),
                    });
                }
                output.push(Message::assistant_with_tool_calls(
                    flatten_content(&message.content, index, &mut diagnostics),
                    refs,
                ));
                continue;
            }

            if let Some(result) = &message.tool_result {
                let call_name = pending.get(&result.internal_call_id).ok_or_else(|| {
                    if completed.contains(&result.internal_call_id) {
                        HistoryProjectionError::DuplicateResult {
                            index,
                            id: result.internal_call_id.to_string(),
                        }
                    } else {
                        HistoryProjectionError::OrphanResult {
                            index,
                            id: result.internal_call_id.to_string(),
                        }
                    }
                })?;
                if call_name != &result.tool_name {
                    return Err(HistoryProjectionError::ResultNameMismatch {
                        index,
                        expected: call_name.clone(),
                        actual: result.tool_name.clone(),
                    });
                }
                let wire_id = aliases.alias_for(&result.internal_call_id)?;
                output.push(Message::tool_with_status(
                    flatten_content(&result.content, index, &mut diagnostics),
                    Some(wire_id),
                    Some(result.internal_call_id.clone()),
                    Some(result.tool_name.clone()),
                    result.status.clone(),
                ));
                pending.remove(&result.internal_call_id);
                completed.insert(result.internal_call_id.clone());
                continue;
            }

            if !pending.is_empty() {
                if self.policy.synthesize_missing_results {
                    for (id, name) in std::mem::take(&mut pending) {
                        let wire_id = aliases.alias_for(&id)?;
                        output.push(Message::tool_with_status(
                            "tool result unavailable; external effect is unknown",
                            Some(wire_id),
                            Some(id.clone()),
                            Some(name),
                            ToolResultStatus::UnknownEffect,
                        ));
                        diagnostics.push(ProjectionDiagnostic::SynthesizedUnknownResult {
                            id: id.to_string(),
                        });
                        completed.insert(id);
                    }
                } else {
                    let id = pending.keys().next().expect("pending is not empty");
                    return Err(HistoryProjectionError::MissingResult { id: id.to_string() });
                }
            }

            output.push(Message {
                role: message.role.clone(),
                content: flatten_content(&message.content, index, &mut diagnostics),
                tool_calls: Vec::new(),
                tool_call_id: None,
                internal_call_id: None,
                tool_name: None,
                tool_result_status: None,
                content_blocks: message.content.clone(),
            });
        }

        if !pending.is_empty() {
            if !self.policy.synthesize_missing_results {
                let id = pending.keys().next().expect("pending is not empty");
                return Err(HistoryProjectionError::MissingResult { id: id.to_string() });
            }
            for (id, name) in pending {
                let wire_id = aliases.alias_for(&id)?;
                output.push(Message::tool_with_status(
                    "tool result unavailable; external effect is unknown",
                    Some(wire_id),
                    Some(id.clone()),
                    Some(name),
                    ToolResultStatus::UnknownEffect,
                ));
                diagnostics
                    .push(ProjectionDiagnostic::SynthesizedUnknownResult { id: id.to_string() });
            }
        }

        Ok(ProjectedHistory {
            messages: output,
            wire_ids: aliases.ids,
            diagnostics,
        })
    }

    /// Deterministically upgrades the legacy Message history without
    /// guessing provider-specific payloads.
    pub fn from_legacy(
        &self,
        history: &[Message],
    ) -> Result<ProjectedHistory, HistoryProjectionError> {
        let mut source = Vec::with_capacity(history.len());
        let mut pending = BTreeMap::<String, (InternalCallId, String)>::new();
        for (index, message) in history.iter().enumerate() {
            if message.role == Role::Assistant && !message.tool_calls.is_empty() {
                let mut calls = Vec::new();
                for (call_index, call) in message.tool_calls.iter().enumerate() {
                    let id = InternalCallId::new(format!("legacy-call-{index}-{call_index}"))
                        .map_err(|error| HistoryProjectionError::InvalidMessage {
                            index,
                            reason: error.to_string(),
                        })?;
                    pending.insert(call.id.clone(), (id.clone(), call.name.clone()));
                    calls.push(ToolCall {
                        internal_call_id: id.clone(),
                        name: call.name.clone(),
                        arguments: call.args.clone(),
                        wire_reference: WireCallReference::new(
                            self.target_protocol.clone(),
                            call.id.clone(),
                        )
                        .ok(),
                    });
                }
                source.push(ModelMessage {
                    schema_version: 1,
                    role: Role::Assistant,
                    content: vec![ContentBlock::text(message.content.clone())],
                    tool_calls: calls,
                    tool_result: None,
                });
            } else if message.role == Role::Tool {
                let id = if let Some(wire_id) = &message.tool_call_id {
                    pending
                        .get(wire_id)
                        .map(|(id, _)| id.clone())
                        .or_else(|| {
                            message.internal_call_id.clone().filter(|candidate| {
                                pending
                                    .values()
                                    .any(|(pending_id, _)| pending_id == candidate)
                            })
                        })
                        .unwrap_or_else(|| {
                            InternalCallId::new(format!("legacy-result-{index}"))
                                .expect("deterministic legacy id is valid")
                        })
                } else if let Some(id) = &message.internal_call_id {
                    id.clone()
                } else if self.policy.allow_legacy_tool_results {
                    let id = pending
                        .values()
                        .next()
                        .map(|(id, _)| id.clone())
                        .unwrap_or_else(|| {
                            InternalCallId::new(format!("legacy-result-{index}"))
                                .expect("deterministic legacy id is valid")
                        });
                    source.push(ModelMessage::tool(ToolResult {
                        internal_call_id: id.clone(),
                        tool_name: message
                            .tool_name
                            .clone()
                            .or_else(|| {
                                pending
                                    .values()
                                    .find(|(candidate, _)| candidate == &id)
                                    .map(|(_, name)| name.clone())
                            })
                            .unwrap_or_else(|| "legacy_tool".to_string()),
                        content: vec![ContentBlock::text(message.content.clone())],
                        status: message.tool_result_status.clone().unwrap_or_default(),
                        error_code: None,
                    }));
                    continue;
                } else {
                    return Err(HistoryProjectionError::OrphanResult {
                        index,
                        id: "missing".to_string(),
                    });
                };
                let name = message
                    .tool_name
                    .clone()
                    .or_else(|| {
                        pending
                            .values()
                            .find(|(candidate, _)| candidate == &id)
                            .map(|(_, name)| name.clone())
                    })
                    .unwrap_or_else(|| "legacy_tool".to_string());
                source.push(ModelMessage::tool(ToolResult {
                    internal_call_id: id,
                    tool_name: name,
                    content: vec![ContentBlock::text(message.content.clone())],
                    status: message.tool_result_status.clone().unwrap_or_default(),
                    error_code: None,
                }));
            } else {
                source.push(ModelMessage::text(
                    message.role.clone(),
                    message.content.clone(),
                ));
            }
        }
        self.project(&source).map(|mut projected| {
            for (index, message) in history.iter().enumerate() {
                if message.role == Role::Tool && message.tool_call_id.is_none() {
                    projected
                        .diagnostics
                        .push(ProjectionDiagnostic::LegacyToolResultId {
                            index,
                            id: format!("legacy-result-{index}"),
                        });
                }
            }
            projected
        })
    }
}

#[derive(Debug, Default)]
struct AliasMap {
    target_protocol: String,
    preserve_source_wire_ids: bool,
    ids: BTreeMap<InternalCallId, String>,
    used: BTreeSet<String>,
}

impl AliasMap {
    fn new(target_protocol: &str, preserve_source_wire_ids: bool) -> Self {
        Self {
            target_protocol: target_protocol.to_string(),
            preserve_source_wire_ids,
            ..Self::default()
        }
    }

    fn alias(&mut self, call: &ToolCall) -> Result<String, HistoryProjectionError> {
        if let Some(existing) = self.ids.get(&call.internal_call_id) {
            return Ok(existing.clone());
        }
        let candidate = call
            .wire_reference
            .as_ref()
            .filter(|reference| {
                self.preserve_source_wire_ids || reference.protocol == self.target_protocol
            })
            .map(|reference| reference.value.clone())
            .unwrap_or_else(|| deterministic_alias(call.internal_call_id.as_str()));
        self.insert(call.internal_call_id.clone(), candidate)
    }

    fn alias_for(&mut self, id: &InternalCallId) -> Result<String, HistoryProjectionError> {
        if let Some(existing) = self.ids.get(id) {
            return Ok(existing.clone());
        }
        self.insert(id.clone(), deterministic_alias(id.as_str()))
    }

    fn insert(
        &mut self,
        id: InternalCallId,
        candidate: String,
    ) -> Result<String, HistoryProjectionError> {
        let mut alias = sanitize_alias(&candidate);
        let base = alias.clone();
        let mut suffix = 1usize;
        while self.used.contains(&alias) {
            suffix = suffix.saturating_add(1);
            alias = format!("{base}_{suffix}");
        }
        if alias.len() > MAX_TOOL_ID_BYTES {
            alias.truncate(MAX_TOOL_ID_BYTES);
        }
        self.used.insert(alias.clone());
        self.ids.insert(id, alias.clone());
        Ok(alias)
    }
}

fn deterministic_alias(id: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("rove_call_{hash:016x}")
}

fn sanitize_alias(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "rove_call".to_string()
    } else {
        sanitized
    }
}

fn flatten_content(
    content: &[ContentBlock],
    index: usize,
    diagnostics: &mut Vec<ProjectionDiagnostic>,
) -> String {
    let mut text = String::new();
    for block in content {
        if let Some(value) = block.text_value() {
            text.push_str(value);
        } else if let ContentBlock::RichReference {
            kind, reference, ..
        } = block
        {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str("[rich ");
            text.push_str(kind);
            text.push_str(": ");
            text.push_str(reference);
            text.push(']');
            diagnostics.push(ProjectionDiagnostic::RichContentDowngraded { index });
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssistantTurn;

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            internal_call_id: InternalCallId::new(id).unwrap(),
            name: name.to_string(),
            arguments: serde_json::json!({"value":1}),
            wire_reference: Some(WireCallReference::new("anthropic-messages", id).unwrap()),
        }
    }

    #[test]
    fn cross_provider_projection_aliases_call_and_result_together() {
        let call = call("internal-1", "echo");
        let result = ToolResult::text(call.internal_call_id.clone(), "echo", "ok");
        let source = vec![
            ModelMessage {
                schema_version: 1,
                role: Role::Assistant,
                content: vec![ContentBlock::text("checking")],
                tool_calls: vec![call],
                tool_result: None,
            },
            ModelMessage::tool(result),
        ];
        let projected = HistoryProjector::new("openai-completions")
            .project(&source)
            .unwrap();
        assert_eq!(
            projected.messages[0].tool_calls[0].id,
            projected.messages[1].tool_call_id.clone().unwrap()
        );
        assert_ne!(projected.messages[0].tool_calls[0].id, "internal-1");
    }

    #[test]
    fn duplicate_or_orphan_results_fail_closed() {
        let id = InternalCallId::new("call-1").unwrap();
        let result = || ModelMessage::tool(ToolResult::text(id.clone(), "echo", "ok"));
        assert!(matches!(
            HistoryProjector::new("fake").project(&[result()]),
            Err(HistoryProjectionError::OrphanResult { .. })
        ));

        let source = vec![
            ModelMessage::assistant(AssistantTurn {
                tool_calls: vec![call("call-1", "echo")],
                stop_reason: crate::StopReason::ToolUse,
                ..AssistantTurn::default()
            }),
            result(),
            result(),
        ];
        assert!(matches!(
            HistoryProjector::new("fake").project(&source),
            Err(HistoryProjectionError::DuplicateResult { .. })
        ));
    }

    #[test]
    fn legacy_result_without_native_id_uses_explicit_compatibility_projection() {
        let history = vec![
            Message::assistant_with_tool_calls(
                "call",
                vec![ToolCallRef {
                    id: "wire-1".to_string(),
                    name: "echo".to_string(),
                    args: serde_json::json!({"value":1}),
                }],
            ),
            Message::tool("ok", None),
        ];
        let projected = HistoryProjector::new("fake")
            .with_policy(HistoryProjectionPolicy {
                allow_legacy_tool_results: true,
                synthesize_missing_results: false,
            })
            .from_legacy(&history)
            .unwrap();
        assert_eq!(projected.messages.len(), 2);
        assert!(
            projected
                .diagnostics
                .iter()
                .any(|item| matches!(item, ProjectionDiagnostic::LegacyToolResultId { .. }))
        );
    }
}
