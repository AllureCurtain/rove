/// Memory traits — M0 is a stub, M1+ adds real working/durable memory.
pub trait MemoryStore: Send + Sync {
    /// Retrieve relevant context for the current task.
    fn retrieve(&self, query: &str) -> Vec<String>;

    /// Store a new memory entry.
    fn store(&mut self, key: &str, value: &str);
}
