/**
 * Shared runtime/API types. Wire contracts are aliases of
 * `generated/api-types.ts` (`pnpm gen:api-types`; `pnpm check:api-types`
 * fails on drift), produced from `apps/api/openapi.json`.
 *
 * Still hand-written, deliberately:
 * - `STREAM_EVENT_NAMES` / `STREAM_EVENT_CONTRACT_VERSION` /
 *   `PRODUCT_EVENT_KINDS` — registries `tests/event_contract.rs` reads from
 *   this source file;
 * - decoded view shapes (`PromptBuildMetadata`, `PromptPruningFacts`,
 *   `ExecutionLifecycleState`) whose members are filled or projected at
 *   read time, and the wire decoders `readPromptPruningFacts` /
 *   `readPromptBuildMetadata` that produce them;
 * - legacy `/jobs` types the OpenAPI surface does not describe
 *   (`RunStatus`, `ProviderType`, `ProviderProfile`, `RunReport`,
 *   `ApprovalDecision`, `ApprovalPolicy`, `ResumeMode`).
 */
import type { components } from "../generated/api-types";

type Schema<K extends keyof components["schemas"]> = components["schemas"][K];

export type RunStatus = "init" | "running" | "done" | "error" | "cancelled" | "interrupted";

export type Usage = Schema<"Usage">;

export type PlanStep = Schema<"PlanStep">;

export type TaskPlan = Schema<"TaskPlan">;

export type StepRecordStatus = Schema<"StepRecordStatus">;

export type StepCompletionBasis = Schema<"StepCompletionBasis">;

export type PlanAmbiguityKind = Schema<"PlanAmbiguityKind">;

export type PlanAmbiguity = Schema<"PlanAmbiguity">;

export type ProcedureDeviationReason = Schema<"ProcedureDeviationReason">;

export type ProcedureCapabilityBinding = Schema<"ProcedureCapabilityBinding">;

export type ProcedureApplication = Schema<"ProcedureApplication">;

export type ProcedureDeviation = Schema<"ProcedureDeviation">;

export type StepRecord = Schema<"StepRecord">;

export type ExecutionBudgetUsage = Schema<"ExecutionBudgetUsage">;

export type ExecutionBudgetLimits = Schema<"ExecutionBudgetLimits">;

export type ExecutionPhase = Schema<"ExecutionPhase">;

export type ExecutionBudgetDimension = Schema<"ExecutionBudgetDimension">;

export type ExecutionBudgetExhaustion = Schema<"ExecutionBudgetExhaustion">;

export type ExecutionBudgetSnapshot = Schema<"ExecutionBudgetSnapshot">;

export type ExecutionStrategy = Schema<"ExecutionStrategy">;

export type StrategySelectionSource = Schema<"StrategySelectionSource">;

export type EvaluatorMode = Schema<"EvaluatorMode">;

export type FinalizerPolicy = Schema<"FinalizerPolicy">;

export type ExecutionPolicy = Schema<"ExecutionPolicy">;

export type ExecutionDegradation = Schema<"ExecutionDegradation">;

export type FinalOutcomeStatus = Schema<"FinalOutcomeStatus">;

export type FinalizationMode = Schema<"FinalizationMode">;

export type FinalizationPhase = Schema<"FinalizationPhase">;

export type FinalizationRecord = Schema<"FinalizationRecord">;

export interface ExecutionLifecycleState {
  policy?: ExecutionPolicy | null;
  budget_usage?: ExecutionBudgetUsage;
  budget_exhaustion?: ExecutionBudgetExhaustion | null;
  finalization?: FinalizationRecord | null;
  degradations?: ExecutionDegradation[];
  procedure_applications?: ProcedureApplication[];
  procedure_deviations?: ProcedureDeviation[];
}

export type PlanDecisionKind = Schema<"PlanDecisionKind">;

export type PlanFinishReason = Schema<"PlanFinishReason">;

export type PlanDecision = Schema<"PlanDecision">;

export type PlanDecisionRecord = Schema<"PlanDecisionRecord">;

export type PlanRevision = Schema<"PlanRevision">;

export type PromptCompactionMode = Schema<"PromptCompactionMode">;

export type PromptCompactionState = Schema<"PromptCompactionState">;

