/**
 * Product API contract types, generated-facing.
 *
 * Every interface/type here is a compile-time alias of
 * `apps/api/openapi.json` through `generated/api-types.ts` (`pnpm
 * gen:api-types`; `pnpm check:api-types` fails on drift). Hand-written
 * fields are gone: the wire shape is the Rust `ToSchema` graph, not a
 * second declaration that can drift from it.
 *
 * What is still hand-written, deliberately:
 * - `export const` limit/name arrays — runtime values the spec cannot
 *   express (they are asserted exact against the generated unions below);
 * - boundary guards (`parseApiErrorResponse`, `parseM1BrowserMigrationRequest`,
 *   `parseStreamEvent`, ...) — runtime checks for bytes that did not come
 *   from the product API trust domain: error payloads, imported legacy
 *   localStorage data, and SSE frames;
 * - `ProductPromptPruning` — the decoded shape, not the wire shape.
 *
 * Everything else — every `Product*` record, request, and response — is the
 * generated schema type under its historical name so callers did not have
 * to change imports.
 */
import type { components } from "../generated/api-types";
import {
  STREAM_EVENT_NAMES,
  type PromptPruningFacts,
} from "../lib/rove-types";

/** Schema lookup helper: `Schema<"X">` is the wire type of one OpenAPI component. */
type Schema<K extends keyof components["schemas"]> = components["schemas"][K];

/** Compile-time exact-match assertion between a const array and a union type. */
type AssertExact<T extends true> = T;


export const M1_BROWSER_SOURCE_SCHEMA_VERSION = 1 as const;

export const MAX_PRODUCT_WORKSPACES = 256;

export const MAX_PRODUCT_SESSIONS = 2_048;

export const MAX_PRODUCT_PROVIDER_PROFILES = 128;

export const MAX_PRODUCT_TEXT_BYTES = 512;

export const MAX_PRODUCT_API_BASE_BYTES = 2_048;

export const MAX_PRODUCT_PATH_BYTES = 32_768;

export const MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES = 128;

export const MAX_PRODUCT_CONTROL_CONTENT_BYTES = 32_768;

export const MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES = 128;

export const PRODUCT_SESSION_STATUSES = [
  "idle",
  "running",
  "error",
  "needs_attention",
  "archived",
] as const;

export const PRODUCT_SESSION_OUTCOMES = [
  "success",
  "failed",
  "cancelled",
] as const;

export const PRODUCT_PROVIDER_TYPES = [
  "openai",
  "openai-responses",
  "anthropic",
  "ollama",
  "fake",
] as const;

export const PRODUCT_THEME_PREFERENCES = ["light", "dark", "system"] as const;

export const PRODUCT_APPROVAL_PREFERENCES = ["ask", "auto", "never"] as const;

export const PRODUCT_REASONING_PREFERENCES = [
  "default",
  "low",
  "medium",
  "high",
] as const;

export const MAX_PRODUCT_MAX_STEPS = 256;

export const PRODUCT_WORKSPACE_KINDS = ["folder", "repo"] as const;

export const PRODUCT_CONNECTION_STATUSES = ["connected"] as const;

export const PRODUCT_STORE_STATUSES = ["ready", "unavailable"] as const;

export const PRODUCT_RESUME_HEALTH_STATUSES = [
  "healthy",
  "needs_attention",
] as const;

export const PRODUCT_EXECUTION_ADAPTERS = ["local"] as const;

export const PRODUCT_EXECUTION_WORKSPACE_KINDS = [
  "folder",
  "repo",
  "task",
] as const;

export const PRODUCT_REVIEW_TARGET_KINDS = [
  "uncommitted",
  "base",
  "commit",
] as const;

export const PRODUCT_REVIEW_STATUSES = [
  "queued",
  "running",
  "pass",
  "findings",
  "partial",
  "stale",
  "needs_attention",
  "unavailable",
  "cancelled",
  "error",
] as const;

export const PRODUCT_REVIEW_CONCLUSIONS = [
  "pass",
  "findings",
  "partial",
  "stale",
  "unavailable",
  "cancelled",
  "error",
] as const;

export const PRODUCT_REVIEW_SEVERITIES = [
  "critical",
  "high",
  "medium",
  "low",
  "info",
] as const;

export const PRODUCT_REVIEW_CONFIDENCES = ["high", "medium", "low"] as const;

export const PRODUCT_REVIEW_LOCATION_STATUSES = [
  "validated",
  "unvalidated",
  "invalid",
] as const;

export const PRODUCT_PRICING_AVAILABILITIES = [
  "priced",
  "local_zero",
  "unpriced",
] as const;

export const MAX_PRODUCT_COMPACTION_SUMMARY_CHARS = 400;

/**
 * Bounded facts about one manual compaction (R9). `triggered` describes *this*
 * call; the remaining fields describe the state the session holds afterwards, so
 * "already compacted", "nothing to compact", and "the breaker refuses" stay
 * distinguishable. The response carries no provider message — only a typed
 * `failure_code`.
 */

export const PRODUCT_FILE_KINDS = ["file", "directory"] as const;

export const PRODUCT_ARTIFACT_SOURCE_KINDS = [
  "report",
  "task_state",
  "trace",
  "registered",
  "tool_artifact",
] as const;

export const PRODUCT_ARTIFACT_AVAILABILITIES = [
  "available",
  "cleaned",
  "invalid",
  "too_large",
] as const;

export const PRODUCT_ARTIFACT_PREVIEW_KINDS = [
  "text",
  "raster_image",
  "download_only",
  "unavailable",
] as const;

export const PRODUCT_DIFF_OPS = [
  "create",
  "update",
  "delete",
  "modified",
  "unknown",
] as const;

export const PRODUCT_DIFF_SOURCES = ["run", "git"] as const;

export const MAX_PRODUCT_SESSION_PAGE_LIMIT = 200;

export const PRODUCT_TRANSCRIPT_STATUSES = ["complete", "partial"] as const;

export const PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES = [
  "missing_run_mapping",
  "runtime_run_missing",
  "runtime_state_unavailable",
  "runtime_identity_mismatch",
  "missing_event_range",
  "corrupt_event",
  "corrupt_artifact",
  "cleaned_history",
  "response_limit_reached",
  "unknown_event_type",
] as const;

export const MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS = 64;

export const M1_MIGRATION_DISPOSITIONS = [
  "applied",
  "already_applied",
] as const;

export const M1_MIGRATION_ISSUE_CODES = [
  "invalid_workspace",
  "missing_workspace",
  "invalid_runtime_hint",
  "ambiguous_runtime_binding",
  "runtime_binding_not_found",
  "invalid_preference_reference",
  "preference_write_conflict",
] as const;

export const PRODUCT_ERROR_CODES = [
  "product_not_found",
  "product_invalid_input",
  "product_store_unavailable",
  "product_session_active",
  "product_session_workspace_mismatch",
  "product_session_resume_conflict",
  "product_session_runtime_state_missing",
  "product_session_runtime_state_corrupt",
  "product_binding_corrupt",
  "product_revision_conflict",
  "product_memory_invalid_slug",
  "product_memory_not_found",
  "product_memory_conflict",
  "project_trust_required",
  "migration_idempotency_conflict",
  "product_control_conflict",
  "product_control_rejected",
  "product_fork_conflict",
  "product_fork_source_invalid",
  "review_target_unavailable",
  "review_conflict",
  "review_unavailable",
  "product_storage_failure",
] as const;

export const PRODUCT_CONTROL_KINDS = ["steer", "followup"] as const;

export const PRODUCT_CONTROL_STATUSES = [
  "pending",
  "accepted",
  "applied",
  "dropped",
  "abandoned",
  "revoked",
] as const;

export const PRODUCT_ATTACHMENT_AVAILABILITIES = [
  "available",
  "missing",
  "corrupt",
  "expired",
] as const;

export const PRODUCT_ATTACHMENT_STATUSES = ["staged", "referenced", "expired"] as const;

export const MAX_MESSAGE_ATTACHMENTS = 32;

export const PRODUCT_MESSAGE_DELIVERIES = ["successor", "current_run"] as const;

export const PRODUCT_MESSAGE_STATUSES = [
  "queued",
  "intervention_requested",
  "applied_current_run",
  "claimed_successor",
  "needs_attention",
  "revoked",
] as const;

export const MAX_PENDING_PRODUCT_MESSAGES = 64;
/** Longest search term the server accepts, in UTF-8 bytes. */

export const MAX_PRODUCT_SEARCH_QUERY_BYTES = 128;
/** Search page bounds the server accepts. */

export const MAX_PRODUCT_SEARCH_LIMIT = 100;

/**
 * One session-scoped message search hit (R7 step 1). The excerpt is bounded and
 * already redacted server-side, so it is rendered verbatim rather than
 * re-truncated or re-highlighted.
 */

