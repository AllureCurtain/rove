//! Bounded, redacted session evidence export.
//!
//! One sanitized value is the source for JSON, HTML, and Markdown so the
//! shareable formats cannot drift on secret handling or evidence coverage.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderValue, Response, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::docs;
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::artifacts::{ProductArtifactsResponse, load_session_artifacts};
use super::usage::load_product_session_usage;
use super::{
    ProductControl, ProductFork, ProductForkId, ProductSessionId, ProductSessionRunModelView,
    ProductSessionStatus, ProductSessionUsageResponse, ProductTranscriptPartialReason,
    ProductTranscriptResponse, ProductWorkspaceId, ProductWorkspaceKind,
};

const EXPORT_SCHEMA_VERSION: u32 = 1;
const EXPORT_KIND: &str = "rove.session.evidence";
const MAX_EXPORT_STRING_BYTES: usize = 64 * 1024;
const MAX_EXPORT_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXPORT_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductExportFormat {
    #[default]
    Json,
    Html,
    Markdown,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProductExportQuery {
    #[serde(default)]
    pub format: ProductExportFormat,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductSessionExport {
    pub schema_version: u32,
    pub export_kind: &'static str,
    pub exported_at: String,
    pub session: ProductExportSession,
    pub workspace: ProductExportWorkspace,
    pub lineage: ProductExportLineage,
    pub transcript: ProductTranscriptResponse,
    pub controls: Vec<ProductControl>,
    pub run_models: Vec<ProductSessionRunModelView>,
    pub usage: ProductSessionUsageResponse,
    pub artifacts: ProductArtifactsResponse,
    pub partial_reasons: ProductExportPartialReasons,
    pub safety: ProductExportSafety,
    pub redaction: ProductExportRedactionSummary,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductExportSession {
    pub id: ProductSessionId,
    pub title: String,
    pub status: ProductSessionStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductExportWorkspace {
    pub id: ProductWorkspaceId,
    pub display_name: String,
    pub kind: ProductWorkspaceKind,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductExportLineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<ProductSessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point_seq: Option<u64>,
    pub direct_children: Vec<ProductExportChild>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductExportChild {
    pub fork_id: ProductForkId,
    pub product_session_id: ProductSessionId,
    pub source_run_id: String,
    pub fork_at_event_seq: u64,
    pub created_at: String,
}

impl From<ProductFork> for ProductExportChild {
    fn from(value: ProductFork) -> Self {
        Self {
            fork_id: value.id,
            product_session_id: value.child_product_session_id,
            source_run_id: value.source_runtime_run_id.to_string(),
            fork_at_event_seq: value.fork_at_event_seq,
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductExportPartialReasons {
    pub transcript: Vec<ProductTranscriptPartialReason>,
    pub usage: Vec<String>,
    pub artifacts: Vec<String>,
    pub export: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProductExportSafety {
    pub artifact_bytes_included: bool,
    pub raw_secrets_included: bool,
    pub absolute_paths_included: bool,
    pub hidden_reasoning_included: bool,
    pub string_limit_bytes: usize,
    pub total_text_limit_bytes: usize,
    pub response_limit_bytes: usize,
}

#[derive(Debug, Default, Serialize, ToSchema)]
pub struct ProductExportRedactionSummary {
    pub secret_fields: u64,
    pub secret_patterns: u64,
    pub environment_values: u64,
    pub absolute_paths: u64,
    pub hidden_reasoning_fields: u64,
    pub truncated_strings: u64,
    pub export_budget_truncations: u64,
    pub emitted_text_bytes: u64,
}

#[utoipa::path(
    post,
    path = "/product/sessions/{session_id}/export",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(
        ("session_id" = ProductSessionId, Path, description = "Product session id"),
        ProductExportQuery
    ),
    responses(
        (status = 200, description = "Redacted, bounded session evidence download", body = ProductSessionExport),
        (status = 400, description = "Unsupported or oversized export", body = ApiErrorResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Evidence collection or serialization failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore or transcript reader is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn export_product_session(
    State(state): State<ApiState>,
    AxumPath(session_id): AxumPath<ProductSessionId>,
    Query(query): Query<ProductExportQuery>,
) -> Result<Response<Body>, ApiError> {
    let value = build_export_value(&state, &session_id).await?;
    let (media_type, extension, body) = match query.format {
        ProductExportFormat::Json => (
            "application/json; charset=utf-8",
            "json",
            render_json(&value)?,
        ),
        ProductExportFormat::Html => (
            "text/html; charset=utf-8",
            "html",
            render_html(&value).into_bytes(),
        ),
        ProductExportFormat::Markdown => (
            "text/markdown; charset=utf-8",
            "md",
            render_markdown(&value).into_bytes(),
        ),
    };
    if body.len() > MAX_EXPORT_BODY_BYTES {
        return Err(ApiError::bad_request_with_code(
            "product_export_too_large",
            format!(
                "rendered evidence export exceeds the {} byte response limit",
                MAX_EXPORT_BODY_BYTES
            ),
        ));
    }
    export_response(&session_id, extension, media_type, body)
}

async fn build_export_value(
    state: &ApiState,
    session_id: &ProductSessionId,
) -> Result<Value, ApiError> {
    let store = state.product_store()?;
    let context = store.get_session_context(session_id).await?;
    let transcript = state
        .product_transcript_reader()?
        .read_transcript(session_id)
        .await?;
    let controls = store.list_controls(session_id, None).await?;
    let children = store.list_forks(session_id).await?;
    let run_models = store.list_session_run_models(session_id).await?;
    let usage = load_product_session_usage(state, session_id).await?;
    let artifacts = load_session_artifacts(state, session_id, true).await?;

    let partial_reasons = ProductExportPartialReasons {
        transcript: transcript.partial_reasons.clone(),
        usage: usage.partial_reasons.clone(),
        artifacts: artifacts.partial_reasons.clone(),
        export: Vec::new(),
    };
    let session = &context.session;
    let export = ProductSessionExport {
        schema_version: EXPORT_SCHEMA_VERSION,
        export_kind: EXPORT_KIND,
        exported_at: chrono::Utc::now().to_rfc3339(),
        session: ProductExportSession {
            id: session.id.clone(),
            title: session.title.clone(),
            status: session.status,
            created_at: session.created_at.clone(),
            updated_at: session.updated_at.clone(),
        },
        workspace: ProductExportWorkspace {
            id: context.workspace.id.clone(),
            display_name: context.workspace.display_name.clone(),
            kind: context.workspace.kind,
        },
        lineage: ProductExportLineage {
            parent_session_id: session.parent_session_id.clone(),
            fork_point_run_id: session.fork_point_run_id.map(|id| id.to_string()),
            fork_point_seq: session.fork_point_seq,
            direct_children: children.into_iter().map(ProductExportChild::from).collect(),
        },
        transcript,
        controls,
        run_models,
        usage,
        artifacts,
        partial_reasons,
        safety: ProductExportSafety {
            artifact_bytes_included: false,
            raw_secrets_included: false,
            absolute_paths_included: false,
            hidden_reasoning_included: false,
            string_limit_bytes: MAX_EXPORT_STRING_BYTES,
            total_text_limit_bytes: MAX_EXPORT_TEXT_BYTES,
            response_limit_bytes: MAX_EXPORT_BODY_BYTES,
        },
        redaction: ProductExportRedactionSummary::default(),
    };
    let mut value = serde_json::to_value(export)
        .map_err(|error| ApiError::internal(format!("evidence serialization failed: {error}")))?;
    let summary = Redactor::from_environment(&context.workspace.canonical_root).redact(&mut value);
    value["redaction"] = serde_json::to_value(summary).map_err(|error| {
        ApiError::internal(format!("redaction summary serialization failed: {error}"))
    })?;
    Ok(value)
}

fn export_response(
    session_id: &ProductSessionId,
    extension: &str,
    media_type: &'static str,
    body: Vec<u8>,
) -> Result<Response<Body>, ApiError> {
    let filename = format!("rove-session-{session_id}-evidence.{extension}");
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(media_type));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|error| ApiError::internal(format!("invalid export filename: {error}")))?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if extension == "html" {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'",
            ),
        );
    }
    Ok(response)
}

fn render_json(value: &Value) -> Result<Vec<u8>, ApiError> {
    let mut body = serde_json::to_vec_pretty(value)
        .map_err(|error| ApiError::internal(format!("evidence serialization failed: {error}")))?;
    body.push(b'\n');
    Ok(body)
}

fn render_html(value: &Value) -> String {
    let title = value
        .pointer("/session/title")
        .and_then(Value::as_str)
        .unwrap_or("Rove session");
    let session_id = value
        .pointer("/session/id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let exported_at = value
        .get("exported_at")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let run_count = value
        .pointer("/transcript/segments")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let event_count = value
        .pointer("/transcript/segments")
        .and_then(Value::as_array)
        .map(|segments| {
            segments
                .iter()
                .filter_map(|segment| segment.get("events").and_then(Value::as_array))
                .map(Vec::len)
                .sum::<usize>()
        })
        .unwrap_or_default();
    let payload = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>body{{font-family:system-ui,sans-serif;max-width:960px;margin:2rem auto;padding:0 1rem;color:#171717;background:#fff}}h1{{font-size:1.5rem}}.meta{{color:#555}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#f5f5f5;border:1px solid #ddd;padding:1rem;font-size:.8rem}}code{{font-family:ui-monospace,monospace}}</style></head><body><h1>{}</h1><p class=\"meta\">Session <code>{}</code><br>Exported {}<br>{} run(s), {} canonical event(s)</p><h2>Evidence payload</h2><pre>{}</pre></body></html>\n",
        html_escape(title),
        html_escape(title),
        html_escape(session_id),
        html_escape(exported_at),
        run_count,
        event_count,
        html_escape(&payload),
    )
}

fn render_markdown(value: &Value) -> String {
    let session_id = value
        .pointer("/session/id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let exported_at = value
        .get("exported_at")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let payload = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    let indented = payload
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# Rove session evidence\n\nSession: `{}`  \nExported: `{}`\n\n## Evidence payload\n\n{}\n",
        markdown_inline(session_id),
        markdown_inline(exported_at),
        indented,
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn markdown_inline(value: &str) -> String {
    value.replace('`', "\\`").replace(['\r', '\n'], " ")
}

struct Redactor {
    environment_values: Vec<String>,
    known_paths: Vec<KnownPath>,
}

struct KnownPath {
    value: String,
    marker: &'static str,
}

impl Redactor {
    fn from_environment(workspace_root: &Path) -> Self {
        let mut environment_values = std::env::vars_os()
            .filter_map(|(_, value)| value.into_string().ok())
            .filter(|value| (8..=4096).contains(&value.len()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        environment_values.sort_by_key(|value| std::cmp::Reverse(value.len()));

        let mut known_paths = vec![KnownPath {
            value: workspace_root.to_string_lossy().into_owned(),
            marker: "[REDACTED:workspace_path]",
        }];
        for name in ["USERPROFILE", "HOME", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(name) {
                known_paths.push(KnownPath {
                    value: PathBuf::from(value).to_string_lossy().into_owned(),
                    marker: "[REDACTED:absolute_path]",
                });
            }
        }
        if let Ok(value) = std::env::current_dir() {
            known_paths.push(KnownPath {
                value: value.to_string_lossy().into_owned(),
                marker: "[REDACTED:absolute_path]",
            });
        }
        known_paths.retain(|path| path.value.len() >= 3);
        known_paths.sort_by_key(|path| std::cmp::Reverse(path.value.len()));
        Self {
            environment_values,
            known_paths,
        }
    }

    #[cfg(test)]
    fn for_test(environment_values: Vec<String>, known_paths: Vec<KnownPath>) -> Self {
        Self {
            environment_values,
            known_paths,
        }
    }

    fn redact(&self, value: &mut Value) -> ProductExportRedactionSummary {
        let mut summary = ProductExportRedactionSummary::default();
        let mut remaining = MAX_EXPORT_TEXT_BYTES;
        self.redact_value(value, &mut summary, &mut remaining);
        summary
    }

    fn redact_value(
        &self,
        value: &mut Value,
        summary: &mut ProductExportRedactionSummary,
        remaining: &mut usize,
    ) {
        match value {
            Value::String(text) => {
                let sanitized = self.redact_string(text, summary);
                *text = apply_text_budget(sanitized, summary, remaining);
            }
            Value::Array(items) => {
                for item in items {
                    self.redact_value(item, summary, remaining);
                }
            }
            Value::Object(fields) => {
                for (key, field) in fields {
                    if is_secret_field(key) {
                        summary.secret_fields = summary.secret_fields.saturating_add(1);
                        *field = Value::String(apply_text_budget(
                            "[REDACTED:secret_field]".to_string(),
                            summary,
                            remaining,
                        ));
                    } else if is_hidden_reasoning_field(key) {
                        summary.hidden_reasoning_fields =
                            summary.hidden_reasoning_fields.saturating_add(1);
                        *field = Value::String(apply_text_budget(
                            "[REDACTED:hidden_reasoning]".to_string(),
                            summary,
                            remaining,
                        ));
                    } else {
                        self.redact_value(field, summary, remaining);
                    }
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    fn redact_string(&self, value: &str, summary: &mut ProductExportRedactionSummary) -> String {
        let mut output = value.to_string();
        for path in &self.known_paths {
            let variants = path_variants(&path.value);
            for variant in variants {
                let (next, count) = replace_literal(&output, &variant, path.marker, true);
                output = next;
                summary.absolute_paths = summary
                    .absolute_paths
                    .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
            }
        }
        let (next, home_count) = redact_generic_home_paths(&output);
        output = next;
        summary.absolute_paths = summary
            .absolute_paths
            .saturating_add(u64::try_from(home_count).unwrap_or(u64::MAX));

        for secret in &self.environment_values {
            let (next, count) =
                replace_literal(&output, secret, "[REDACTED:environment_value]", false);
            output = next;
            summary.environment_values = summary
                .environment_values
                .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        }
        let (output, count) = redact_secret_patterns(output);
        summary.secret_patterns = summary
            .secret_patterns
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        output
    }
}

fn apply_text_budget(
    mut value: String,
    summary: &mut ProductExportRedactionSummary,
    remaining: &mut usize,
) -> String {
    if value.len() > MAX_EXPORT_STRING_BYTES {
        value = truncate_utf8(&value, MAX_EXPORT_STRING_BYTES).to_string();
        value.push_str("[TRUNCATED:string_limit]");
        summary.truncated_strings = summary.truncated_strings.saturating_add(1);
    }
    if value.len() > *remaining {
        let prefix = truncate_utf8(&value, *remaining);
        let mut truncated = prefix.to_string();
        truncated.push_str("[TRUNCATED:export_budget]");
        summary.export_budget_truncations = summary.export_budget_truncations.saturating_add(1);
        summary.emitted_text_bytes = summary
            .emitted_text_bytes
            .saturating_add(u64::try_from(truncated.len()).unwrap_or(u64::MAX));
        *remaining = 0;
        return truncated;
    }
    *remaining -= value.len();
    summary.emitted_text_bytes = summary
        .emitted_text_bytes
        .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
    value
}

fn truncate_utf8(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn is_secret_field(key: &str) -> bool {
    let normalized = key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "authorization"
            | "proxy_authorization"
            | "cookie"
            | "set_cookie"
            | "password"
            | "passwd"
            | "secret"
            | "api_key"
            | "apikey"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "client_secret"
            | "private_key"
            | "credential"
            | "credentials"
    ) || (normalized.ends_with("_token") && !normalized.ends_with("_tokens"))
}

fn is_hidden_reasoning_field(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "hidden_reasoning" | "reasoning_content" | "chain_of_thought" | "thinking"
    )
}

fn path_variants(value: &str) -> Vec<String> {
    let mut variants = vec![value.to_string()];
    let forward = value.replace('\\', "/");
    let backward = value.replace('/', "\\");
    if !variants.contains(&forward) {
        variants.push(forward);
    }
    if !variants.contains(&backward) {
        variants.push(backward);
    }
    variants
}

fn replace_literal(
    value: &str,
    needle: &str,
    replacement: &str,
    ascii_case_insensitive: bool,
) -> (String, usize) {
    if needle.is_empty() {
        return (value.to_string(), 0);
    }
    if ascii_case_insensitive && needle.is_ascii() {
        let mut output = String::with_capacity(value.len());
        let mut rest = value;
        let mut count = 0;
        while let Some(index) = find_ascii_case_insensitive(rest, needle) {
            output.push_str(&rest[..index]);
            output.push_str(replacement);
            rest = &rest[index + needle.len()..];
            count += 1;
        }
        output.push_str(rest);
        return (output, count);
    }
    let count = value.matches(needle).count();
    (value.replace(needle, replacement), count)
}

fn find_ascii_case_insensitive(value: &str, needle: &str) -> Option<usize> {
    value
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn redact_generic_home_paths(value: &str) -> (String, usize) {
    let mut output = value.to_string();
    let mut count = 0;
    for prefix in ["C:\\Users\\", "C:/Users/", "/home/", "/Users/"] {
        while let Some(start) = find_ascii_case_insensitive(&output, prefix) {
            let owner_start = start + prefix.len();
            let owner_len = output[owner_start..]
                .find(['/', '\\', ' ', '\t', '\r', '\n', '"', '\''])
                .unwrap_or(output.len() - owner_start);
            if owner_len == 0 {
                break;
            }
            output.replace_range(start..owner_start + owner_len, "[REDACTED:absolute_path]");
            count += 1;
        }
    }
    while let Some(start) = output.find("/root") {
        let end = start + "/root".len();
        if output[end..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            break;
        }
        output.replace_range(start..end, "[REDACTED:absolute_path]");
        count += 1;
    }
    (output, count)
}

fn redact_secret_patterns(mut value: String) -> (String, usize) {
    let mut count = 0;
    for prefix in ["Authorization: Bearer ", "Authorization=Bearer "] {
        let (next, replaced) = redact_token_after_prefix(&value, prefix, true, false);
        value = next;
        count += replaced;
    }
    for prefix in [
        "sk-ant-",
        "sk-proj-",
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AIza",
    ] {
        let (next, replaced) = redact_token_after_prefix(&value, prefix, false, false);
        value = next;
        count += replaced;
    }
    let (next, replaced) = redact_token_after_prefix(&value, "Bearer ", true, false);
    value = next;
    count += replaced;
    for prefix in [
        "password=",
        "passwd=",
        "token=",
        "api_key=",
        "apikey=",
        "secret=",
    ] {
        let (next, replaced) = redact_token_after_prefix(&value, prefix, true, true);
        value = next;
        count += replaced;
    }
    (value, count)
}

fn redact_token_after_prefix(
    value: &str,
    prefix: &str,
    case_insensitive: bool,
    preserve_prefix: bool,
) -> (String, usize) {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    let mut count = 0;
    loop {
        let index = if case_insensitive {
            find_ascii_case_insensitive(rest, prefix)
        } else {
            rest.find(prefix)
        };
        let Some(index) = index else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..index]);
        if preserve_prefix {
            output.push_str(&rest[index..index + prefix.len()]);
        }
        let token_start = index + prefix.len();
        let token_len = rest[token_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '"' | '\'' | ',' | ';' | ')' | ']' | '}')
            })
            .unwrap_or(rest.len() - token_start)
            .min(512);
        if token_len == 0 {
            output.push_str(&rest[index..token_start]);
            rest = &rest[token_start..];
            continue;
        }
        output.push_str("[REDACTED:secret_pattern]");
        rest = &rest[token_start + token_len..];
        count += 1;
    }
    (output, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_scrubs_secret_fields_environment_values_paths_and_hidden_reasoning() {
        let redactor = Redactor::for_test(
            vec!["ENV-CANARY-123456".to_string()],
            vec![KnownPath {
                value: "D:\\private\\workspace".to_string(),
                marker: "[REDACTED:workspace_path]",
            }],
        );
        let mut value = serde_json::json!({
            "authorization": "Bearer raw-auth-canary",
            "message": "ENV-CANARY-123456 at D:\\private\\workspace\\secret.txt",
            "nested": {
                "password": "password-canary",
                "reasoning_content": "private chain",
                "total_tokens": 42,
                "text": "sk-test-secret-canary"
            },
            "other_path": "/home/private-user/project/file.txt"
        });

        let summary = redactor.redact(&mut value);
        let serialized = serde_json::to_string(&value).unwrap();

        for secret in [
            "raw-auth-canary",
            "ENV-CANARY-123456",
            "D:\\private\\workspace",
            "password-canary",
            "private chain",
            "test-secret-canary",
            "private-user",
        ] {
            assert!(!serialized.contains(secret), "leaked {secret}");
        }
        assert_eq!(value["nested"]["total_tokens"], 42);
        assert!(summary.secret_fields >= 2);
        assert!(summary.environment_values >= 1);
        assert!(summary.absolute_paths >= 2);
        assert_eq!(summary.hidden_reasoning_fields, 1);
        assert!(summary.secret_patterns >= 1);
    }

    #[test]
    fn renderers_share_the_sanitized_value_and_html_never_emits_active_markup() {
        let value = serde_json::json!({
            "exported_at": "2026-08-05T00:00:00Z",
            "session": { "id": "session-1", "title": "<script>alert(1)</script>" },
            "transcript": { "segments": [] },
            "message": "[REDACTED:secret_field]"
        });

        let html = render_html(&value);
        let markdown = render_markdown(&value);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("[REDACTED:secret_field]"));
        assert!(markdown.contains("[REDACTED:secret_field]"));
        assert!(markdown.contains("session-1"));
    }

    #[test]
    fn text_budget_is_utf8_safe_and_bounded() {
        let mut summary = ProductExportRedactionSummary::default();
        let mut remaining = 7;
        let output = apply_text_budget("中文-long-value".to_string(), &mut summary, &mut remaining);

        assert!(output.starts_with("中文-"));
        assert!(output.ends_with("[TRUNCATED:export_budget]"));
        assert_eq!(summary.export_budget_truncations, 1);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn usage_token_fields_are_not_treated_as_credentials() {
        assert!(!is_secret_field("prompt_tokens"));
        assert!(!is_secret_field("completion_tokens"));
        assert!(is_secret_field("access_token"));
        assert!(is_secret_field("Authorization"));
    }
}