export interface PromptBuildMetadata {
  prompt_hash: string;
  stable_prefix_hash: string;
  workspace_fingerprint: string;
  tool_signature: string;
  token_estimate: number;
  included_history_messages: number;
  dropped_history_messages: number;
  /**
   * The server's own cache identity for this prompt.
   *
   * Absent when the runtime had none to report (`skip_serializing_if` on the
   * Rust side), which is why both decoders treat it as optional and
   * `state/transcript-projection.ts` falls back to `prompt_hash` rather than
   * rendering an empty key.
   */
  prompt_cache_key?: string;
  /**
   * Absent on a build that pruned nothing, which is most of them.
   *
   * Both arrival paths produce the decoded shape: the restored path validates
   * every member before defaulting the ones the server omitted, and the live
   * path runs the same wire record through `readPromptBuildMetadata`. Nothing
   * here is optional once `pruning` exists.
   */
  pruning?: PromptPruningFacts;
}

/**
 * What one bounded compaction request elided (R9).
 *
 * The runtime records these on the `PromptBuilt` metadata of the turn that ran
 * the compaction probe, so they describe that request rather than the turn's own
 * prompt. Each member is `skip_serializing_if`-defaulted on the wire, and a build
 * that pruned nothing carries no `pruning` object at all, so a reader must treat
 * the whole fact set's absence as "nothing was pruned" instead of reading a zero
 * the server never sent. This is the decoded shape the product parser produces:
 * it fills members the server omitted with their defaults, so nothing here is
 * optional once `pruning` exists.
 */

export interface PromptPruningFacts {
  /** Oversized tool payloads replaced by a bounded excerpt. */
  pruned_tool_results: number;
  /** Serialized bytes those payloads occupied before the projection. */
  pruned_payload_bytes: number;
  /** Serialized bytes they occupy as the request carried them. */
  pruned_excerpt_bytes: number;
  /** Older messages left out whole so the request stayed inside its bounds. */
  pruned_omitted_messages: number;
  /** Serialized bytes those omitted messages occupied. */
  pruned_omitted_bytes: number;
  /** Newest-first identities of the elided payloads; identity, not volume. */
  pruned_payload_digests: string[];
  /** The rule that produced the excerpt, absent when nothing was pruned. */
  pruning_policy: string | null;
}

/**
 * The decoded members of `PromptPruningFacts` that the runtime writes as counts.
 *
 * Derived from the fact set rather than listed again, so the key list below and
 * the shape it decodes into cannot drift: a key that is not a number member of the
 * fact set does not typecheck here, and a number member added to the fact set
 * without being added to the list leaves the object literal at the end of
 * `readPromptPruningFacts` missing a property.
 */

export const PRUNING_WIRE_COUNT_KEYS: readonly PruningCountMember[] = [
  "pruned_tool_results",
  "pruned_payload_bytes",
  "pruned_excerpt_bytes",
  "pruned_omitted_messages",
  "pruned_omitted_bytes",
] as const;

/**
 * The bounds the wire record is read inside.
 *
 * The strict restored path has always refused a record outside them; the live
 * path has no schema to refuse a frame with, so it withholds what does not fit
 * rather than rendering it. Both use these numbers, so a raised policy cap cannot
 * make one path accept what the other throws on. The digest count is generous on
 * purpose: the policy caps the list at 8, and a transcript read must not turn
 * into a schema error if that cap ever rises.
 */

export const MAX_PRUNED_PAYLOAD_DIGESTS = 64;

export const MAX_PRUNING_DIGEST_BYTES = 512;

export const MAX_PRUNING_POLICY_BYTES = 128;

/**
 * The control characters the restored path's schema refuses in a string member —
 * the same class `product-api-types.ts`'s `expectString` rejects under
 * `noControlCharacters`, copied here so the live reader applies the identical
 * predicate instead of a looser one.
 */

