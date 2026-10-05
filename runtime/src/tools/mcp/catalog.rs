//! Tool catalog discovery, validation, and snapshotting.
//!
//! Discovery is all-or-nothing. A page that fails validation aborts the whole
//! catalog rather than registering a partial tool set, because a silently
//! shortened catalog looks identical to a server that genuinely offers fewer
//! tools.
//!
//! Remote metadata is untrusted: a server-declared annotation never grants
//! local permission. Every discovered tool is conservatively marked destructive
//! and not parallel-safe, and only the local safety path can relax that.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::protocol::{
    MAX_MCP_LIST_PAGES, MAX_MCP_TOOL_SCHEMA_BYTES, MAX_MCP_TOOLS_PER_SERVER, McpProtocolError,
    bounded_diagnostic, validate_cursor,
};

/// One validated remote tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

/// One validated remote tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCatalogEntry {
    /// Exact remote name, used verbatim in `tools/call`.
    pub remote_name: String,
    /// Local alias, namespaced by server.
    pub local_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub annotations: McpToolAnnotations,
    pub capability_id: String,
    /// Hash of the bounded descriptor received from the server. The raw
    /// descriptor itself is never retained in runtime identity or events.
    pub raw_descriptor_hash: String,
}

/// A complete catalog for one server at one point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCatalogSnapshot {
    pub server_name: String,
    pub protocol_version: String,
    /// Digest of the validated serverInfo/capabilities from initialize.
    #[serde(default)]
    pub server_identity_hash: String,
    pub entries: Vec<McpCatalogEntry>,
    /// Stable digest of the validated catalog, for pinning within a run.
    pub catalog_hash: String,
}

impl McpCatalogSnapshot {
    pub fn tool_count(&self) -> usize {
        self.entries.len()
    }

    pub fn entry(&self, local_name: &str) -> Option<&McpCatalogEntry> {
        self.entries
            .iter()
            .find(|entry| entry.local_name == local_name)
    }
}

/// Namespaced local identity for a remote tool.
pub fn mcp_tool_identity(server_name: &str, remote_name: &str) -> (String, String) {
    let sanitized_server = bounded_identity_component(server_name, "server");
    let sanitized_tool = bounded_identity_component(remote_name, "tool");
    let identity_hash =
        crate::prompt_metadata::stable_hash(&format!("mcp-tool:v1:{server_name}\0{remote_name}"));
    let short_hash = identity_hash
        .strip_prefix("sha256:")
        .unwrap_or(&identity_hash)
        .chars()
        .take(12)
        .collect::<String>();
    (
        format!("mcp__{sanitized_server}__{sanitized_tool}"),
        format!("mcp.{sanitized_server}.{sanitized_tool}.{short_hash}"),
    )
}

/// Namespace prefix reserved for one server's validated tool catalog.
pub fn mcp_server_namespace(server_name: &str) -> String {
    format!(
        "mcp__{}__",
        bounded_identity_component(server_name, "server")
    )
}

fn bounded_identity_component(value: &str, fallback: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect::<String>();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

/// One `tools/list` page as returned by a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPage {
    pub entries: Vec<McpCatalogEntry>,
    pub next_cursor: Option<String>,
}

/// Validate a single `tools/list` result page.
///
/// Enforces the per-page tool bound, per-tool schema size, non-empty exact
/// names, and cursor bounds. The returned cursor is validated but not trusted to
/// terminate: the caller enforces the page limit and repeat detection.
pub fn parse_catalog_page(
    server_name: &str,
    result: &Value,
) -> Result<CatalogPage, McpProtocolError> {
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| McpProtocolError::Transport {
            detail: "MCP tools/list response is missing the tools array".to_string(),
        })?;
    if tools.len() > MAX_MCP_TOOLS_PER_SERVER {
        return Err(McpProtocolError::Transport {
            detail: "MCP tools/list page contains too many tools".to_string(),
        });
    }

    let mut entries = Vec::with_capacity(tools.len());
    for tool in tools {
        entries.push(parse_catalog_entry(server_name, tool)?);
    }

    let next_cursor = match result.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) => Some(validate_cursor(cursor)?),
        Some(_) => {
            return Err(McpProtocolError::Transport {
                detail: "MCP tools/list nextCursor must be a string".to_string(),
            });
        }
    };

    Ok(CatalogPage {
        entries,
        next_cursor,
    })
}

