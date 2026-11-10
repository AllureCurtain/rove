//! Bounded compaction-request facts — the data record of what one bounded
//! shrink request elided, plus the pruning policy version string. The
//! compaction machinery that produces them stays in
//! `rove-runtime::context::compaction`.

/// Version label recorded on pruning facts so a trace reader can tell two
/// policies apart.
pub const PRUNING_POLICY_VERSION: &str = "rove.pruning.v1";

/// What one bounded shrink request elided, as a deterministic fact.
///
/// The request is the material the summary model is given, and it is deliberately
/// *not* all of the dropped prefix: payloads above the per-result budget become
/// excerpts, and messages beyond the request's own bounds are left out. Both are
/// recorded here so the model is never silently shown less than the contract
/// says it may be, and so a trace reader can tell what the summary was built
/// from without re-deriving the policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruningFacts {
    /// How many tool-result payloads were replaced by head+tail excerpts.
    pub pruned_tool_results: usize,
    /// Serialized bytes those messages occupied in the request before the
    /// projection. Measured on the message, not on its payload text, because the
    /// blocks dropped with a payload and the framing that carries both are part
    /// of what the request stopped carrying, and a replayed message carries the
    /// payload twice (flattened text plus the canonical block).
    pub pruned_payload_bytes: usize,
    /// Serialized bytes those messages occupy in the request after the
    /// projection, so `pruned_payload_bytes - pruned_excerpt_bytes` is exactly
    /// the loss. Measured the same way, for the same reason.
    pub pruned_excerpt_bytes: usize,
    /// Older messages left out because the request's own bounds were reached.
    pub omitted_older_messages: usize,
    /// Original bytes of the messages left out.
    pub omitted_older_bytes: usize,
    /// `result id + content digest` for the elided payloads, newest first,
    /// capped at `COMPACTION_PRUNED_DIGEST_LIMIT`.
    pub pruned_payload_digests: Vec<String>,
}

impl PruningFacts {
    /// Whether this request elided anything at all.
    pub fn is_empty(&self) -> bool {
        self.pruned_tool_results == 0 && self.omitted_older_messages == 0
    }
}
