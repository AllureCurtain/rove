use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{CallId, ToolDescriptor, ToolError, ToolMutation, validate_tool_args};

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
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub mutations: Vec<ToolMutation>,
}

impl ToolOutput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            mutations: Vec::new(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolDescriptor;

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError>;
}

/// In-memory registry of embedding- or runtime-supplied tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.schema().name.clone(), tool);
    }

    pub fn schemas(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|tool| tool.schema()).collect()
    }

    pub fn model_schemas(&self) -> Vec<rove_models::ToolSchema> {
        self.tools
            .values()
            .map(|tool| tool.schema().model_schema())
            .collect()
    }

    pub fn schema(&self, name: &str) -> Result<ToolDescriptor, ToolError> {
        self.tools
            .get(name)
            .map(|tool| tool.schema())
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
        let tool = self.tools.get(name).ok_or_else(|| ToolError::UnknownTool {
            name: name.to_string(),
        })?;
        validate_tool_args(&tool.schema().parameters, &args)?;
        tool.execute(args, ctx).await
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

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
