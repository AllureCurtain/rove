use super::types::RetrievedChunk;

#[derive(Debug, Clone, Default)]
pub struct RagPromptService;

impl RagPromptService {
    pub fn format_context(&self, query: &str, chunks: &[RetrievedChunk]) -> String {
        let mut out = String::new();
        out.push_str(&format!("RAG evidence for query: {}\n", query.trim()));
        out.push_str("Use only the evidence inside this boundary when citing retrieved context.\n");
        out.push_str("BEGIN RAG EVIDENCE\n");
        for (idx, chunk) in chunks.iter().enumerate() {
            out.push_str(&format!(
                "[{}] id={} path={} score={:.3} source={}",
                idx + 1,
                chunk.id,
                chunk.path,
                chunk.score,
                chunk.source
            ));
            if let Some(heading) = chunk.heading.as_deref() {
                out.push_str(&format!(" heading={heading}"));
            }
            out.push('\n');
            out.push_str(chunk.content.trim());
            out.push_str("\n\n");
        }
        out.push_str("END RAG EVIDENCE");
        out
    }
}
