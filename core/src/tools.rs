use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::tool_result::{ToolArtifactRef, ToolOutputEnvelope, ToolResultOutcome};
use crate::{CallId, ToolDescriptor, ToolError, ToolMutation, validate_tool_args};
use rove_models::{ModelToolSchema, ToolSchemaValidationError, validate_model_tools};

pub const MAX_CAPABILITY_ID_BYTES: usize = 256;

/// Invocation-scoped context supplied by the Agent harness.
///
/// Lower layers own only call identity and cancellation. An embedding or the
/// persistent runtime may attach typed services through `with_extension`
/// without making `rove-core` depend on workspace, memory, approval, or UI
/// types.
#[derive(Clone)]
pub struct ToolContext<'a> {
    pub call_id: CallId,
    pub cancel_token: CancellationToken,
    extensions: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    invocation: PhantomData<&'a ()>,
}

impl ToolContext<'_> {
    pub fn new(call_id: CallId, cancel_token: CancellationToken) -> Self {
        Self {
            call_id,
            cancel_token,
            extensions: Arc::new(HashMap::new()),
            invocation: PhantomData,
        }
    }

    pub fn with_extension<T>(mut self, extension: Arc<T>) -> Self
    where
        T: Any + Send + Sync,
    {
        Arc::make_mut(&mut self.extensions).insert(TypeId::of::<T>(), extension);
        self
    }

    pub fn extension<T>(&self) -> Option<&T>
    where
        T: Any + Send + Sync,
    {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|extension| extension.as_ref().downcast_ref::<T>())
    }
}

impl std::fmt::Debug for ToolContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("call_id", &self.call_id)
            .field("cancel_token", &self.cancel_token)
            .field("extension_count", &self.extensions.len())
            .finish()
    }
}

/// Result returned directly by a Tool implementation.
///
/// `content` and `mutations` remain the legacy surface every existing tool and
/// consumer uses. `envelope` is the additive rich contract: it is `None` for a
/// plain tool, and when present its `summary_text` is kept identical to
/// `content` so there is exactly one text truth.
///
/// The envelope is boxed because a `ToolOutput` travels inline through several
/// event and turn enums. Inlining it would make every variant of those enums
/// pay for the rich contract even though the common tool returns plain text.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub mutations: Vec<ToolMutation>,
    pub envelope: Option<Box<ToolOutputEnvelope>>,
}

impl ToolOutput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            mutations: Vec::new(),
            envelope: None,
        }
    }

    /// Builds an output from a rich envelope.
    ///
    /// The legacy `content` and `mutations` are projected from the envelope, so
    /// a consumer that knows nothing about envelopes still sees a correct text
    /// result and the same mutations.
    pub fn from_envelope(envelope: ToolOutputEnvelope) -> Self {
        let envelope = envelope.enforce_bounds();
        Self {
            content: envelope.model_projection(),
            mutations: envelope.mutations.clone(),
            envelope: Some(Box::new(envelope)),
        }
    }

    /// The rich outcome, or the `Success` implied by a legacy text result.
    pub fn outcome(&self) -> ToolResultOutcome {
        self.envelope
            .as_ref()
            .map(|envelope| envelope.outcome)
            .unwrap_or_default()
    }

    /// Artifacts this result references, empty for a legacy result.
    pub fn artifacts(&self) -> &[ToolArtifactRef] {
        self.envelope
            .as_ref()
            .map(|envelope| envelope.artifacts.as_slice())
            .unwrap_or_default()
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolDescriptor;

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError>;
}

/// In-memory registry of embedding- or runtime-supplied tools.
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}