fn parse_catalog_entry(
    server_name: &str,
    tool: &Value,
) -> Result<McpCatalogEntry, McpProtocolError> {
    let remote_name = tool
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| McpProtocolError::Transport {
            detail: "MCP tool entry is missing a usable name".to_string(),
        })?
        .to_string();

    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .map(bounded_diagnostic)
        .unwrap_or_else(|| "MCP server tool".to_string());
    let title = tool
        .get("title")
        .and_then(Value::as_str)
        .map(bounded_diagnostic)
        .filter(|title| !title.is_empty());

    let parameters = tool
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));
    // A pathological schema must not become an unbounded prompt payload.
    let schema_bytes = serde_json::to_vec(&parameters)
        .map_err(|_| McpProtocolError::Transport {
            detail: "MCP tool schema could not be encoded".to_string(),
        })?
        .len();
    if schema_bytes > MAX_MCP_TOOL_SCHEMA_BYTES {
        return Err(McpProtocolError::Transport {
            detail: "MCP tool schema exceeds the supported size".to_string(),
        });
    }

    let (local_name, capability_id) = mcp_tool_identity(server_name, &remote_name);
    rove_models::ModelToolSchema {
        name: local_name.clone(),
        description: description.clone(),
        parameters: parameters.clone(),
    }
    .validate()
    .map_err(|error| McpProtocolError::Transport {
        detail: bounded_diagnostic(&format!("MCP input schema is invalid: {error}")),
    })?;

    let output_schema = tool.get("outputSchema").cloned();
    if let Some(schema) = output_schema.as_ref() {
        rove_models::ModelToolSchema {
            name: local_name.clone(),
            description: String::new(),
            parameters: schema.clone(),
        }
        .validate()
        .map_err(|error| McpProtocolError::Transport {
            detail: bounded_diagnostic(&format!("MCP output schema is invalid: {error}")),
        })?;
    }
    let annotations = parse_annotations(tool.get("annotations"))?;
    let raw_descriptor_hash =
        crate::prompt_metadata::stable_hash(&serde_json::to_string(tool).unwrap_or_default());
    Ok(McpCatalogEntry {
        remote_name,
        local_name,
        title,
        description,
        parameters,
        output_schema,
        annotations,
        capability_id,
        raw_descriptor_hash,
    })
}

fn parse_annotations(value: Option<&Value>) -> Result<McpToolAnnotations, McpProtocolError> {
    let Some(value) = value else {
        return Ok(McpToolAnnotations::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| McpProtocolError::Transport {
            detail: "MCP tool annotations must be an object".to_string(),
        })?;
    let boolean = |name: &str| -> Result<Option<bool>, McpProtocolError> {
        match object.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(McpProtocolError::Transport {
                detail: format!("MCP tool annotation {name} must be boolean"),
            }),
        }
    };
    Ok(McpToolAnnotations {
        read_only_hint: boolean("readOnlyHint")?,
        destructive_hint: boolean("destructiveHint")?,
        idempotent_hint: boolean("idempotentHint")?,
        open_world_hint: boolean("openWorldHint")?,
    })
}

/// Accumulates validated pages into one complete catalog.
///
/// The builder is the place where "complete" is decided, so a caller cannot
/// accidentally register a truncated catalog.
#[derive(Debug)]
pub struct CatalogBuilder {
    server_name: String,
    protocol_version: String,
    server_identity_hash: Option<String>,
    entries: Vec<McpCatalogEntry>,
    seen_cursors: HashSet<String>,
    seen_remote_names: HashSet<String>,
    seen_local_names: HashSet<String>,
    pages: usize,
}