export const PRODUCT_SEARCH_SOURCES = ["message", "trace"] as const;

// ---- Wire contract types: aliases of the generated OpenAPI schemas ----

export type ProductWorkspaceId = Schema<"ProductWorkspaceId">;
export type ProductSessionId = Schema<"ProductSessionId">;
export type ProductForkId = Schema<"ProductForkId">;
export type ProductReviewId = Schema<"ProductReviewId">;
export type ProductProviderProfileId = Schema<"ProductProviderProfileId">;
export type ProductMigrationReceiptId = Schema<"ProductMigrationReceiptId">;
export type ProductSessionStatus = Schema<"ProductSessionStatus">;
export type ProductSessionOutcome = Schema<"ProductSessionOutcome">;
export type ProductProviderType = Schema<"ProductProviderType">;
export type ProductThemePreference = Schema<"ProductThemePreference">;
export type ProductApprovalPreference = Schema<"ProductApprovalPreference">;
export type ProductReasoningPreference = Schema<"ProductReasoningPreference">;
export type ProductWorkspaceKind = Schema<"ProductWorkspaceKind">;
export type ProductWorkspace = Schema<"ProductWorkspace">;
export type ProductRuntimeBinding = Schema<"ProductRuntimeBinding">;
export type ProductSession = Schema<"ProductSession">;
export type ProductConnectionStatus = Schema<"ProductConnectionStatus">;
export type ProductStoreStatus = Schema<"ProductStoreStatus">;
export type ProductResumeHealthStatus = Schema<"ProductResumeHealthStatus">;
export type ProductExecutionAdapter = Schema<"ProductExecutionAdapter">;
export type ProductExecutionWorkspaceKind = Schema<"ProductExecutionWorkspaceKind">;
export type ProductExecutionCapabilities = Schema<"ProductExecutionCapabilities">;
export type ProductExecutionEnvironmentInfo = Schema<"ProductExecutionEnvironmentInfo">;
export type ProductAgentRuntimeInfo = Schema<"ProductAgentRuntimeInfo">;
export type ProductResumeHealth = Schema<"ProductResumeHealth">;
export type ProductRuntimeInfo = Schema<"ProductRuntimeInfo">;
export type ProductReviewTargetKind = Schema<"ReviewTargetKind">;
export type ProductReviewStatus = Schema<"ProductReviewStatus">;
export type ProductReviewConclusion = Schema<"ReviewConclusion">;
export type ProductReviewSeverity = Schema<"ReviewSeverity">;
export type ProductReviewConfidence = Schema<"ReviewConfidence">;
export type ProductReviewLocationStatus = Schema<"ReviewLocationStatus">;
export type ProductReviewTargetSpec = Schema<"ReviewTargetSpec">;
export type ProductReviewTargetSummary = Schema<"ReviewTargetSummary">;
export type ProductReviewLocation = Schema<"ReviewLocation">;
export type ProductReviewEvidence = Schema<"ReviewEvidence">;
export type ProductReviewFinding = Schema<"ReviewFinding">;
export type ProductReviewStats = Schema<"ReviewStats">;
export type ProductReviewUnchecked = Schema<"ReviewUnchecked">;
export type ProductReviewResult = Schema<"ReviewResult">;
export type ProductReview = Schema<"ProductReview">;
export type ProductReviewsResponse = Schema<"ProductReviewsResponse">;
export type ProductReviewFindingPageItem = Schema<"ProductReviewFinding">;
export type ProductReviewFindingsResponse = Schema<"ProductReviewFindingsResponse">;
export type CreateProductReviewRequest = Schema<"CreateProductReviewRequest">;
export type ProductSessionRunBinding = Schema<"ProductSessionRunBinding">;
export type ProductFork = Schema<"ProductFork">;
export type ProductProviderCredentialSource = Schema<"ProductProviderCredentialSource">;
export type ProductProviderProfile = Schema<"ProductProviderProfile">;
export type ProductProviderSelection = Schema<"ProductProviderSelection">;
export type ProductSessionModelConfig = Schema<"ProductSessionModelConfig">;
export type UpdateProductSessionModelConfigRequest = Schema<"UpdateProductSessionModelConfigRequest">;
export type ProductModelDescriptor = Schema<"ProductModelDescriptor">;
export type ProductProviderModelsResponse = Schema<"ProductProviderModelsResponse">;
export type ProductSessionRunModelView = Schema<"ProductSessionRunModelView">;
export type ProductSessionRunModelsResponse = Schema<"ProductSessionRunModelsResponse">;
export type ProductPricingAvailability = Schema<"ProductPricingAvailability">;
export type ProductUsage = Schema<"ProductUsage">;
export type ProductCostBreakdown = Schema<"ProductCostBreakdown">;
export type ProductContextOccupancy = Schema<"ProductContextOccupancy">;
export type ProductRunUsage = Schema<"ProductRunUsage">;
export type ProductSessionUsageResponse = Schema<"ProductSessionUsageResponse">;
export type ProductSessionCompaction = Schema<"ProductSessionCompaction">;
export type ProductFileKind = Schema<"ProductFileKind">;
export type ProductFileEntry = Schema<"ProductFileEntry">;
export type ProductFilesResponse = Schema<"ProductFilesResponse">;
export type ProductImageMetadata = Schema<"ProductImageMetadata">;
export type ProductFileContentEnvelope = Schema<"ProductFileContentEnvelope">;
export type ProductArtifactSourceKind = Schema<"ProductArtifactSourceKind">;
export type ProductArtifactAvailability = Schema<"ProductArtifactAvailability">;
export type ProductArtifactPreviewKind = Schema<"ProductArtifactPreviewKind">;
export type ProductArtifactView = Schema<"ProductArtifactView">;
export type ProductArtifactsResponse = Schema<"ProductArtifactsResponse">;
export type ProductArtifactContentEnvelope = Schema<"ProductArtifactContentEnvelope">;
export type ProductDiffOp = Schema<"ProductDiffOp">;
export type ProductDiffSource = Schema<"ProductDiffSource">;
export type ProductDiffEntry = Schema<"ProductDiffEntry">;
export type ProductSessionDiffResponse = Schema<"ProductSessionDiffResponse">;
export type ProductPreferences = Schema<"ProductPreferences">;
export type CreateProductWorkspaceRequest = Schema<"CreateProductWorkspaceRequest">;
export type CreateProductSessionRequest = Schema<"CreateProductSessionRequest">;
export type CreateProductForkRequest = Schema<"CreateProductForkRequest">;
export type ProductForkResponse = Schema<"ProductForkResponse">;
export type ProductForksResponse = Schema<"ProductForksResponse">;
export type UpdateProductSessionRequest = Schema<"UpdateProductSessionRequest">;
export type CreateProductProviderProfileRequest = Schema<"CreateProductProviderProfileRequest">;
export type UpdateProductProviderProfileRequest = Schema<"UpdateProductProviderProfileRequest">;
export type OnboardProductProviderRequest = Schema<"OnboardProductProviderRequest">;
export type ProductProviderOnboardingProbe = Schema<"ProductProviderOnboardingProbe">;
export type ProductProviderOnboardingReceipt = Schema<"ProductProviderOnboardingReceipt">;
export type UpdateProductPreferencesRequest = Schema<"UpdateProductPreferencesRequest">;
export type ProductWorkspacesResponse = Schema<"ProductWorkspacesResponse">;
export type ProductSessionsResponse = Schema<"ProductSessionsResponse">;
export type ProductProviderProfilesResponse = Schema<"ProductProviderProfilesResponse">;
export type ProductTranscriptStatus = Schema<"ProductTranscriptStatus">;
export type ProductTranscriptPartialReasonCode = Schema<"ProductTranscriptPartialReasonCode">;
export type ProductTranscriptPartialReason = Schema<"ProductTranscriptPartialReason">;
export type ProductTranscriptFallbackSource = Schema<"ProductTranscriptFallbackSource">;
export type ProductTranscriptFallback = Schema<"ProductTranscriptFallback">;
export type ProductTranscriptRunSegment = Schema<"ProductTranscriptRunSegment">;
export type ProductTranscriptResponse = Schema<"ProductTranscriptResponse">;
export type ProductAuthorizationOutcome = Schema<"ProductAuthorizationOutcome">;
export type ProductAuthorizationRecord = Schema<"ProductAuthorizationRecord">;
export type ProductAuthorizationsResponse = Schema<"ProductAuthorizationsResponse">;
export type CreateProductPreviewRequest = Schema<"CreateProductPreviewRequest">;
export type ProductPreviewSession = Schema<"ProductPreviewSession">;
export type M1BrowserMigrationSource = Schema<"M1BrowserMigrationSource">;
export type M1WorkspaceImport = Schema<"M1WorkspaceImport">;
export type M1SessionImport = Schema<"M1SessionImport">;
export type M1ProviderProfileImport = Schema<"M1ProviderProfileImport">;
export type M1ProviderSelectionImport = Schema<"M1ProviderSelectionImport">;
export type M1SafePreferencesImport = Schema<"M1SafePreferencesImport">;
export type M1BrowserMigrationRequest = Schema<"M1BrowserMigrationRequest">;
export type M1WorkspaceIdMapping = Schema<"M1WorkspaceIdMapping">;
export type M1SessionIdMapping = Schema<"M1SessionIdMapping">;
export type M1ProviderProfileIdMapping = Schema<"M1ProviderProfileIdMapping">;
export type M1MigrationDisposition = Schema<"M1MigrationDisposition">;
export type M1MigrationIssueCode = Schema<"M1MigrationIssueCode">;
export type M1MigrationIssue = Schema<"M1MigrationIssue">;
export type M1BrowserMigrationResponse = Schema<"M1BrowserMigrationResponse">;
export type ProductErrorCode = Schema<"ProductErrorCode">;
export type ApiErrorResponse = Schema<"ApiErrorResponse">;
export type ProductToolExecutionStatus = Schema<"ToolExecutionStatus">;
export type ProductToolRiskLevel = Schema<"ToolRiskLevel">;
export type ProductToolExecutionMetadata = Schema<"ToolExecutionMetadata">;
export type ProductToolResult = Schema<"ToolResult">;
export type ProductPromptBuildMetadata = Schema<"PromptBuildMetadata">;
export type ProductStreamEvent = Schema<"StreamEvent">;
export type ProductControlId = Schema<"ProductControlId">;
export type ProductControlKind = Schema<"ProductControlKind">;
export type ProductControlStatus = Schema<"ProductControlStatus">;
export type ProductControl = Schema<"ProductControl">;
export type CreateProductControlRequest = Schema<"CreateProductControlRequest">;
export type ProductAttachmentAvailability = Schema<"ProductAttachmentAvailability">;
export type ProductAttachmentStatus = Schema<"ProductAttachmentStatus">;
export type ProductMessageAttachmentRef = Schema<"ProductMessageAttachmentRef">;
export type ProductMessageAttachmentRequest = Schema<"ProductMessageAttachmentRequest">;
export type ProductAttachmentUpload = Schema<"ProductAttachmentUploadResponse">;
export type ProductControlsResponse = Schema<"ProductControlsResponse">;
export type ProductMessageDelivery = Schema<"ProductMessageDelivery">;
export type ProductMessageStatus = Schema<"ProductMessageStatus">;
export type ProductMessage = Schema<"ProductMessage">;
export type CreateProductMessageRequest = Schema<"CreateProductMessageRequest">;
export type ProductMessagesResponse = Schema<"ProductMessagesResponse">;
export type PromoteProductMessageRequest = Schema<"PromoteProductMessageRequest">;
export type ReorderProductMessagesRequest = Schema<"ReorderProductMessagesRequest">;
export type ProductQueueResponse = Schema<"ProductQueueResponse">;
export type ProductMessageSearchHit = Schema<"ProductMessageSearchHit">;
export type ProductMessagesSearchResponse = Schema<"ProductMessagesSearchResponse">;
export type ProductSearchSource = Schema<"ProductSearchSource">;
export type ProductSearchHit = Schema<"ProductSearchHit">;
export type ProductSearchResponse = Schema<"ProductSearchResponse">;
export type ProductControlStatusFilter = Schema<"ProductControlStatusFilter">;
export type ProductJobStreamEvent = Schema<"JobStreamEvent">;