struct RegisteredTool {
    tool: Box<dyn Tool>,
    descriptor: ToolDescriptor,
    model_schema: ModelToolSchema,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolRegistrationError {
    #[error("invalid tool schema: {0}")]
    InvalidSchema(#[from] ToolSchemaValidationError),
    #[error("duplicate tool name `{name}`")]
    DuplicateName { name: String },
    #[error("invalid capability ID `{capability_id}` for tool `{tool_name}`")]
    InvalidCapabilityId {
        tool_name: String,
        capability_id: String,
    },
    #[error("invalid operational metadata for tool `{tool_name}`: {reason}")]
    InvalidOperationalMetadata { tool_name: String, reason: String },
    #[error(
        "capability ID `{capability_id}` is already bound to tool `{existing_tool}` and cannot also bind to `{candidate_tool}`"
    )]
    DuplicateCapabilityId {
        capability_id: String,
        existing_tool: String,
        candidate_tool: String,
    },
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        if let Err(error) = self.try_register(tool) {
            panic!("invalid trusted tool registration: {error}");
        }
    }

    pub fn try_register(&mut self, tool: Box<dyn Tool>) -> Result<(), ToolRegistrationError> {
        self.try_register_batch(vec![tool]).map(|_| ())
    }

    /// Validate the complete candidate catalog before committing any entry.
    pub fn try_register_batch(
        &mut self,
        tools: Vec<Box<dyn Tool>>,
    ) -> Result<usize, ToolRegistrationError> {
        let mut candidates = Vec::with_capacity(tools.len());
        let mut candidate_names = BTreeSet::new();
        for tool in tools {
            let descriptor = tool.schema();
            let model_schema = descriptor.model_schema();
            model_schema.validate()?;
            if self.tools.contains_key(&descriptor.name)
                || !candidate_names.insert(descriptor.name.clone())
            {
                return Err(ToolRegistrationError::DuplicateName {
                    name: descriptor.name,
                });
            }
            validate_capability_id(&descriptor)?;
            validate_operational_metadata(&descriptor)?;
            candidates.push(RegisteredTool {
                tool,
                descriptor,
                model_schema,
            });
        }

        let mut model_schemas = self.model_schemas();
        model_schemas.extend(
            candidates
                .iter()
                .map(|candidate| candidate.model_schema.clone()),
        );
        validate_model_tools(&model_schemas)?;

        let mut capabilities = self
            .tools
            .values()
            .filter_map(|entry| {
                entry
                    .descriptor
                    .capability_id
                    .as_ref()
                    .map(|id| (id.clone(), entry.descriptor.name.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        for candidate in &candidates {
            let Some(capability_id) = &candidate.descriptor.capability_id else {
                continue;
            };
            if let Some(existing_tool) = capabilities.get(capability_id) {
                return Err(ToolRegistrationError::DuplicateCapabilityId {
                    capability_id: capability_id.clone(),
                    existing_tool: existing_tool.clone(),
                    candidate_tool: candidate.descriptor.name.clone(),
                });
            }
            capabilities.insert(capability_id.clone(), candidate.descriptor.name.clone());
        }

        let registered = candidates.len();
        for candidate in candidates {
            self.tools
                .insert(candidate.descriptor.name.clone(), candidate);
        }
        Ok(registered)
    }

    /// Operational descriptors for runtime/policy use (not model payloads).
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect()
    }

    /// Model-facing schemas projected from operational descriptors.
    pub fn model_schemas(&self) -> Vec<rove_models::ModelToolSchema> {
        self.tools
            .values()
            .map(|entry| entry.model_schema.clone())
            .collect()
    }

    pub fn descriptor(&self, name: &str) -> Result<ToolDescriptor, ToolError> {
        self.tools
            .get(name)
            .map(|entry| entry.descriptor.clone())
            .ok_or_else(|| ToolError::UnknownTool {
                name: name.to_string(),
            })
    }

    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        let entry = self.tools.get(name).ok_or_else(|| ToolError::UnknownTool {
            name: name.to_string(),
        })?;
        validate_tool_args(&entry.descriptor.parameters, &args)?;
        entry.tool.execute(args, ctx).await
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

fn validate_capability_id(descriptor: &ToolDescriptor) -> Result<(), ToolRegistrationError> {
    let Some(capability_id) = &descriptor.capability_id else {
        return Ok(());
    };
    let valid = capability_id.len() <= MAX_CAPABILITY_ID_BYTES
        && capability_id.split('.').count() >= 2
        && capability_id.split('.').all(|component| {
            !component.is_empty()
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        });
    if !valid {
        return Err(ToolRegistrationError::InvalidCapabilityId {
            tool_name: descriptor.name.clone(),
            capability_id: capability_id.clone(),
        });
    }
    Ok(())
}

fn validate_operational_metadata(descriptor: &ToolDescriptor) -> Result<(), ToolRegistrationError> {
    let Some(capability) = &descriptor.capability else {
        return Ok(());
    };
    let status_valid = !capability.status.trim().is_empty()
        && capability.status.len() <= 64
        && capability
            .status
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if !status_valid {
        return Err(ToolRegistrationError::InvalidOperationalMetadata {
            tool_name: descriptor.name.clone(),
            reason: "capability status must be a portable token of at most 64 bytes".to_string(),
        });
    }
    if capability
        .feature
        .as_ref()
        .is_some_and(|feature| feature.len() > 256)
    {
        return Err(ToolRegistrationError::InvalidOperationalMetadata {
            tool_name: descriptor.name.clone(),
            reason: "capability feature exceeds 256 bytes".to_string(),
        });
    }
    if capability
        .message
        .as_ref()
        .is_some_and(|message| message.len() > 1_024)
    {
        return Err(ToolRegistrationError::InvalidOperationalMetadata {
            tool_name: descriptor.name.clone(),
            reason: "capability message exceeds 1024 bytes".to_string(),
        });
    }
    Ok(())
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct CountingSchemaTool {
        name: String,
        capability_id: Option<String>,
        schema_calls: Arc<AtomicUsize>,
        valid: bool,
    }

    #[async_trait]
    impl Tool for CountingSchemaTool {
        fn schema(&self) -> ToolDescriptor {
            self.schema_calls.fetch_add(1, Ordering::SeqCst);
            ToolDescriptor {
                name: self.name.clone(),
                description: "fixture".to_string(),
                parameters: if self.valid {
                    serde_json::json!({"type": "object", "properties": {}})
                } else {
                    serde_json::json!({"type": "object", "properties": {}, "oneOf": []})
                },
                destructive: false,
                parallel_safe: true,
                capability_id: self.capability_id.clone(),
                capability: None,
            }
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &ToolContext<'_>,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text("ok"))
        }
    }

    fn fixture(name: &str, capability_id: Option<&str>, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
        Box::new(CountingSchemaTool {
            name: name.to_string(),
            capability_id: capability_id.map(str::to_string),
            schema_calls: calls,
            valid: true,
        })
    }

    #[test]
    fn registration_pins_schema_once_and_orders_catalog() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .try_register_batch(vec![
                fixture("zeta", Some("test.zeta"), calls.clone()),
                fixture("alpha", Some("test.alpha"), calls.clone()),
            ])
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            registry
                .descriptors()
                .into_iter()
                .map(|descriptor| descriptor.name)
                .collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );
        let _ = registry.model_schemas();
        let _ = registry.descriptor("alpha").unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failed_batch_is_atomic_for_schema_name_and_capability_errors() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .try_register(fixture("existing", Some("test.shared"), calls.clone()))
            .unwrap();

        let invalid = Box::new(CountingSchemaTool {
            name: "invalid".to_string(),
            capability_id: Some("test.invalid".to_string()),
            schema_calls: calls.clone(),
            valid: false,
        });
        assert!(registry.try_register_batch(vec![invalid]).is_err());
        assert_eq!(registry.len(), 1);

        assert!(matches!(
            registry.try_register_batch(vec![
                fixture("first", Some("test.first"), calls.clone()),
                fixture("existing", Some("test.second"), calls.clone()),
            ]),
            Err(ToolRegistrationError::DuplicateName { .. })
        ));
        assert_eq!(registry.len(), 1);

        assert!(matches!(
            registry.try_register(fixture("candidate", Some("test.shared"), calls.clone())),
            Err(ToolRegistrationError::DuplicateCapabilityId { .. })
        ));
        assert_eq!(registry.len(), 1);
        assert!(!registry.has("candidate"));
    }
}