export function readPromptPruningFacts(record: object): PromptPruningFacts | undefined {
  const source = record as Record<string, unknown>;
  // Carried counts, validated one member at a time. A count the schema would
  // refuse withholds the whole fact set below instead of being read as zero.
  const carried: Partial<Record<PruningCountMember, number>> = {};
  for (const key of PRUNING_WIRE_COUNT_KEYS) {
    const value = source[key];
    if (value === undefined || value === null) {
      continue;
    }
    const count = readWireCount(value);
    if (count === null) {
      return undefined;
    }
    carried[key] = count;
  }
  // The digest list: the schema refuses a list longer than the cap, so this
  // reader withholds the fact set rather than answering with a shortened one.
  const rawDigests = source.pruned_payload_digests;
  const digests: string[] = [];
  if (rawDigests !== undefined && rawDigests !== null) {
    if (!Array.isArray(rawDigests) || rawDigests.length > MAX_PRUNED_PAYLOAD_DIGESTS) {
      return undefined;
    }
    for (const digest of rawDigests) {
      if (!describesWireString(digest, MAX_PRUNING_DIGEST_BYTES)) {
        return undefined;
      }
      digests.push(digest);
    }
  }
  const rawPolicy = source.pruning_policy;
  let policy: string | null = null;
  if (rawPolicy !== undefined && rawPolicy !== null) {
    if (!describesWireString(rawPolicy, MAX_PRUNING_POLICY_BYTES)) {
      return undefined;
    }
    policy = rawPolicy;
  }
  if (Object.keys(carried).length === 0 && digests.length === 0 && policy === null) {
    // Nothing survived: the wire state the runtime produces for a build that
    // pruned nothing, which is an absence and not a fact set of zeroes.
    return undefined;
  }
  return {
    // A member this record does not carry was reported as nothing, so it reads
    // as the default the restored path also produces — never as a measurement.
    pruned_tool_results: carried.pruned_tool_results ?? 0,
    pruned_payload_bytes: carried.pruned_payload_bytes ?? 0,
    pruned_excerpt_bytes: carried.pruned_excerpt_bytes ?? 0,
    pruned_omitted_messages: carried.pruned_omitted_messages ?? 0,
    pruned_omitted_bytes: carried.pruned_omitted_bytes ?? 0,
    pruned_payload_digests: digests,
    pruning_policy: policy,
  };
}

/** Whether one wire string member is what the restored path's schema accepts. */

export function readPromptBuildMetadata(value: unknown): PromptBuildMetadata | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const source = value as Record<string, unknown>;
  const promptHash = readWireString(source.prompt_hash);
  const stablePrefixHash = readWireString(source.stable_prefix_hash);
  const workspaceFingerprint = readWireString(source.workspace_fingerprint);
  const toolSignature = readWireString(source.tool_signature);
  const tokenEstimate = readWireCount(source.token_estimate);
  const includedHistoryMessages = readWireCount(source.included_history_messages);
  const droppedHistoryMessages = readWireCount(source.dropped_history_messages);
  if (
    promptHash === null ||
    stablePrefixHash === null ||
    workspaceFingerprint === null ||
    toolSignature === null ||
    tokenEstimate === null ||
    includedHistoryMessages === null ||
    droppedHistoryMessages === null
  ) {
    return null;
  }
  const metadata: PromptBuildMetadata = {
    prompt_hash: promptHash,
    stable_prefix_hash: stablePrefixHash,
    workspace_fingerprint: workspaceFingerprint,
    tool_signature: toolSignature,
    token_estimate: tokenEstimate,
    included_history_messages: includedHistoryMessages,
    dropped_history_messages: droppedHistoryMessages,
  };
  const cacheKey = readOptionalWireString(source.prompt_cache_key);
  if (cacheKey === null) {
    // Present but not a string the schema's `nonEmpty` rule accepts: the strict
    // parse refuses the whole record over this member, so this reader must not
    // answer with a build it would have refused — an optional member is still a
    // member of the record.
    return null;
  }
  if (cacheKey !== undefined) {
    metadata.prompt_cache_key = cacheKey;
  }
  // The record is decoded twice on the restored path — the product parser nests
  // the facts, and the projection then runs this decoder over its own output —
  // so an already-decoded `pruning` object is read as the fact set it is rather
  // than being dropped for carrying no flat keys. Both shapes use the same
  // member names, so one reader serves both; the nested object wins when a record
  // carries both, because that is the decoded shape of the same record and the
  // wire record has no nested member at all.
  const nested = source.pruning;
  const pruning = readPromptPruningFacts(
    typeof nested === "object" && nested !== null ? nested : source,
  );
  if (pruning !== undefined) {
    metadata.pruning = pruning;
  }
  return metadata;
}

/**
 * A wire string member as the restored path's schema reads it, or `null` when the
 * frame carried anything else.
 *
 * The trim rule is the schema's `nonEmpty: true`: a member of spaces is not an
 * identity there and must not become one here. The schema applies no
 * control-character rule to the build's identity members (only to the pruning
 * record's strings), so this reader applies none either — the two paths have to
 * agree on the same record, not on the same rule set in the abstract.
 */

