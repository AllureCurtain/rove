use std::{collections::HashMap, sync::Arc};

use thiserror::Error;

use super::{WireProtocol, WireProtocolId};

/// Immutable-after-assembly registry for wire protocol strategies.
#[derive(Default)]
pub struct WireProtocolRegistry {
    protocols: HashMap<WireProtocolId, Arc<dyn WireProtocol>>,
}

impl WireProtocolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        protocol: Arc<dyn WireProtocol>,
    ) -> Result<(), WireProtocolRegistryError> {
        let id = protocol.id().clone();
        if self.protocols.contains_key(&id) {
            return Err(WireProtocolRegistryError::Duplicate { id });
        }
        self.protocols.insert(id, protocol);
        Ok(())
    }

    pub fn get(
        &self,
        id: &WireProtocolId,
    ) -> Result<Arc<dyn WireProtocol>, WireProtocolRegistryError> {
        self.protocols
            .get(id)
            .cloned()
            .ok_or_else(|| WireProtocolRegistryError::Unknown {
                id: id.clone(),
                available: self.ids(),
            })
    }

    pub fn ids(&self) -> Vec<WireProtocolId> {
        let mut ids = self.protocols.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn len(&self) -> usize {
        self.protocols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.protocols.is_empty()
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum WireProtocolRegistryError {
    #[error("wire protocol '{id}' is already registered")]
    Duplicate { id: WireProtocolId },
    #[error("wire protocol '{id}' is not registered; available protocols: {available:?}")]
    Unknown {
        id: WireProtocolId,
        available: Vec<WireProtocolId>,
    },
}

#[cfg(test)]
mod tests {
    use reqwest::{Method, StatusCode, header::HeaderMap};

    use super::*;
    use crate::provider::{AuthStyle, Framing, StreamDecoder, WireRequest, WireRequestInput};
    use crate::{ModelError, ModelEvent};

    struct NoopDecoder;

    impl StreamDecoder for NoopDecoder {
        fn push(&mut self, _frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
            Ok(Vec::new())
        }
    }

    struct TestProtocol {
        id: WireProtocolId,
    }

    impl TestProtocol {
        fn new(id: &str) -> Self {
            Self {
                id: WireProtocolId::new(id).unwrap(),
            }
        }
    }

    impl WireProtocol for TestProtocol {
        fn id(&self) -> &WireProtocolId {
            &self.id
        }

        fn build_request(&self, _input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
            Ok(WireRequest {
                method: Method::POST,
                path: "/test".to_string(),
                headers: HeaderMap::new(),
                body: serde_json::json!({}),
            })
        }

        fn framing(&self) -> Framing {
            Framing::JsonLines
        }

        fn decoder(&self) -> Box<dyn StreamDecoder> {
            Box::new(NoopDecoder)
        }

        fn classify_error(
            &self,
            status: StatusCode,
            _headers: &HeaderMap,
            _body: &str,
        ) -> ModelError {
            ModelError::RequestFailed(format!("HTTP {status}"))
        }

        fn default_auth_style(&self) -> AuthStyle {
            AuthStyle::None
        }
    }

    #[test]
    fn registers_and_resolves_open_protocol_ids() {
        let mut registry = WireProtocolRegistry::new();
        registry
            .register(Arc::new(TestProtocol::new("vendor/custom")))
            .unwrap();

        let id = WireProtocolId::new("vendor/custom").unwrap();
        let protocol = registry.get(&id).unwrap();

        assert_eq!(protocol.id(), &id);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn duplicate_registration_is_rejected_without_replacement() {
        let mut registry = WireProtocolRegistry::new();
        registry
            .register(Arc::new(TestProtocol::new("vendor/custom")))
            .unwrap();

        let error = registry
            .register(Arc::new(TestProtocol::new("vendor/custom")))
            .unwrap_err();

        assert!(matches!(
            error,
            WireProtocolRegistryError::Duplicate { ref id }
                if id.as_str() == "vendor/custom"
        ));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn unknown_error_lists_available_ids_in_stable_order() {
        let mut registry = WireProtocolRegistry::new();
        registry
            .register(Arc::new(TestProtocol::new("zeta/chat")))
            .unwrap();
        registry
            .register(Arc::new(TestProtocol::new("alpha/chat")))
            .unwrap();

        let missing = WireProtocolId::new("missing/chat").unwrap();
        let error = registry.get(&missing).err().unwrap();

        assert_eq!(
            error,
            WireProtocolRegistryError::Unknown {
                id: missing,
                available: vec![
                    WireProtocolId::new("alpha/chat").unwrap(),
                    WireProtocolId::new("zeta/chat").unwrap(),
                ],
            }
        );
    }
}
