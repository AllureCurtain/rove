use super::traits::MemoryStore;

/// In-memory working memory for the current session.
///
/// M0: Simple key-value store. M1+: structured memory with retrieval.
pub struct WorkingMemory {
    entries: Vec<(String, String)>,
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl Default for WorkingMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStore for WorkingMemory {
    fn retrieve(&self, _query: &str) -> Vec<String> {
        // M0: return all entries (no semantic search yet)
        self.entries.iter().map(|(_, v)| v.clone()).collect()
    }

    fn store(&mut self, key: &str, value: &str) {
        self.entries.push((key.to_string(), value.to_string()));
    }
}