export type ToolResult = Schema<"ToolResult">;

export const TOOL_RESULT_OUTCOMES = [
  "success",
  "partial",
  "error",
  "rejected",
  "cancelled",
  "timed_out_known_not_sent",
  "indeterminate",
] as const;

export type ToolResultOutcome = Schema<"ToolResultOutcome">;

export const ARTIFACT_VALIDATION_STATES = [
  "validated",
  "claim_rejected",
  "quota_exceeded",
] as const;

export type ArtifactValidation = Schema<"ArtifactValidation">;

export const TOOL_ARTIFACT_KINDS = [
  "text",
  "image",
  "audio",
  "resource",
  "unknown",
] as const;

export type ToolArtifactKind = Schema<"ToolArtifactKind">;

export type ToolArtifactSource = Schema<"ToolArtifactSource">;

export type ToolArtifactRef = Schema<"ToolArtifactRef">;

export type ToolContentBlockMeta = Schema<"ContentBlockMeta">;

export type ToolContentBlock = Schema<"ToolContentBlock">;

export type StructuredToolContent = Schema<"StructuredToolContent">;

export type ToolProtocolMetadata = Schema<"ToolProtocolMetadata">;

export type ToolDiagnostic = Schema<"ToolDiagnostic">;

export type ExternalEffect = Schema<"ExternalEffect">;

export type ToolOutputEnvelope = Schema<"ToolOutputEnvelope">;

export type ToolMutation = Schema<"ToolMutation">;

export type ToolMutationOperation = Schema<"ToolMutationOperation">;

export type ToolExecutionStatus = Schema<"ToolExecutionStatus">;

export type ToolRiskLevel = Schema<"ToolRiskLevel">;

export type ToolExecutionMetadata = Schema<"ToolExecutionMetadata">;

export type ToolCallRef = Schema<"ToolCallRef">;

export type ToolError = Schema<"ToolError">;

export type StreamEvent = Schema<"StreamEvent">;

export type AgentProfileIdentity = Schema<"AgentProfileIdentity">;

export type AgentDiagnostic = Schema<"AgentDiagnostic">;

export type ProcedureReference = Schema<"ProcedureReference">;

export type CreateJobWorkspaceKind = Schema<"CreateJobWorkspaceKind">;

export type CreateJobWorkspace = Schema<"CreateJobWorkspace">;

export type CreateJobRequest = Schema<"CreateJobRequest">;

export type ProviderType =
  | "openai"
  | "openai-responses"
  | "anthropic"
  | "ollama"
  | "fake";

export interface ProviderProfile {
  /**
   * Provider type: openai | openai-responses | anthropic | ollama | fake.
   * Maps to an internal wire protocol on the API. Not "official vs relay".
   */
  provider_type?: ProviderType;
  /**
   * Optional display label. When omitted/empty the API derives a name from
   * `api_base`. Use `provider_type` to select the type, not `name`.
   */
  name?: string;
  api_base: string;
  api_key_env?: string;
}

export type ProviderTestRequest = Schema<"ProviderTestRequest">;

export type ProviderTestResponse = Schema<"ProviderTestResponse">;

export type ProviderModelsRequest = Schema<"ProviderModelsRequest">;

export type ProviderModelsResponse = Schema<"ProviderModelsResponse">;

export type CreateJobResponse = Schema<"CreateJobResponse">;

export type JobStateResponse = Schema<"JobStateResponse">;

export type JobStreamEvent = Schema<"JobStreamEvent">;

export type ListRunsResponse = Schema<"ListRunsResponse">;

export type RunSummary = Schema<"RunSummaryResponse">;

export interface RunReport {
  session_id: string;
  job_id: string;
  run_id: string;
  workspace_root: string;
  workspace_kind: string;
  model_id: string;
  status: string;
  termination_reason: string;
  steps: number;
  total_usage: Usage;
  tool_calls: number;
  tool_failures: number;
  tool_mutations: ToolMutation[];
  step_records?: StepRecord[];
  output?: string | null;
  timestamp: string;
}

export type PendingApproval = Schema<"PendingApprovalResponse">;

export type PendingInput = Schema<"PendingInputResponse">;

export type ApprovalDecision = "approve" | "reject";

export type ApprovalPolicy = "ask" | "auto" | "never";

export type ResumeMode = "latest";

