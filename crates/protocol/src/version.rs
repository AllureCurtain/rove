//! Protocol version and the compatibility rules that govern it.

/// Version of the Rove wire protocol carried by SSE events.
///
/// Bump this when a change would make an older client misread a newer server.
/// Adding an optional field, or adding a variant a client is expected to skip,
/// does not require a bump.
///
/// | version | shipped with | change |
/// |---------|--------------|--------|
/// | 1       | Phase 4      | first explicitly versioned envelope; identifiers, lifecycle enums, and the `v` field on stream events |
pub const PROTOCOL_VERSION: u32 = 1;

/// Serde default hook so a deserialized event that predates the `v` field is
/// read as version 1 rather than failing.
pub const fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::{PROTOCOL_VERSION, protocol_version};

    #[test]
    fn the_serde_default_matches_the_advertised_version() {
        assert_eq!(protocol_version(), PROTOCOL_VERSION);
    }
}