impl CatalogBuilder {
    pub fn new(server_name: impl Into<String>, protocol_version: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            protocol_version: protocol_version.into(),
            server_identity_hash: None,
            entries: Vec::new(),
            seen_cursors: HashSet::new(),
            seen_remote_names: HashSet::new(),
            seen_local_names: HashSet::new(),
            pages: 0,
        }
    }

    /// Pin the identity returned by the completed initialize handshake.
    pub fn with_server_identity_hash(mut self, hash: impl Into<String>) -> Self {
        self.server_identity_hash = Some(hash.into());
        self
    }

    /// Absorb one page and report the cursor to request next, if any.
    pub fn push_page(&mut self, page: CatalogPage) -> Result<Option<String>, McpProtocolError> {
        self.pages += 1;
        if self.pages > MAX_MCP_LIST_PAGES {
            return Err(McpProtocolError::Transport {
                detail: "MCP tools/list exceeded the supported page count".to_string(),
            });
        }

        for entry in page.entries {
            // A duplicate remote name would make `tools/call` ambiguous.
            if !self.seen_remote_names.insert(entry.remote_name.clone()) {
                return Err(McpProtocolError::Transport {
                    detail: "MCP tools/list returned a duplicate tool name".to_string(),
                });
            }
            if !self.seen_local_names.insert(entry.local_name.clone()) {
                return Err(McpProtocolError::Transport {
                    detail: "MCP tool names collide after local alias normalization".to_string(),
                });
            }
            self.entries.push(entry);
        }
        if self.entries.len() > MAX_MCP_TOOLS_PER_SERVER {
            return Err(McpProtocolError::Transport {
                detail: "MCP server exposes too many tools".to_string(),
            });
        }

        match page.next_cursor {
            Some(cursor) => {
                // A repeated cursor is a server bug that would loop forever.
                if !self.seen_cursors.insert(cursor.clone()) {
                    return Err(McpProtocolError::Transport {
                        detail: "MCP tools/list repeated a pagination cursor".to_string(),
                    });
                }
                Ok(Some(cursor))
            }
            None => Ok(None),
        }
    }

    /// Finish the catalog. Fails when the server exposed no usable tool.
    pub fn finish(self) -> Result<McpCatalogSnapshot, McpProtocolError> {
        if self.entries.is_empty() {
            return Err(McpProtocolError::Transport {
                detail: "MCP server exposed no usable tools".to_string(),
            });
        }
        let catalog_hash = catalog_hash(&self.server_name, &self.protocol_version, &self.entries);
        Ok(McpCatalogSnapshot {
            server_name: self.server_name,
            protocol_version: self.protocol_version,
            server_identity_hash: self.server_identity_hash.unwrap_or_else(|| {
                crate::prompt_metadata::stable_hash("mcp-server:unnegotiated:v1")
            }),
            entries: self.entries,
            catalog_hash,
        })
    }
}