export const STREAM_EVENT_NAMES = [
  "run_started",
  "agent_profile_activated",
  "workspace_instructions_resolved",
  "execution_strategy_selected",
  "instruction_overlay_applied",
  "procedures_selected",
  "procedure_hydrated",
  "procedure_applied",
  "procedure_deviation",
  "execution_budget_updated",
  "execution_degraded",
  "llm_chunk",
  "model_status",
  "provider_retry",
  "llm_message",
  "tool_call_started",
  "tool_call_approval_needed",
  "tool_call_completed",
  "tool_call_failed",
  "tool_artifact_stored",
  "tool_artifact_rejected",
  "mcp_server_degraded",
  "mcp_capabilities_refreshed",
  "input_needed",
  "plan_created",
  "plan_step_started",
  "step_result",
  "plan_decision",
  "plan_revised",
  "finalization_started",
  "finalization_completed",
  "prompt_compacted",
  "memory_flushed",
  "prompt_built",
  "run_completed",
  "steer_accepted",
  "steer_applied",
  "steer_dropped",
  "followup_queued",
  "followup_dequeued",
  "followup_abandoned",
  "message_queued",
  "message_intervention_requested",
  "message_applied_current_run",
  "message_claimed_successor",
  "message_needs_attention",
  "message_revoked",
] as const;

export type StreamEventName = (typeof STREAM_EVENT_NAMES)[number];

/**
 * Canonical-event contract version this client can decode.
 *
 * The version is the newest `STREAM_EVENT_KINDS` entry version in
 * `runtime/src/foundation/events.rs` that this bundle understands: version 1 is
 * every kind before `provider_retry`, and version 2 is `provider_retry`. The
 * transcript request declares it as `event_contract`, so a newer server withholds
 * the rows this bundle cannot decode and reports their durable positions instead
 * of failing the restore. Adding a kind here means adding its version there and
 * bumping this constant in the same change; `tests/event_contract.rs` asserts the
 * three stay in agreement.
 */

export const STREAM_EVENT_CONTRACT_VERSION = 2;

/**
 * Product directory event kinds served by `GET /product/events`.
 *
 * The stream names every frame after its kind, so this list is the
 * subscription: a kind missing here is never delivered. A kind added later on
 * the server therefore costs latency, not correctness, because every delivered
 * frame refreshes the catalog and the catalog is what the UI renders.
 */

export const PRODUCT_EVENT_KINDS = [
  "session.created",
  "session.updated",
  "session.deleted",
  "session.status_changed",
  "workspace.created",
  "workspace.updated",
  "workspace.deleted",
  "preferences.changed",
  "control.queued",
  "control.promoted",
  "control.revoked",
] as const;

export type ProductEventKind = Schema<"ProductEventKind">;

export type ProductEvent = Schema<"ProductEvent">;

export type BenchSuiteInfo = Schema<"BenchSuiteInfoResponse">;

export type ListBenchSuitesResponse = Schema<"ListBenchSuitesResponse">;

export type StartBenchRunRequest = Schema<"StartBenchRunRequest">;

export type StartBenchRunResponse = Schema<"StartBenchRunResponse">;

export type BenchRunSummary = Schema<"BenchRunSummary">;

export type ListBenchRunsResponse = Schema<"ListBenchRunsResponse">;

export type BenchCheckResult = Schema<"BenchCheckResultResponse">;

export type BenchArtifacts = Schema<"BenchArtifactsResponse">;

export type BenchTaskResult = Schema<"BenchTaskResultResponse">;

export type BenchRunDetail = Schema<"BenchRunDetailResponse">;

// ---- Internal wire-reading helpers for the decoders above ----
type PruningCountMember = {
  [K in keyof PromptPruningFacts]: PromptPruningFacts[K] extends number ? K : never;
}[keyof PromptPruningFacts];

/**
 * The wire keys the runtime writes only while they carry something.
 *
 * `runtime/src/context/prompt_metadata.rs` marks all five counts and the policy
 * with `skip_serializing_if`, and the digest list with `Vec::is_empty`, so the
 * metadata a frame carries is a subset of the seven facts rather than seven
 * values. They sit flat on the metadata — the nesting below is this shell's own
 * decoded shape, which is why both decoders read this exact key list.
 *
 * The counts are separate from the policy and the digest list because this list
 * decides which members are counts and the strict restored path validates exactly
 * these members. It names members of `PromptPruningFacts` rather than arbitrary
 * strings, because presence is decided by the keys this reader carried: a key the
 * fact set does not have as a number would be counted as presence while decoding
 * to nothing, and "absence is not a measured zero" would break silently. Adding a
 * wire count therefore means adding it to the fact set *and* to this list, and
 * TypeScript refuses either half on its own.
 */