// ---- Const arrays asserted exact against their generated unions ----
type _AssertPRODUCT_SESSION_STATUSESExact = AssertExact<
  [(typeof PRODUCT_SESSION_STATUSES)[number]] extends [Schema<"ProductSessionStatus">]
    ? [Schema<"ProductSessionStatus">] extends [(typeof PRODUCT_SESSION_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_SESSION_OUTCOMESExact = AssertExact<
  [(typeof PRODUCT_SESSION_OUTCOMES)[number]] extends [Schema<"ProductSessionOutcome">]
    ? [Schema<"ProductSessionOutcome">] extends [(typeof PRODUCT_SESSION_OUTCOMES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_PROVIDER_TYPESExact = AssertExact<
  [(typeof PRODUCT_PROVIDER_TYPES)[number]] extends [Schema<"ProductProviderType">]
    ? [Schema<"ProductProviderType">] extends [(typeof PRODUCT_PROVIDER_TYPES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_THEME_PREFERENCESExact = AssertExact<
  [(typeof PRODUCT_THEME_PREFERENCES)[number]] extends [Schema<"ProductThemePreference">]
    ? [Schema<"ProductThemePreference">] extends [(typeof PRODUCT_THEME_PREFERENCES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_APPROVAL_PREFERENCESExact = AssertExact<
  [(typeof PRODUCT_APPROVAL_PREFERENCES)[number]] extends [Schema<"ProductApprovalPreference">]
    ? [Schema<"ProductApprovalPreference">] extends [(typeof PRODUCT_APPROVAL_PREFERENCES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REASONING_PREFERENCESExact = AssertExact<
  [(typeof PRODUCT_REASONING_PREFERENCES)[number]] extends [Schema<"ProductReasoningPreference">]
    ? [Schema<"ProductReasoningPreference">] extends [(typeof PRODUCT_REASONING_PREFERENCES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_WORKSPACE_KINDSExact = AssertExact<
  [(typeof PRODUCT_WORKSPACE_KINDS)[number]] extends [Schema<"ProductWorkspaceKind">]
    ? [Schema<"ProductWorkspaceKind">] extends [(typeof PRODUCT_WORKSPACE_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_CONNECTION_STATUSESExact = AssertExact<
  [(typeof PRODUCT_CONNECTION_STATUSES)[number]] extends [Schema<"ProductConnectionStatus">]
    ? [Schema<"ProductConnectionStatus">] extends [(typeof PRODUCT_CONNECTION_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_STORE_STATUSESExact = AssertExact<
  [(typeof PRODUCT_STORE_STATUSES)[number]] extends [Schema<"ProductStoreStatus">]
    ? [Schema<"ProductStoreStatus">] extends [(typeof PRODUCT_STORE_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_RESUME_HEALTH_STATUSESExact = AssertExact<
  [(typeof PRODUCT_RESUME_HEALTH_STATUSES)[number]] extends [Schema<"ProductResumeHealthStatus">]
    ? [Schema<"ProductResumeHealthStatus">] extends [(typeof PRODUCT_RESUME_HEALTH_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_EXECUTION_ADAPTERSExact = AssertExact<
  [(typeof PRODUCT_EXECUTION_ADAPTERS)[number]] extends [Schema<"ProductExecutionAdapter">]
    ? [Schema<"ProductExecutionAdapter">] extends [(typeof PRODUCT_EXECUTION_ADAPTERS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_EXECUTION_WORKSPACE_KINDSExact = AssertExact<
  [(typeof PRODUCT_EXECUTION_WORKSPACE_KINDS)[number]] extends [Schema<"ProductExecutionWorkspaceKind">]
    ? [Schema<"ProductExecutionWorkspaceKind">] extends [(typeof PRODUCT_EXECUTION_WORKSPACE_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REVIEW_TARGET_KINDSExact = AssertExact<
  [(typeof PRODUCT_REVIEW_TARGET_KINDS)[number]] extends [Schema<"ReviewTargetKind">]
    ? [Schema<"ReviewTargetKind">] extends [(typeof PRODUCT_REVIEW_TARGET_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REVIEW_STATUSESExact = AssertExact<
  [(typeof PRODUCT_REVIEW_STATUSES)[number]] extends [Schema<"ProductReviewStatus">]
    ? [Schema<"ProductReviewStatus">] extends [(typeof PRODUCT_REVIEW_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REVIEW_CONCLUSIONSExact = AssertExact<
  [(typeof PRODUCT_REVIEW_CONCLUSIONS)[number]] extends [Schema<"ReviewConclusion">]
    ? [Schema<"ReviewConclusion">] extends [(typeof PRODUCT_REVIEW_CONCLUSIONS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REVIEW_SEVERITIESExact = AssertExact<
  [(typeof PRODUCT_REVIEW_SEVERITIES)[number]] extends [Schema<"ReviewSeverity">]
    ? [Schema<"ReviewSeverity">] extends [(typeof PRODUCT_REVIEW_SEVERITIES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REVIEW_CONFIDENCESExact = AssertExact<
  [(typeof PRODUCT_REVIEW_CONFIDENCES)[number]] extends [Schema<"ReviewConfidence">]
    ? [Schema<"ReviewConfidence">] extends [(typeof PRODUCT_REVIEW_CONFIDENCES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_REVIEW_LOCATION_STATUSESExact = AssertExact<
  [(typeof PRODUCT_REVIEW_LOCATION_STATUSES)[number]] extends [Schema<"ReviewLocationStatus">]
    ? [Schema<"ReviewLocationStatus">] extends [(typeof PRODUCT_REVIEW_LOCATION_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_PRICING_AVAILABILITIESExact = AssertExact<
  [(typeof PRODUCT_PRICING_AVAILABILITIES)[number]] extends [Schema<"ProductPricingAvailability">]
    ? [Schema<"ProductPricingAvailability">] extends [(typeof PRODUCT_PRICING_AVAILABILITIES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_FILE_KINDSExact = AssertExact<
  [(typeof PRODUCT_FILE_KINDS)[number]] extends [Schema<"ProductFileKind">]
    ? [Schema<"ProductFileKind">] extends [(typeof PRODUCT_FILE_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_ARTIFACT_SOURCE_KINDSExact = AssertExact<
  [(typeof PRODUCT_ARTIFACT_SOURCE_KINDS)[number]] extends [Schema<"ProductArtifactSourceKind">]
    ? [Schema<"ProductArtifactSourceKind">] extends [(typeof PRODUCT_ARTIFACT_SOURCE_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_ARTIFACT_AVAILABILITIESExact = AssertExact<
  [(typeof PRODUCT_ARTIFACT_AVAILABILITIES)[number]] extends [Schema<"ProductArtifactAvailability">]
    ? [Schema<"ProductArtifactAvailability">] extends [(typeof PRODUCT_ARTIFACT_AVAILABILITIES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_ARTIFACT_PREVIEW_KINDSExact = AssertExact<
  [(typeof PRODUCT_ARTIFACT_PREVIEW_KINDS)[number]] extends [Schema<"ProductArtifactPreviewKind">]
    ? [Schema<"ProductArtifactPreviewKind">] extends [(typeof PRODUCT_ARTIFACT_PREVIEW_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_DIFF_OPSExact = AssertExact<
  [(typeof PRODUCT_DIFF_OPS)[number]] extends [Schema<"ProductDiffOp">]
    ? [Schema<"ProductDiffOp">] extends [(typeof PRODUCT_DIFF_OPS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_DIFF_SOURCESExact = AssertExact<
  [(typeof PRODUCT_DIFF_SOURCES)[number]] extends [Schema<"ProductDiffSource">]
    ? [Schema<"ProductDiffSource">] extends [(typeof PRODUCT_DIFF_SOURCES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_TRANSCRIPT_STATUSESExact = AssertExact<
  [(typeof PRODUCT_TRANSCRIPT_STATUSES)[number]] extends [Schema<"ProductTranscriptStatus">]
    ? [Schema<"ProductTranscriptStatus">] extends [(typeof PRODUCT_TRANSCRIPT_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_TRANSCRIPT_PARTIAL_REASON_CODESExact = AssertExact<
  [(typeof PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES)[number]] extends [Schema<"ProductTranscriptPartialReasonCode">]
    ? [Schema<"ProductTranscriptPartialReasonCode">] extends [(typeof PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES)[number]]
      ? true
      : false
    : false
>;

type _AssertM1_MIGRATION_DISPOSITIONSExact = AssertExact<
  [(typeof M1_MIGRATION_DISPOSITIONS)[number]] extends [Schema<"M1MigrationDisposition">]
    ? [Schema<"M1MigrationDisposition">] extends [(typeof M1_MIGRATION_DISPOSITIONS)[number]]
      ? true
      : false
    : false
>;

type _AssertM1_MIGRATION_ISSUE_CODESExact = AssertExact<
  [(typeof M1_MIGRATION_ISSUE_CODES)[number]] extends [Schema<"M1MigrationIssueCode">]
    ? [Schema<"M1MigrationIssueCode">] extends [(typeof M1_MIGRATION_ISSUE_CODES)[number]]
      ? true
      : false
    : false
>;

// PRODUCT_ERROR_CODES is the client's known-code subset, not the enum: the
// server may answer codes a newer build introduced, so only ⊆ is asserted.
type _AssertPRODUCT_ERROR_CODESSubset = AssertExact<
  [(typeof PRODUCT_ERROR_CODES)[number]] extends [Schema<"ProductErrorCode">]
    ? true
    : false
>;

type _AssertPRODUCT_CONTROL_KINDSExact = AssertExact<
  [(typeof PRODUCT_CONTROL_KINDS)[number]] extends [Schema<"ProductControlKind">]
    ? [Schema<"ProductControlKind">] extends [(typeof PRODUCT_CONTROL_KINDS)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_CONTROL_STATUSESExact = AssertExact<
  [(typeof PRODUCT_CONTROL_STATUSES)[number]] extends [Schema<"ProductControlStatus">]
    ? [Schema<"ProductControlStatus">] extends [(typeof PRODUCT_CONTROL_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_ATTACHMENT_AVAILABILITIESExact = AssertExact<
  [(typeof PRODUCT_ATTACHMENT_AVAILABILITIES)[number]] extends [Schema<"ProductAttachmentAvailability">]
    ? [Schema<"ProductAttachmentAvailability">] extends [(typeof PRODUCT_ATTACHMENT_AVAILABILITIES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_ATTACHMENT_STATUSESExact = AssertExact<
  [(typeof PRODUCT_ATTACHMENT_STATUSES)[number]] extends [Schema<"ProductAttachmentStatus">]
    ? [Schema<"ProductAttachmentStatus">] extends [(typeof PRODUCT_ATTACHMENT_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_MESSAGE_DELIVERIESExact = AssertExact<
  [(typeof PRODUCT_MESSAGE_DELIVERIES)[number]] extends [Schema<"ProductMessageDelivery">]
    ? [Schema<"ProductMessageDelivery">] extends [(typeof PRODUCT_MESSAGE_DELIVERIES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_MESSAGE_STATUSESExact = AssertExact<
  [(typeof PRODUCT_MESSAGE_STATUSES)[number]] extends [Schema<"ProductMessageStatus">]
    ? [Schema<"ProductMessageStatus">] extends [(typeof PRODUCT_MESSAGE_STATUSES)[number]]
      ? true
      : false
    : false
>;

type _AssertPRODUCT_SEARCH_SOURCESExact = AssertExact<
  [(typeof PRODUCT_SEARCH_SOURCES)[number]] extends [Schema<"ProductSearchSource">]
    ? [Schema<"ProductSearchSource">] extends [(typeof PRODUCT_SEARCH_SOURCES)[number]]
      ? true
      : false
    : false
>;

// ---- Boundary guards: runtime checks for untrusted-byte boundaries ----

export type ProductPromptPruning = PromptPruningFacts;

export class ProductApiSchemaError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProductApiSchemaError";
  }
}

export function isSafeEnvironmentVariableName(value: string): boolean {
  return /^[A-Z_][A-Z0-9_]{0,255}$/.test(value);
}

export function assertSafeProductProviderConfiguration(
  providerType: ProductProviderType,
  apiBase: string,
  apiKeyEnv: string | undefined,
  path = "provider",
): void {
  if (
    apiKeyEnv !== undefined &&
    !isSafeEnvironmentVariableName(apiKeyEnv)
  ) {
    schemaError(`${path}.api_key_env`, "a valid environment variable name");
  }

  if (providerType === "fake") {
    if (apiKeyEnv !== undefined) {
      schemaError(`${path}.api_key_env`, "absent for the fake provider");
    }
    if (apiBase.trim() !== "") {
      schemaError(`${path}.api_base`, "empty for the fake provider");
    }
    return;
  }

  let parsed: URL;
  try {
    parsed = new URL(apiBase);
  } catch {
    return schemaError(`${path}.api_base`, "an absolute HTTP(S) URL");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    schemaError(`${path}.api_base`, "an HTTP(S) URL");
  }
  if (parsed.username || parsed.password) {
    schemaError(`${path}.api_base`, "a URL without user information");
  }
  if (parsed.search || parsed.hash) {
    schemaError(`${path}.api_base`, "a URL without query parameters or fragments");
  }
}

export function isKnownProductCompactionMode(
  mode: string,
): mode is (typeof PRODUCT_COMPACTION_MODES)[number] {
  return (PRODUCT_COMPACTION_MODES as readonly string[]).includes(mode);
}

export function parseM1BrowserMigrationRequest(
  value: unknown,
  path = "M1 browser migration request",
): M1BrowserMigrationRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source",
      "source_schema_version",
      "idempotency_key",
      "workspaces",
      "sessions",
      "provider_profiles",
      "safe_preferences",
    ],
    path,
  );
  return {
    source: expectEnum(
      record.source,
      ["web_m1_local_storage"] as const,
      `${path}.source`,
    ),
    source_schema_version: expectInteger(
      record.source_schema_version,
      `${path}.source_schema_version`,
      { min: 1 },
    ),
    idempotency_key: expectM1MigrationIdempotencyKey(
      record.idempotency_key,
      `${path}.idempotency_key`,
    ),
    workspaces: expectArray(
      record.workspaces,
      `${path}.workspaces`,
      parseM1WorkspaceImport,
      MAX_PRODUCT_WORKSPACES,
    ),
    sessions: expectArray(
      record.sessions,
      `${path}.sessions`,
      parseM1SessionImport,
      MAX_PRODUCT_SESSIONS,
    ),
    provider_profiles: expectArray(
      record.provider_profiles,
      `${path}.provider_profiles`,
      parseM1ProviderProfileImport,
      MAX_PRODUCT_PROVIDER_PROFILES,
    ),
    safe_preferences: parseM1SafePreferencesImport(
      record.safe_preferences,
      `${path}.safe_preferences`,
    ),
  };
}

export function parseM1BrowserMigrationResponse(
  value: unknown,
): M1BrowserMigrationResponse {
  const path = "M1 browser migration response";
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source_schema_version",
      "idempotency_key",
      "receipt_id",
      "disposition",
      "workspace_mappings",
      "session_mappings",
      "provider_profile_mappings",
      "issues",
      "applied_at",
    ],
    path,
  );
  return {
    source_schema_version: expectInteger(
      record.source_schema_version,
      `${path}.source_schema_version`,
      { min: 1 },
    ),
    idempotency_key: expectString(
      record.idempotency_key,
      `${path}.idempotency_key`,
      { nonEmpty: true, maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES },
    ),
    receipt_id: expectId(record.receipt_id, `${path}.receipt_id`),
    disposition: expectEnum(
      record.disposition,
      M1_MIGRATION_DISPOSITIONS,
      `${path}.disposition`,
    ),
    workspace_mappings: expectArray(
      record.workspace_mappings,
      `${path}.workspace_mappings`,
      parseM1WorkspaceIdMapping,
      MAX_PRODUCT_WORKSPACES,
    ),
    session_mappings: expectArray(
      record.session_mappings,
      `${path}.session_mappings`,
      parseM1SessionIdMapping,
      MAX_PRODUCT_SESSIONS,
    ),
    provider_profile_mappings: expectArray(
      record.provider_profile_mappings,
      `${path}.provider_profile_mappings`,
      parseM1ProviderProfileIdMapping,
      MAX_PRODUCT_PROVIDER_PROFILES,
    ),
    issues: expectArray(record.issues, `${path}.issues`, parseM1MigrationIssue),
    applied_at: expectString(record.applied_at, `${path}.applied_at`, {
      nonEmpty: true,
    }),
  };
}

export function parseApiErrorResponse(value: unknown): ApiErrorResponse | null {
  if (!isRecord(value)) {
    return null;
  }
  if (typeof value.code !== "string" || typeof value.error !== "string") {
    return null;
  }
  return { code: value.code, error: value.error };
}

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function schemaError(path: string, expectation: string): never {
  throw new ProductApiSchemaError(`${path} must be ${expectation}`);
}

function expectRecord(value: unknown, path: string): UnknownRecord {
  if (!isRecord(value)) {
    return schemaError(path, "an object");
  }
  return value;
}

function expectOnlyKeys(
  value: UnknownRecord,
  allowed: readonly string[],
  path: string,
): void {
  const allowedKeys = new Set(allowed);
  const unknown = Object.keys(value).find((key) => !allowedKeys.has(key));
  if (unknown) {
    schemaError(`${path}.${unknown}`, "a known field");
  }
}

function expectString(
  value: unknown,
  path: string,
  options: {
    nonEmpty?: boolean;
    maxBytes?: number;
    noControlCharacters?: boolean;
  } = {},
): string {
  if (typeof value !== "string") {
    return schemaError(path, "a string");
  }
  if (options.nonEmpty && value.trim().length === 0) {
    return schemaError(path, "a non-empty string");
  }
  if (
    options.maxBytes !== undefined &&
    new TextEncoder().encode(value).length > options.maxBytes
  ) {
    return schemaError(path, `at most ${options.maxBytes} UTF-8 bytes`);
  }
  if (
    options.noControlCharacters &&
    /[\u0000-\u001f\u007f-\u009f]/u.test(value)
  ) {
    return schemaError(path, "free of control characters");
  }
  return value;
}

function optionalString(
  value: UnknownRecord,
  key: string,
  path: string,
  options: {
    nonEmpty?: boolean;
    maxBytes?: number;
    noControlCharacters?: boolean;
  } = {},
): string | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectString(candidate, `${path}.${key}`, options);
}

const RFC3339_TIMESTAMP_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|[+-](\d{2}):(\d{2}))$/u;

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

function expectRfc3339Timestamp(value: unknown, path: string): string {
  const timestamp = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControlCharacters: true,
  });
  const match = RFC3339_TIMESTAMP_PATTERN.exec(timestamp);
  if (match === null) {
    return schemaError(path, "an RFC3339 timestamp");
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  const offsetHour = match[7] === undefined ? 0 : Number(match[7]);
  const offsetMinute = match[8] === undefined ? 0 : Number(match[8]);
  const monthDays = [
    31,
    isLeapYear(year) ? 29 : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > monthDays[month - 1]! ||
    hour > 23 ||
    minute > 59 ||
    second > 60 ||
    offsetHour > 23 ||
    offsetMinute > 59
  ) {
    return schemaError(path, "an RFC3339 timestamp");
  }
  return timestamp;
}

function expectM1MigrationIdempotencyKey(
  value: unknown,
  path: string,
): string {
  const key = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES,
  });
  if (!/^[A-Za-z0-9_.:-]+$/u.test(key)) {
    return schemaError(path, "a valid migration idempotency key");
  }
  return key;
}

function expectBoolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    return schemaError(path, "a boolean");
  }
  return value;
}

function optionalBoolean(
  value: UnknownRecord,
  key: string,
  path: string,
): boolean | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectBoolean(candidate, `${path}.${key}`);
}

function expectInteger(
  value: unknown,
  path: string,
  options: { min?: number; max?: number } = {},
): number {
  if (!Number.isSafeInteger(value)) {
    return schemaError(path, "a safe integer");
  }
  const numberValue = Number(value);
  if (options.min !== undefined && numberValue < options.min) {
    return schemaError(path, `at least ${options.min}`);
  }
  if (options.max !== undefined && numberValue > options.max) {
    return schemaError(path, `at most ${options.max}`);
  }
  return numberValue;
}

function expectArray<T>(
  value: unknown,
  path: string,
  parseItem: (item: unknown, itemPath: string) => T,
  maxLength?: number,
): T[] {
  if (!Array.isArray(value)) {
    return schemaError(path, "an array");
  }
  if (maxLength !== undefined && value.length > maxLength) {
    return schemaError(path, `an array with at most ${maxLength} items`);
  }
  return value.map((item, index) => parseItem(item, `${path}[${index}]`));
}

function isEnumValue<const T extends readonly string[]>(
  value: unknown,
  values: T,
): value is T[number] {
  return (
    typeof value === "string" &&
    values.some((candidate) => candidate === value)
  );
}

function expectEnum<const T extends readonly string[]>(
  value: unknown,
  values: T,
  path: string,
): T[number] {
  if (!isEnumValue(value, values)) {
    return schemaError(path, `one of ${values.join(", ")}`);
  }
  return value;
}

function assignOptional<T extends object, K extends keyof T>(
  target: T,
  key: K,
  value: T[K] | undefined,
): void {
  if (value !== undefined) {
    target[key] = value;
  }
}

function expectId(value: unknown, path: string): string {
  return expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControlCharacters: true,
  });
}

const PRODUCT_COMPACTION_MODES = [
  "none",
  "deterministic",
  "model_generated",
  "automatic",
  "degraded",
  "disabled",
] as const;

function parseM1WorkspaceImport(value: unknown, path: string): M1WorkspaceImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["source_id", "root", "kind", "display_name", "pinned", "last_opened_at"],
    path,
  );
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    root: expectString(record.root, `${path}.root`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
      noControlCharacters: true,
    }),
    kind: expectEnum(record.kind, PRODUCT_WORKSPACE_KINDS, `${path}.kind`),
    display_name: expectString(record.display_name, `${path}.display_name`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    pinned: expectBoolean(record.pinned, `${path}.pinned`),
    last_opened_at: expectRfc3339Timestamp(
      record.last_opened_at,
      `${path}.last_opened_at`,
    ),
  };
}

function parseM1SessionImport(value: unknown, path: string): M1SessionImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source_id",
      "source_workspace_id",
      "title",
      "created_at",
      "updated_at",
      "legacy_active_job_id",
      "legacy_active_run_id",
      "legacy_resumed_from_run_id",
      "legacy_has_durable_turn",
    ],
    path,
  );
  const session: M1SessionImport = {
    source_id: expectId(record.source_id, `${path}.source_id`),
    source_workspace_id: expectId(
      record.source_workspace_id,
      `${path}.source_workspace_id`,
    ),
    title: expectString(record.title, `${path}.title`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    created_at: expectRfc3339Timestamp(
      record.created_at,
      `${path}.created_at`,
    ),
    updated_at: expectRfc3339Timestamp(
      record.updated_at,
      `${path}.updated_at`,
    ),
  };
  assignOptional(
    session,
    "legacy_active_job_id",
    optionalString(record, "legacy_active_job_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    session,
    "legacy_active_run_id",
    optionalString(record, "legacy_active_run_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    session,
    "legacy_resumed_from_run_id",
    optionalString(record, "legacy_resumed_from_run_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    session,
    "legacy_has_durable_turn",
    optionalBoolean(record, "legacy_has_durable_turn", path),
  );
  return session;
}

function parseM1ProviderProfileImport(
  value: unknown,
  path: string,
): M1ProviderProfileImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source_id",
      "label",
      "provider_type",
      "api_base",
      "api_key_env",
      "default_model",
      "updated_at",
    ],
    path,
  );
  const providerType = expectEnum(
    record.provider_type,
    PRODUCT_PROVIDER_TYPES,
    `${path}.provider_type`,
  );
  const apiBase = expectString(record.api_base, `${path}.api_base`, {
    maxBytes: MAX_PRODUCT_API_BASE_BYTES,
  });
  const apiKeyEnv = optionalString(record, "api_key_env", path, {
    nonEmpty: true,
    maxBytes: 256,
  });
  assertSafeProductProviderConfiguration(
    providerType,
    apiBase,
    apiKeyEnv,
    path,
  );
  const profile: M1ProviderProfileImport = {
    source_id: expectId(record.source_id, `${path}.source_id`),
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    provider_type: providerType,
    api_base: apiBase,
    updated_at: expectRfc3339Timestamp(
      record.updated_at,
      `${path}.updated_at`,
    ),
  };
  assignOptional(profile, "api_key_env", apiKeyEnv);
  assignOptional(
    profile,
    "default_model",
    optionalString(record, "default_model", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return profile;
}

function parseM1ProviderSelectionImport(
  value: unknown,
  path: string,
): M1ProviderSelectionImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["source_profile_id", "model", "approval", "max_steps"],
    path,
  );
  const selection: M1ProviderSelectionImport = {
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    approval: expectEnum(
      record.approval,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.approval`,
    ),
    max_steps: expectInteger(record.max_steps, `${path}.max_steps`, {
      min: 1,
      max: 4_096,
    }),
  };
  assignOptional(
    selection,
    "source_profile_id",
    optionalString(record, "source_profile_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return selection;
}

function parseM1SafePreferencesImport(
  value: unknown,
  path: string,
): M1SafePreferencesImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "theme",
      "source_active_workspace_id",
      "source_active_session_id",
      "provider_selection",
    ],
    path,
  );
  const preferences: M1SafePreferencesImport = {};
  if (record.theme !== undefined && record.theme !== null) {
    preferences.theme = expectEnum(
      record.theme,
      PRODUCT_THEME_PREFERENCES,
      `${path}.theme`,
    );
  }
  assignOptional(
    preferences,
    "source_active_workspace_id",
    optionalString(record, "source_active_workspace_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    preferences,
    "source_active_session_id",
    optionalString(record, "source_active_session_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  if (
    record.provider_selection !== undefined &&
    record.provider_selection !== null
  ) {
    preferences.provider_selection = parseM1ProviderSelectionImport(
      record.provider_selection,
      `${path}.provider_selection`,
    );
  }
  return preferences;
}

/**
 * SSE frame boundary guard. The frame's own bytes are the only untrusted
 * surface left on this path: JSON parse already happened, so the check is
 * that the value is an object whose `type` is a canonical event kind. Field
 * shape is asserted by the generated `StreamEvent` type and pinned end-to-end
 * by `tests/event_contract.rs`; re-validating every field here duplicated the
 * spec by hand.
 */
export function parseStreamEvent(
  value: unknown,
  path = "stream event",
): ProductStreamEvent {
  const record = expectRecord(value, path);
  expectEnum(record.type, STREAM_EVENT_NAMES, `${path}.type`);
  return value as ProductStreamEvent;
}

function parseM1WorkspaceIdMapping(
  value: unknown,
  path: string,
): M1WorkspaceIdMapping {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["source_id", "workspace_id"], path);
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
  };
}

function parseM1SessionIdMapping(
  value: unknown,
  path: string,
): M1SessionIdMapping {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["source_id", "product_session_id"], path);
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
  };
}

function parseM1ProviderProfileIdMapping(
  value: unknown,
  path: string,
): M1ProviderProfileIdMapping {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["source_id", "provider_profile_id"], path);
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    provider_profile_id: expectId(
      record.provider_profile_id,
      `${path}.provider_profile_id`,
    ),
  };
}

function parseM1MigrationIssue(value: unknown, path: string): M1MigrationIssue {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["code", "entity", "source_id"], path);
  const issue: M1MigrationIssue = {
    code: expectEnum(
      record.code,
      M1_MIGRATION_ISSUE_CODES,
      `${path}.code`,
    ),
    entity: expectString(record.entity, `${path}.entity`, { nonEmpty: true }),
  };
  assignOptional(
    issue,
    "source_id",
    optionalString(record, "source_id", path, { nonEmpty: true }),
  );
  return issue;
}
// ---- Semantic guards: request shapes and transcript-page invariants ----
//
// These are not field validators — the generated types pin the wire shape at
// compile time. They exist because the client depends on cross-field rules
// the server promises but the schema cannot express: a review target whose
// revision pairing matches its kind, a control body that is non-empty after
// trimming, a provider profile whose credential source is consistent with its
// provider type, and a transcript page whose ordinals, event sequences, and
// partial reasons account for every gap. A violation means the server is
// publishing a contract this build cannot reason about, so the guard refuses
// the payload instead of letting a silently inconsistent page merge into the
// transcript.

function optionalInteger(
  value: UnknownRecord,
  key: string,
  path: string,
  options: { min?: number; max?: number } = {},
): number | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectInteger(candidate, `${path}.${key}`, options);
}

function optionalArray<T>(
  value: UnknownRecord,
  key: string,
  path: string,
  parseItem: (item: unknown, itemPath: string) => T,
  maxLength?: number,
): T[] | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectArray(candidate, `${path}.${key}`, parseItem, maxLength);
}

function optionalId(
  value: UnknownRecord,
  key: string,
  path: string,
): string | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectId(candidate, `${path}.${key}`);
}

const PRODUCT_TRANSCRIPT_POSITIONLESS_GAP_REASON_CODES = new Set<
  ProductTranscriptPartialReasonCode
>([
  "runtime_state_unavailable",
  "runtime_identity_mismatch",
  "missing_event_range",
  "corrupt_event",
  "corrupt_artifact",
  "cleaned_history",
  "response_limit_reached",
]);

/**
 * Whether a reason belongs to this run ordinal, and to this runtime run when
 * both sides name one.
 */
function partialReasonNamesRun(
  reason: ProductTranscriptPartialReason,
  runOrdinal: number,
  runId?: string,
): boolean {
  if (reason.run_ordinal !== runOrdinal) {
    return false;
  }
  return runId === undefined || reason.run_id === undefined || reason.run_id === runId;
}

/**
 * Whether a withheld-row reason for this run ordinal covers a sequence step from
 * `expectedSeq` to `nextSeq`.
 *
 * The server withholds exactly the durable rows whose kind entered the canonical
 * event contract after the version this client declared, and reports the lowest
 * and highest withheld sequence as one `unknown_event_type` reason per run. Only
 * that declared reason makes a non-contiguous delivered sequence valid.
 */
function unknownEventTypeCoversStep(
  reasons: readonly ProductTranscriptPartialReason[],
  runOrdinal: number,
  expectedSeq: number,
  nextSeq: number,
  runId?: string,
): boolean {
  if (nextSeq <= expectedSeq) {
    return false;
  }
  return reasons.some(
    (reason) =>
      partialReasonNamesRun(reason, runOrdinal, runId) &&
      reason.code === "unknown_event_type" &&
      (reason.expected_seq ?? 0) <= expectedSeq &&
      (reason.observed_seq ?? 0) >= nextSeq - 1,
  );
}

function parseJobStreamEvent(
  value: unknown,
  path: string,
): ProductJobStreamEvent {
  const record = expectRecord(value, path);
  return {
    seq: expectInteger(record.seq, `${path}.seq`, { min: 1 }),
    event: parseStreamEvent(record.event, `${path}.event`),
  };
}

function parseProductSessionRunBinding(
  value: unknown,
  path: string,
): ProductSessionRunBinding {
  const record = expectRecord(value, path);
  const binding: ProductSessionRunBinding = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    ordinal: expectInteger(record.ordinal, `${path}.ordinal`, { min: 1 }),
    runtime_session_id: expectId(
      record.runtime_session_id,
      `${path}.runtime_session_id`,
    ),
    runtime_job_id: expectId(record.runtime_job_id, `${path}.runtime_job_id`),
    runtime_run_id: expectId(record.runtime_run_id, `${path}.runtime_run_id`),
    bound_at: expectString(record.bound_at, `${path}.bound_at`, {
      nonEmpty: true,
    }),
  };
  assignOptional(
    binding,
    "resumed_from_run_id",
    optionalString(record, "resumed_from_run_id", path, { nonEmpty: true }),
  );
  return binding;
}

function parseProductTranscriptPartialReason(
  value: unknown,
  path: string,
): ProductTranscriptPartialReason {
  const record = expectRecord(value, path);
  const reason: ProductTranscriptPartialReason = {
    code: expectEnum(
      record.code,
      PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES,
      `${path}.code`,
    ),
  };
  assignOptional(
    reason,
    "run_ordinal",
    optionalInteger(record, "run_ordinal", path, { min: 1 }),
  );
  assignOptional(
    reason,
    "run_id",
    optionalString(record, "run_id", path, { nonEmpty: true }),
  );
  assignOptional(
    reason,
    "expected_seq",
    optionalInteger(record, "expected_seq", path, { min: 0 }),
  );
  assignOptional(
    reason,
    "observed_seq",
    optionalInteger(record, "observed_seq", path, { min: 0 }),
  );
  return reason;
}

function parseProductTranscriptFallback(
  value: unknown,
  path: string,
): ProductTranscriptFallback {
  const record = expectRecord(value, path);
  const fallback: ProductTranscriptFallback = {
    source: expectEnum(record.source, ["report"] as const, `${path}.source`),
    status: expectString(record.status, `${path}.status`, { nonEmpty: true }),
  };
  assignOptional(fallback, "summary", optionalString(record, "summary", path));
  return fallback;
}

function parseProductTranscriptRunSegment(
  value: unknown,
  path: string,
): ProductTranscriptRunSegment {
  const record = expectRecord(value, path);
  const segment: ProductTranscriptRunSegment = {
    binding: parseProductSessionRunBinding(record.binding, `${path}.binding`),
    inherited: expectBoolean(record.inherited, `${path}.inherited`),
    run_status: expectString(record.run_status, `${path}.run_status`, {
      nonEmpty: true,
    }),
    observed_through_seq: expectInteger(
      record.observed_through_seq,
      `${path}.observed_through_seq`,
      { min: 0 },
    ),
    last_event_seq: expectInteger(
      record.last_event_seq,
      `${path}.last_event_seq`,
      { min: 0 },
    ),
    events: expectArray(record.events, `${path}.events`, parseJobStreamEvent),
  };
  assignOptional(
    segment,
    "source_product_session_id",
    optionalId(record, "source_product_session_id", path),
  );
  assignOptional(
    segment,
    "answer_aborted",
    optionalBoolean(record, "answer_aborted", path),
  );
  if (record.fallback !== undefined && record.fallback !== null) {
    segment.fallback = parseProductTranscriptFallback(
      record.fallback,
      `${path}.fallback`,
    );
  }
  return segment;
}

export function parseProductTranscriptResponse(
  value: unknown,
): ProductTranscriptResponse {
  const path = "product transcript response";
  const record = expectRecord(value, path);
  const response: ProductTranscriptResponse = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
    status: expectEnum(
      record.status,
      PRODUCT_TRANSCRIPT_STATUSES,
      `${path}.status`,
    ),
    partial_reasons: expectArray(
      record.partial_reasons,
      `${path}.partial_reasons`,
      parseProductTranscriptPartialReason,
    ),
    segments: expectArray(
      record.segments,
      `${path}.segments`,
      parseProductTranscriptRunSegment,
    ),
  };
  assignOptional(
    response,
    "next_before_ordinal",
    optionalInteger(record, "next_before_ordinal", path, { min: 1 }),
  );
  assignOptional(response, "has_more", optionalBoolean(record, "has_more", path));
  if (response.next_before_ordinal !== undefined && response.has_more === undefined) {
    schemaError(
      `${path}.has_more`,
      "present on a response that carries next_before_ordinal",
    );
  }
  if (
    response.has_more !== undefined &&
    response.has_more !== (response.next_before_ordinal !== undefined)
  ) {
    schemaError(
      `${path}.has_more`,
      "equal to whether next_before_ordinal is present",
    );
  }
  if (
    (response.status === "complete" && response.partial_reasons.length !== 0) ||
    (response.status === "partial" && response.partial_reasons.length === 0)
  ) {
    schemaError(
      `${path}.status`,
      "consistent with whether partial_reasons is empty",
    );
  }
  // A cursor page starts wherever the cursor landed, so the ordinals below its
  // first segment wait for an older page instead of a partial reason. Legacy
  // responses still account for every leading gap.
  const cursorPage = response.has_more !== undefined;

  let previousOrdinal = 0;
  for (const [segmentIndex, segment] of response.segments.entries()) {
    const segmentPath = `${path}.segments[${segmentIndex}]`;
    if (
      !segment.inherited &&
      segment.binding.product_session_id !== response.product_session_id
    ) {
      schemaError(
        `${segmentPath}.binding.product_session_id`,
        "the transcript product_session_id",
      );
    }
    if (
      segment.inherited &&
      (!segment.source_product_session_id ||
        segment.binding.product_session_id !== segment.source_product_session_id)
    ) {
      schemaError(
        `${segmentPath}.source_product_session_id`,
        "the source product session id for inherited history",
      );
    }
    if (!segment.inherited && segment.source_product_session_id !== undefined) {
      schemaError(
        `${segmentPath}.source_product_session_id`,
        "absent for local session history",
      );
    }
    if (segment.binding.ordinal <= previousOrdinal) {
      schemaError(
        `${segmentPath}.binding.ordinal`,
        "strictly greater than the previous segment ordinal",
      );
    }
    for (
      let missingOrdinal =
        segmentIndex === 0 && cursorPage ? segment.binding.ordinal : previousOrdinal + 1;
      missingOrdinal < segment.binding.ordinal;
      missingOrdinal += 1
    ) {
      if (
        !response.partial_reasons.some(
          (reason) => reason.run_ordinal === missingOrdinal,
        )
      ) {
        schemaError(
          `${segmentPath}.binding.ordinal`,
          `covered by a partial reason for missing ordinal ${missingOrdinal}`,
        );
      }
    }
    let expectedSeq = 1;
    for (const [eventIndex, event] of segment.events.entries()) {
      if (
        event.seq !== expectedSeq &&
        !unknownEventTypeCoversStep(
          response.partial_reasons,
          segment.binding.ordinal,
          expectedSeq,
          event.seq,
          segment.binding.runtime_run_id,
        )
      ) {
        schemaError(
          `${segmentPath}.events[${eventIndex}].seq`,
          `the contiguous sequence ${expectedSeq}`,
        );
      }
      expectedSeq = event.seq + 1;
    }
    const observedThrough =
      segment.events[segment.events.length - 1]?.seq ?? 0;
    if (segment.observed_through_seq !== observedThrough) {
      schemaError(
        `${segmentPath}.observed_through_seq`,
        "the sequence of the final returned event",
      );
    }
    if (segment.observed_through_seq > segment.last_event_seq) {
      schemaError(
        `${segmentPath}.observed_through_seq`,
        "at most last_event_seq",
      );
    }
    if (
      segment.observed_through_seq < segment.last_event_seq &&
      !response.partial_reasons.some(
        (reason) =>
          partialReasonNamesRun(
            reason,
            segment.binding.ordinal,
            segment.binding.runtime_run_id,
          ) && PRODUCT_TRANSCRIPT_POSITIONLESS_GAP_REASON_CODES.has(reason.code),
      ) &&
      // A withheld-row reason claims exact durable positions, so it may only
      // explain a tail it actually reaches. Without this bound a response with
      // `observed_through_seq: 1` and `last_event_seq: 100` would parse clean
      // when a single withheld row at 2 is reported, and the client could never
      // notice that the server under-reported the rest of the gap. The server
      // stays consistent with this rule: a residue beyond the withheld range is
      // reported as `missing_event_range`.
      !unknownEventTypeCoversStep(
        response.partial_reasons,
        segment.binding.ordinal,
        segment.observed_through_seq + 1,
        segment.last_event_seq + 1,
        segment.binding.runtime_run_id,
      )
    ) {
      schemaError(
        `${segmentPath}.last_event_seq`,
        "equal to observed_through_seq or covered by a typed partial reason for this run ordinal",
      );
    }
    previousOrdinal = segment.binding.ordinal;
  }
  return response;
}

function parseProductReviewTargetSpec(
  value: unknown,
  path: string,
): ProductReviewTargetSpec {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["kind", "revision"], path);
  const kind = expectEnum(
    record.kind,
    PRODUCT_REVIEW_TARGET_KINDS,
    `${path}.kind`,
  );
  const revision = optionalString(record, "revision", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControlCharacters: true,
  });
  if (kind === "uncommitted" && revision !== undefined) {
    schemaError(`${path}.revision`, "omitted for an uncommitted target");
  }
  if ((kind === "base" || kind === "commit") && revision === undefined) {
    schemaError(`${path}.revision`, "present for a base or commit target");
  }
  return revision === undefined ? { kind } : { kind, revision };
}

export function parseCreateProductReviewRequest(
  value: CreateProductReviewRequest,
): CreateProductReviewRequest {
  const record = expectRecord(value, "create product review request");
  expectOnlyKeys(record, ["target", "idempotency_key", "max_steps"], "create product review request");
  const request: CreateProductReviewRequest = {
    target: parseProductReviewTargetSpec(record.target, "create product review request.target"),
  };
  const key = optionalString(record, "idempotency_key", "create product review request", {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES,
    noControlCharacters: true,
  });
  if (key !== undefined) {
    if (!/^[A-Za-z0-9_.:-]+$/u.test(key)) {
      schemaError("create product review request.idempotency_key", "a valid idempotency key");
    }
    request.idempotency_key = key;
  }
  const maxSteps = record.max_steps;
  if (maxSteps !== undefined && maxSteps !== null) {
    request.max_steps = expectInteger(maxSteps, "create product review request.max_steps", {
      min: 1,
      max: MAX_PRODUCT_MAX_STEPS,
    });
  }
  return request;
}

export function parseCreateProductControlRequest(
  value: unknown,
  path = "create product control request",
): CreateProductControlRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["content", "idempotency_key"], path);
  const content = expectString(record.content, `${path}.content`, {
    maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
  }).trim();
  if (!content) {
    return schemaError(`${path}.content`, "a non-empty string");
  }
  const request: CreateProductControlRequest = { content };
  assignOptional(
    request,
    "idempotency_key",
    optionalString(record, "idempotency_key", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES,
    }),
  );
  return request;
}

export function parseProductProviderProfileRequest(
  value: unknown,
  path = "product provider profile request",
): CreateProductProviderProfileRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "label",
      "provider_type",
      "api_base",
      "api_key_env",
      "default_model",
      "expected_revision",
    ],
    path,
  );
  const providerType = expectEnum(
    record.provider_type,
    PRODUCT_PROVIDER_TYPES,
    `${path}.provider_type`,
  );
  const apiBase = expectString(record.api_base, `${path}.api_base`, {
    maxBytes: MAX_PRODUCT_API_BASE_BYTES,
  });
  const apiKeyEnv = optionalString(record, "api_key_env", path, {
    nonEmpty: true,
    maxBytes: 256,
  });
  assertSafeProductProviderConfiguration(
    providerType,
    apiBase,
    apiKeyEnv,
    path,
  );
  const request: CreateProductProviderProfileRequest = {
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: providerType,
    api_base: apiBase,
  };
  assignOptional(request, "api_key_env", apiKeyEnv);
  assignOptional(
    request,
    "default_model",
    optionalString(record, "default_model", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  assignOptional(
    request,
    "expected_revision",
    optionalString(record, "expected_revision", path, { nonEmpty: true }),
  );
  return request;
}

function parseProductProviderSelection(
  value: unknown,
  path: string,
): ProductProviderSelection {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["profile_id", "model", "approval", "max_steps"], path);
  const selection: ProductProviderSelection = {
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    approval: expectEnum(
      record.approval,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.approval`,
    ),
    max_steps: expectInteger(record.max_steps, `${path}.max_steps`, {
      min: 1,
      max: 4_096,
    }),
  };
  assignOptional(
    selection,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  return selection;
}

export function parseUpdateProductPreferencesRequest(
  value: unknown,
  path = "update product preferences request",
): UpdateProductPreferencesRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "schema_version",
      "expected_revision",
      "theme",
      "default_approval_policy",
      "active_workspace_id",
      "active_session_id",
      "provider_selection",
    ],
    path,
  );
  const request: UpdateProductPreferencesRequest = {
    schema_version: expectInteger(
      record.schema_version,
      `${path}.schema_version`,
      { min: 1 },
    ),
    theme: expectEnum(
      record.theme,
      PRODUCT_THEME_PREFERENCES,
      `${path}.theme`,
    ),
  };
  assignOptional(
    request,
    "expected_revision",
    optionalInteger(record, "expected_revision", path, { min: 0 }),
  );
  if (
    record.default_approval_policy !== undefined &&
    record.default_approval_policy !== null
  ) {
    request.default_approval_policy = expectEnum(
      record.default_approval_policy,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.default_approval_policy`,
    );
  }
  assignOptional(
    request,
    "active_workspace_id",
    optionalString(record, "active_workspace_id", path, { nonEmpty: true }),
  );
  assignOptional(
    request,
    "active_session_id",
    optionalString(record, "active_session_id", path, { nonEmpty: true }),
  );
  if (
    record.provider_selection !== undefined &&
    record.provider_selection !== null
  ) {
    request.provider_selection = parseProductProviderSelection(
      record.provider_selection,
      `${path}.provider_selection`,
    );
  }
  return request;
}

export function parseCreateProductMessageRequest(
  value: unknown,
  path = "create product message request",
): CreateProductMessageRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["content", "idempotency_key", "attachments"], path);
  const content = expectString(record.content, `${path}.content`, {
    maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
  }).trim();
  if (!content) {
    return schemaError(`${path}.content`, "a non-empty string");
  }
  const request: CreateProductMessageRequest = { content };
  assignOptional(
    request,
    "idempotency_key",
    optionalString(record, "idempotency_key", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES,
    }),
  );
  const attachments = optionalArray(
    record,
    "attachments",
    path,
    parseProductMessageAttachmentRequest,
    MAX_MESSAGE_ATTACHMENTS,
  );
  if (attachments !== undefined && attachments.length > 0) {
    request.attachments = attachments;
  }
  return request;
}

function parseProductMessageAttachmentRequest(
  value: unknown,
  path = "product message attachment request",
): ProductMessageAttachmentRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["attachment_id", "name"], path);
  const request: ProductMessageAttachmentRequest = {
    attachment_id: expectId(record.attachment_id, `${path}.attachment_id`),
  };
  assignOptional(
    request,
    "name",
    optionalString(record, "name", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  return request;
}

export function parsePromoteProductMessageRequest(
  value: unknown,
  path = "promote product message request",
): PromoteProductMessageRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["delivery"], path);
  const request: PromoteProductMessageRequest = {};
  const delivery = record.delivery;
  if (delivery !== undefined && delivery !== null) {
    request.delivery = expectEnum(delivery, PRODUCT_MESSAGE_DELIVERIES, `${path}.delivery`);
  }
  return request;
}

export function parseReorderProductMessagesRequest(
  value: unknown,
  path = "reorder product messages request",
): ReorderProductMessagesRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["ordered_ids"], path);
  const orderedIds = expectArray(
    record.ordered_ids,
    `${path}.ordered_ids`,
    (item, itemPath) => expectId(item, itemPath),
    MAX_PENDING_PRODUCT_MESSAGES,
  );
  // The server refuses a duplicate with 400 instead of merging it, so the client
  // refuses it before the request leaves: a duplicate list is never meaningful.
  const seen = new Set(orderedIds);
  if (seen.size !== orderedIds.length) {
    return schemaError(`${path}.ordered_ids`, "a list of unique ids");
  }
  return { ordered_ids: orderedIds };
}

export function parseOnboardProductProviderRequest(
  value: unknown,
  path = "provider onboarding request",
): OnboardProductProviderRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "profile_id",
      "label",
      "provider_type",
      "api_base",
      "model",
      "make_default",
      "expected_revision",
      "credential",
    ],
    path,
  );
  const request: OnboardProductProviderRequest = {
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: expectEnum(
      record.provider_type,
      PRODUCT_PROVIDER_TYPES,
      `${path}.provider_type`,
    ),
    api_base: expectString(record.api_base, `${path}.api_base`, {
      maxBytes: MAX_PRODUCT_API_BASE_BYTES,
    }),
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES * 2,
    }),
    credential: expectString(record.credential, `${path}.credential`, {
      nonEmpty: true,
      maxBytes: 16 * 1024,
    }),
  };
  assignOptional(
    request,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  assignOptional(
    request,
    "make_default",
    optionalBoolean(record, "make_default", path),
  );
  assignOptional(
    request,
    "expected_revision",
    optionalString(record, "expected_revision", path, { nonEmpty: true }),
  );
  return request;
}
