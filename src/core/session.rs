use crate::core::types::SessionId;

/// Session tracks a user's interaction across multiple jobs.
///
/// M0: Minimal — just an ID. M1+: working memory, history, resume.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: SessionId,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: SessionId::new(),
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