fn catalog_hash(server_name: &str, protocol_version: &str, entries: &[McpCatalogEntry]) -> String {
    let projection: Vec<Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "remote_name": entry.remote_name,
                "local_name": entry.local_name,
                "title": entry.title,
                "description": entry.description,
                "parameters": entry.parameters,
                "output_schema": entry.output_schema,
                "annotations": entry.annotations,
                "raw_descriptor_hash": entry.raw_descriptor_hash,
            })
        })
        .collect();
    let payload = json!({
        "server": server_name,
        "protocol_version": protocol_version,
        "tools": projection,
    });
    crate::prompt_metadata::stable_hash(&payload.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> Value {
        json!({
            "name": name,
            "description": "does a thing",
            "inputSchema": { "type": "object", "properties": {} }
        })
    }

    #[test]
    fn a_page_is_validated_and_namespaced() {
        let page = parse_catalog_page("files", &json!({ "tools": [tool("read_file")] })).unwrap();
        assert_eq!(page.entries.len(), 1);
        let entry = &page.entries[0];
        assert_eq!(
            entry.remote_name, "read_file",
            "the exact remote name is kept"
        );
        assert_eq!(entry.local_name, "mcp__files__read_file");
        assert!(entry.capability_id.starts_with("mcp.files.read_file."));
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn a_server_name_with_separators_cannot_forge_another_identity() {
        let (local, capability) = mcp_tool_identity("evil__server", "tool:name");
        assert_eq!(local, "mcp__evil__server__tool_name");
        assert!(capability.starts_with("mcp.evil__server.tool_name."));
        assert_eq!(capability.len(), "mcp.evil__server.tool_name.".len() + 12);
    }

    #[test]
    fn invalid_tool_entries_are_rejected() {
        for invalid in [
            json!({ "tools": [{ "description": "no name" }] }),
            json!({ "tools": [{ "name": "   " }] }),
            json!({ "tools": [{ "name": "" }] }),
        ] {
            assert!(
                parse_catalog_page("files", &invalid).is_err(),
                "must reject {invalid}"
            );
        }
        // A missing tools array is a protocol error, not an empty catalog.
        assert!(parse_catalog_page("files", &json!({})).is_err());
    }

    #[test]
    fn output_schema_and_annotations_are_validated_but_never_grant_local_safety() {
        let result = json!({
            "tools": [{
                "name": "inspect",
                "title": "Inspector",
                "inputSchema": { "type": "object", "properties": {} },
                "outputSchema": {
                    "type": "object",
                    "properties": { "ok": { "type": "boolean" } },
                    "required": ["ok"]
                },
                "annotations": {
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false
                }
            }]
        });
        let page = parse_catalog_page("files", &result).unwrap();
        let entry = &page.entries[0];
        assert_eq!(entry.title.as_deref(), Some("Inspector"));
        assert!(entry.output_schema.is_some());
        assert_eq!(entry.annotations.read_only_hint, Some(true));
        assert_eq!(entry.annotations.destructive_hint, Some(false));
        assert_eq!(entry.annotations.idempotent_hint, Some(true));
        assert_eq!(entry.annotations.open_world_hint, Some(false));
        assert!(entry.raw_descriptor_hash.starts_with("sha256:"));

        let invalid_schema = json!({
            "tools": [{ "name": "bad", "outputSchema": { "type": "mystery" } }]
        });
        assert!(parse_catalog_page("files", &invalid_schema).is_err());
        let invalid_annotations = json!({
            "tools": [{ "name": "bad", "annotations": { "readOnlyHint": "yes" } }]
        });
        assert!(parse_catalog_page("files", &invalid_annotations).is_err());
    }

    #[test]
    fn aliases_that_collide_after_normalization_abort_the_complete_catalog() {
        let page = parse_catalog_page(
            "files",
            &json!({ "tools": [tool("read:file"), tool("read/file")] }),
        )
        .unwrap();
        let mut builder = CatalogBuilder::new("files", "2025-06-18");
        assert!(builder.push_page(page).is_err());
    }

    #[test]
    fn an_oversized_tool_schema_is_rejected() {
        let huge = "x".repeat(MAX_MCP_TOOL_SCHEMA_BYTES);
        let result = json!({
            "tools": [{ "name": "big", "inputSchema": { "type": "object", "description": huge } }]
        });
        assert!(parse_catalog_page("files", &result).is_err());
    }

    #[test]
    fn a_non_string_cursor_is_rejected() {
        let result = json!({ "tools": [tool("a")], "nextCursor": 7 });
        assert!(parse_catalog_page("files", &result).is_err());
    }

    #[test]
    fn pagination_accumulates_every_page_into_one_catalog() {
        let mut builder = CatalogBuilder::new("files", "2025-06-18");
        let first = parse_catalog_page(
            "files",
            &json!({ "tools": [tool("a"), tool("b")], "nextCursor": "page-2" }),
        )
        .unwrap();
        assert_eq!(builder.push_page(first).unwrap().as_deref(), Some("page-2"));

        let second = parse_catalog_page("files", &json!({ "tools": [tool("c")] })).unwrap();
        assert_eq!(builder.push_page(second).unwrap(), None);

        let snapshot = builder.finish().unwrap();
        assert_eq!(snapshot.tool_count(), 3);
        assert!(snapshot.entry("mcp__files__c").is_some());
        assert!(snapshot.catalog_hash.starts_with("sha256:"));
    }

    #[test]
    fn a_repeated_cursor_is_refused_instead_of_looping() {
        let mut builder = CatalogBuilder::new("files", "2025-06-18");
        let page = parse_catalog_page(
            "files",
            &json!({ "tools": [tool("a")], "nextCursor": "same" }),
        )
        .unwrap();
        assert!(builder.push_page(page).is_ok());

        let repeat = parse_catalog_page(
            "files",
            &json!({ "tools": [tool("b")], "nextCursor": "same" }),
        )
        .unwrap();
        assert!(builder.push_page(repeat).is_err());
    }

    #[test]
    fn a_duplicate_tool_name_across_pages_is_refused() {
        let mut builder = CatalogBuilder::new("files", "2025-06-18");
        let first = parse_catalog_page(
            "files",
            &json!({ "tools": [tool("a")], "nextCursor": "p2" }),
        )
        .unwrap();
        builder.push_page(first).unwrap();
        let second = parse_catalog_page("files", &json!({ "tools": [tool("a")] })).unwrap();
        assert!(
            builder.push_page(second).is_err(),
            "a duplicate name would make tools/call ambiguous"
        );
    }

    #[test]
    fn the_page_count_is_bounded() {
        let mut builder = CatalogBuilder::new("files", "2025-06-18");
        for index in 0..MAX_MCP_LIST_PAGES {
            let page = parse_catalog_page(
                "files",
                &json!({
                    "tools": [tool(&format!("tool-{index}"))],
                    "nextCursor": format!("cursor-{index}")
                }),
            )
            .unwrap();
            builder.push_page(page).unwrap();
        }
        let overflow = parse_catalog_page("files", &json!({ "tools": [tool("last")] })).unwrap();
        assert!(builder.push_page(overflow).is_err());
    }

    #[test]
    fn an_empty_catalog_is_an_error_rather_than_a_silent_success() {
        let builder = CatalogBuilder::new("files", "2025-06-18");
        assert!(builder.finish().is_err());
    }

    #[test]
    fn the_catalog_hash_tracks_the_validated_contents() {
        let build = |tool_name: &str| {
            let mut builder = CatalogBuilder::new("files", "2025-06-18");
            let page = parse_catalog_page("files", &json!({ "tools": [tool(tool_name)] })).unwrap();
            builder.push_page(page).unwrap();
            builder.finish().unwrap().catalog_hash
        };
        assert_eq!(build("a"), build("a"), "the hash is stable");
        assert_ne!(build("a"), build("b"), "a changed catalog changes the hash");
    }
}