const WIRE_CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/u;

/**
 * Decode the flat wire pruning keys of one prompt-build metadata record.
 *
 * One reader for both arrival paths, because they used to disagree: the restored
 * path validated and nested these members, while the live path kept the metadata
 * object exactly as the SSE frame carried it — flat — so a live turn's message
 * had no `pruning` member at all and its facts appeared only once the transcript
 * had been re-fetched.
 *
 * Absence is not a measured zero. The runtime omits each member while it is zero
 * or empty, so a record carrying none of them means "this build pruned nothing"
 * and answers `undefined` rather than an all-zero fact set.
 *
 * Both paths judge a member by the same predicates — the type, the trim-non-empty
 * rule, the control-character rule and the byte bound that the restored path's
 * schema applies, and the same count bound on the digest list — and they dispose
 * of a member that fails the same way: the restored path has a schema and refuses
 * the record, which surfaces as a visible restore error, while this reader answers
 * `undefined` and the fact set is missing. That includes an optional member such
 * as `prompt_cache_key`: a record carrying one the schema would refuse is refused
 * whole there, so a build is either describable by both paths or by neither.
 *
 * The two paths read the same record in different domains. The strict parser reads
 * a flat wire record; this reader reads a flat wire record *or* its own already
 * decoded output, whose fact set is nested under `pruning`, which is why the
 * nested object wins when both are present — no wire record has one.
 *
 * Dropping just the offending member is deliberately not an option — the panel
 * prints the digest count, so a filtered list would report a shorter list as the
 * whole measurement, and half a fact set rendered as a measurement is worse than
 * the fact being missing. A truncated digest is worse still: it would name a
 * different payload.
 *
 * The one member whose *content* this shell cannot vouch for is a digest: it is
 * `format!("{identity}:{digest}")` over `tool_call_id`, which the runtime copies
 * from the provider without bounding it. A digest longer than the bound or
 * carrying a control character therefore is reachable through a provider, and the
 * two paths then answer as above — a restore error on the restored path, an
 * absent fact set on the live one. Neither invents a fact.
 */

function describesWireString(value: unknown, maxBytes: number): value is string {
  return (
    typeof value === "string" &&
    value.trim().length > 0 &&
    !WIRE_CONTROL_CHARACTERS.test(value) &&
    wireBytes(value) <= maxBytes
  );
}

/**
 * Decode one `prompt_built` metadata object into what consumers read.
 *
 * The wire record is flat, and `readPromptPruningFacts` turns its flat pruning
 * keys into the nested fact set, so a message carries the same object whichever
 * path built it. Answers `null` when the build's identity or counts are missing
 * or malformed: the live path cannot refuse the frame, and half a build would
 * render as "undefined tokens" in the trace and the evidence panel, which is a
 * worse answer than the event having no build description at all. The four
 * hashes are required for the same reason even though the summary panel reads
 * only the token estimate — the trace and the evidence panel name the build by
 * them, so a record that cannot name itself is not a description this shell can
 * render, and the reducer keeps the moment with a trace row that says so instead.
 */

function readWireString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

/**
 * An optional wire string member, distinguishing the three things the strict
 * parse distinguishes.
 *
 * `undefined` means the frame does not carry the member at all, which
 * `optionalString` reads as absent; a string is the value it accepts; `null` means
 * the frame carries something that member's schema refuses, which makes the strict
 * parse throw for the *whole* record. A frame cannot be described by one path and
 * refused by the other, so the caller must answer `null` for the record rather
 * than dropping the member.
 */

function readOptionalWireString(value: unknown): string | null | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  return readWireString(value);
}

/**
 * A non-negative safe-integer count, or `null` when the frame carried anything
 * else.
 *
 * `Number.isSafeInteger` rather than `Number.isInteger`, matching what the
 * restored path's schema accepts: a count beyond the safe range is not a
 * measurement either path should carry, and the two must not disagree about it.
 */

function readWireCount(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : null;
}

/** What one bounded string costs as UTF-8, which is the unit the caps use. */

function wireBytes(value: string): number {
  return new TextEncoder().encode(value).length;
}

