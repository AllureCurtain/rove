mod auth;
mod client;
mod external_adapter;
mod framing;
mod id;
mod registry;
mod transport;
mod wire;

pub mod protocols;

pub use auth::{AuthConfigurationError, Redacted, ResolvedAuth, ResolvedHeader};
pub use client::{ProviderClient, ProviderClientConfig, ProviderClientError};
pub use external_adapter::{ExternalAdapterClient, ExternalAdapterConfig};
pub use framing::{FrameBuffer, FramingError, FramingLimits};
pub use id::{
    ANTHROPIC_MESSAGES_PROTOCOL, EXTERNAL_ADAPTER_V1_PROTOCOL, FAKE_PROTOCOL, OLLAMA_PROTOCOL,
    OPENAI_COMPLETIONS_PROTOCOL, OPENAI_RESPONSES_PROTOCOL, WireProtocolId, WireProtocolIdError,
};
pub use registry::{WireProtocolRegistry, WireProtocolRegistryError};
pub use transport::{RedirectPolicy, Transport, TransportConfig, TransportError};
pub use wire::{AuthStyle, Framing, StreamDecoder, WireProtocol, WireRequest, WireRequestInput};
