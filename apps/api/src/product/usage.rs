//! Per-session usage, cost, and context aggregation.
//!
//! Token totals come from durable runtime `report.json` files for each product
//! run binding. Cost uses the pricing snapshot frozen on the product run model
//! row at claim/bind time so later rate-table edits never rewrite old runs.

use axum::Json;
use axum::extract::{Path, State};

use rove_runtime::state::report::RunReport;
use rove_runtime::types::{PromptCompactionMode, RunId, TaskState};

use crate::docs;
use crate::pricing::{CostBreakdown, PricingAvailability, PricingSnapshot, round_usd};
use crate::{ApiError, ApiErrorResponse, ApiState};

use super::{
    ProductContextOccupancy, ProductCostBreakdown, ProductPricingAvailability, ProductRunUsage,
    ProductSessionId, ProductSessionRunModelView, ProductSessionUsageResponse, ProductUsage,
};

#[utoipa::path(
    get,
    path = "/product/sessions/{session_id}/usage",
    tag = docs::PRODUCT_TAG,
    security(("BearerAuth" = [])),
    params(("session_id" = ProductSessionId, Path, description = "Product session id")),
    responses(
        (status = 200, description = "Aggregate usage, cost, and context occupancy", body = ProductSessionUsageResponse),
        (status = 404, description = "Product session not found", body = ApiErrorResponse),
        (status = 500, description = "Product store or runtime report operation failed", body = ApiErrorResponse),
        (status = 503, description = "ProductStore is unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_product_session_usage(
    State(state): State<ApiState>,
    Path(session_id): Path<ProductSessionId>,
) -> Result<Json<ProductSessionUsageResponse>, ApiError> {
    Ok(Json(load_product_session_usage(&state, &session_id).await?))
}

pub(crate) async fn load_product_session_usage(
    state: &ApiState,
    session_id: &ProductSessionId,
) -> Result<ProductSessionUsageResponse, ApiError> {
    let store = state.product_store()?;
    let context = store.get_session_context(session_id).await?;
    let bindings = store.list_run_bindings(session_id).await?;
    let run_models = store.list_session_run_models(session_id).await?;
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;

    let mut partial_reasons = Vec::new();
    let mut loaded = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let snapshot = run_models
            .iter()
            .find(|model| model.runtime_run_id == binding.runtime_run_id)
            .cloned();
        let report = match state_store.load_report(binding.runtime_run_id).await {
            Ok(report) => Some(report),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                partial_reasons.push(format!(
                    "run {}: report.json missing",
                    binding.runtime_run_id
                ));
                None
            }
            Err(error) => {
                partial_reasons.push(format!(
                    "run {}: report.json unreadable ({error})",
                    binding.runtime_run_id
                ));
                None
            }
        };
        let task_state = match state_store.load_task_state(binding.runtime_run_id).await {
            Ok(task_state) => Some(task_state),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                partial_reasons.push(format!(
                    "run {}: task_state.json missing; compaction evidence unavailable",
                    binding.runtime_run_id
                ));
                None
            }
            Err(error) => {
                partial_reasons.push(format!(
                    "run {}: task_state.json unreadable; compaction evidence unavailable ({error})",
                    binding.runtime_run_id
                ));
                None
            }
        };
        loaded.push(LoadedRun {
            ordinal: binding.ordinal,
            runtime_run_id: binding.runtime_run_id,
            report,
            task_state,
            snapshot,
        });
    }

    let (totals, totals_cost, latest_context, runs, extra_reasons) = aggregate_loaded_runs(loaded);
    partial_reasons.extend(extra_reasons);

    Ok(ProductSessionUsageResponse {
        product_session_id: session_id.clone(),
        totals,
        totals_cost,
        latest_context,
        runs,
        partial_reasons,
    })
}

struct LoadedRun {
    ordinal: u64,
    runtime_run_id: RunId,
    report: Option<RunReport>,
    task_state: Option<TaskState>,
    snapshot: Option<ProductSessionRunModelView>,
}

fn aggregate_loaded_runs(
    loaded: Vec<LoadedRun>,
) -> (
    ProductUsage,
    Option<ProductCostBreakdown>,
    Option<ProductContextOccupancy>,
    Vec<ProductRunUsage>,
    Vec<String>,
) {
    let mut totals = ProductUsage::default();
    let mut sum_cost: Option<CostBreakdown> = None;
    let mut cost_incomplete = false;
    let mut pricing_source: Option<String> = None;
    let mut pricing_version: Option<String> = None;
    let mut mixed_pricing_meta = false;
    let mut runs = Vec::new();
    let mut latest_context: Option<(u64, ProductContextOccupancy)> = None;
    let mut extra_reasons = Vec::new();

    for item in loaded {
        let Some(report) = item.report.as_ref() else {
            cost_incomplete = true;
            continue;
        };

        totals.prompt_tokens = totals
            .prompt_tokens
            .saturating_add(report.total_usage.prompt_tokens);
        totals.completion_tokens = totals
            .completion_tokens
            .saturating_add(report.total_usage.completion_tokens);
        totals.total_tokens = totals
            .total_tokens
            .saturating_add(report.total_usage.total_tokens);
        totals.cached_tokens = totals
            .cached_tokens
            .saturating_add(report.total_usage.cached_tokens);

        let usage = ProductUsage {
            prompt_tokens: report.total_usage.prompt_tokens,
            completion_tokens: report.total_usage.completion_tokens,
            total_tokens: report.total_usage.total_tokens,
            cached_tokens: report.total_usage.cached_tokens,
        };
        let context =
            context_from_sources(report, item.task_state.as_ref(), item.snapshot.as_ref());
        if let Some(context) = context.clone() {
            match &latest_context {
                Some((ordinal, _)) if *ordinal >= item.ordinal => {}
                _ => latest_context = Some((item.ordinal, context)),
            }
        }

        let (cost, snapshot_meta) = cost_for_run(&item, report, &mut extra_reasons);
        if let Some((source, version)) = snapshot_meta {
            match (&pricing_source, &pricing_version) {
                (None, None) => {
                    pricing_source = Some(source);
                    pricing_version = Some(version);
                }
                (Some(existing_source), Some(existing_version))
                    if existing_source == &source && existing_version == &version => {}
                _ => mixed_pricing_meta = true,
            }
        }
        match &cost {
            Some(value) => add_cost(&mut sum_cost, value),
            None => cost_incomplete = true,
        }

        runs.push(ProductRunUsage {
            runtime_run_id: item.runtime_run_id,
            ordinal: item.ordinal,
            model: item
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.model.clone())
                .unwrap_or_else(|| report.model_id.clone()),
            usage,
            cost: cost.as_ref().map(to_contract_cost),
            context,
            steps: report.steps,
            tool_calls: report.tool_calls,
        });
    }

    let totals_cost = if cost_incomplete {
        Some(ProductCostBreakdown {
            currency: "USD".to_string(),
            availability: ProductPricingAvailability::Unpriced,
            total_usd: None,
            prompt_usd: None,
            completion_usd: None,
            cache_read_usd: None,
            pricing_source: if mixed_pricing_meta {
                None
            } else {
                pricing_source
            },
            pricing_version: if mixed_pricing_meta {
                None
            } else {
                pricing_version
            },
        })
    } else {
        sum_cost.map(|value| {
            let mut contract = to_contract_cost(&value);
            if !mixed_pricing_meta {
                contract.pricing_source = pricing_source;
                contract.pricing_version = pricing_version;
            }
            contract
        })
    };

    (
        totals,
        totals_cost,
        latest_context.map(|(_, context)| context),
        runs,
        extra_reasons,
    )
}

fn cost_for_run(
    item: &LoadedRun,
    report: &RunReport,
    extra_reasons: &mut Vec<String>,
) -> (Option<CostBreakdown>, Option<(String, String)>) {
    if let Some(snapshot) = item.snapshot.as_ref() {
        if let Some(availability) = snapshot.pricing_availability {
            let pricing = PricingSnapshot {
                source: snapshot
                    .pricing_source
                    .clone()
                    .unwrap_or_else(|| crate::pricing::BUNDLED_PRICING_SOURCE.to_string()),
                version: snapshot
                    .pricing_version
                    .clone()
                    .unwrap_or_else(|| crate::pricing::BUNDLED_PRICING_VERSION.to_string()),
                currency: snapshot
                    .pricing_currency
                    .clone()
                    .unwrap_or_else(|| crate::pricing::BUNDLED_PRICING_CURRENCY.to_string()),
                availability: match availability {
                    ProductPricingAvailability::Priced => PricingAvailability::Priced,
                    ProductPricingAvailability::LocalZero => PricingAvailability::LocalZero,
                    ProductPricingAvailability::Unpriced => PricingAvailability::Unpriced,
                },
                per_mtok_prompt: snapshot.per_mtok_prompt,
                per_mtok_completion: snapshot.per_mtok_completion,
                per_mtok_cache_read: snapshot.per_mtok_cache_read,
            };
            return (
                pricing.cost_for(&report.total_usage),
                Some((pricing.source.clone(), pricing.version.clone())),
            );
        }
        extra_reasons.push(format!(
            "run {}: historical model snapshot has no pricing fields",
            item.runtime_run_id
        ));
    } else {
        extra_reasons.push(format!(
            "run {}: product model snapshot missing; using live bundled rates",
            item.runtime_run_id
        ));
    }

    // Fallback only when no durable snapshot exists (legacy bindings). Still
    // classify fake models as local-zero rather than inventing commercial rates.
    let live = PricingSnapshot::bundled_for_model(&report.model_id);
    (
        live.cost_for(&report.total_usage),
        Some((live.source.clone(), live.version.clone())),
    )
}

fn context_from_sources(
    report: &RunReport,
    task_state: Option<&TaskState>,
    snapshot: Option<&ProductSessionRunModelView>,
) -> Option<ProductContextOccupancy> {
    let latest = report.prompt_builds.last();
    let checkpoint = task_state.and_then(|state| state.checkpoint.as_ref());
    if latest.is_none() && checkpoint.is_none() {
        return None;
    }
    let compaction = checkpoint.map(|checkpoint| &checkpoint.compaction);
    Some(ProductContextOccupancy {
        token_estimate: latest
            .map(|metadata| metadata.token_estimate)
            .or_else(|| checkpoint.map(|checkpoint| checkpoint.token_estimate))
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX),
        context_window: snapshot.and_then(|snapshot| snapshot.context_window),
        estimate_kind: "heuristic_char_div4".to_string(),
        included_history_messages: latest
            .map(|metadata| metadata.included_history_messages)
            .or_else(|| checkpoint.map(|checkpoint| checkpoint.preserved_tail.len()))
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX),
        dropped_history_messages: latest
            .map(|metadata| metadata.dropped_history_messages)
            .or_else(|| checkpoint.map(|checkpoint| checkpoint.compacted_history_messages))
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX),
        compaction_mode: compaction.map(|state| compaction_mode_name(&state.mode).to_string()),
        compaction_degraded: compaction.is_some_and(|state| state.degraded),
        compaction_auto_triggered: compaction.is_some_and(|state| state.auto_triggered),
        compacted_history_messages: checkpoint
            .and_then(|checkpoint| u64::try_from(checkpoint.compacted_history_messages).ok())
            .unwrap_or(0),
        compaction_source_messages: compaction
            .and_then(|state| u64::try_from(state.source_message_count).ok())
            .unwrap_or(0),
        compaction_prompt_version: compaction.and_then(|state| state.prompt_version.clone()),
        prompt_hash: latest.map(|metadata| metadata.prompt_hash.clone()),
    })
}

fn compaction_mode_name(mode: &PromptCompactionMode) -> &'static str {
    match mode {
        PromptCompactionMode::None => "none",
        PromptCompactionMode::Deterministic => "deterministic",
        PromptCompactionMode::ModelGenerated => "model_generated",
        PromptCompactionMode::Automatic => "automatic",
        PromptCompactionMode::Degraded => "degraded",
        PromptCompactionMode::Disabled => "disabled",
    }
}

fn add_cost(acc: &mut Option<CostBreakdown>, value: &CostBreakdown) {
    match acc {
        Some(existing) => {
            existing.total_usd += value.total_usd;
            existing.prompt_usd += value.prompt_usd;
            existing.completion_usd += value.completion_usd;
            existing.cache_read_usd += value.cache_read_usd;
            if existing.availability != value.availability {
                existing.availability = PricingAvailability::Priced;
            }
        }
        None => *acc = Some(value.clone()),
    }
}

fn to_contract_cost(value: &CostBreakdown) -> ProductCostBreakdown {
    let availability = match value.availability {
        PricingAvailability::Priced => ProductPricingAvailability::Priced,
        PricingAvailability::LocalZero => ProductPricingAvailability::LocalZero,
        PricingAvailability::Unpriced => ProductPricingAvailability::Unpriced,
    };
    ProductCostBreakdown {
        currency: value.currency.clone(),
        availability,
        total_usd: Some(round_usd(value.total_usd)),
        prompt_usd: Some(round_usd(value.prompt_usd)),
        completion_usd: Some(round_usd(value.completion_usd)),
        cache_read_usd: Some(round_usd(value.cache_read_usd)),
        pricing_source: None,
        pricing_version: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_models::Usage;
    use rove_runtime::types::{JobId, SessionId, TerminationReason};
    use rove_runtime::workspace::WorkspaceKind;
    use std::path::PathBuf;

    fn make_report(model: &str, prompt: u32, completion: u32, cached: u32) -> RunReport {
        let total = prompt + completion;
        RunReport {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            workspace_root: PathBuf::from("/tmp/test-ws"),
            workspace_kind: WorkspaceKind::Folder,
            model_id: model.to_string(),
            status: "success".to_string(),
            termination_reason: TerminationReason::Final,
            steps: 1,
            total_usage: Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: total,
                cached_tokens: cached,
            },
            tool_calls: 0,
            tool_failures: 0,
            tool_mutations: Vec::new(),
            tool_execution_metadata: Vec::new(),
            prompt_builds: Vec::new(),
            runtime_identity: None,
            step_records: Vec::new(),
            plan_decisions: Vec::new(),
            plan_revisions: Vec::new(),
            execution_lifecycle: Default::default(),
            final_outcome: None,
            tool_artifacts: Vec::new(),
            rejected_tool_artifacts: Vec::new(),
            message_deliveries: Vec::new(),
            output: None,
            timestamp: "2026-08-04T00:00:00Z".to_string(),
        }
    }

    fn snapshot_for(model: &str, run_id: RunId, ordinal: u64) -> ProductSessionRunModelView {
        let pricing = PricingSnapshot::bundled_for_model(model);
        ProductSessionRunModelView {
            product_session_id: ProductSessionId::new(),
            ordinal,
            runtime_run_id: run_id,
            profile_id: None,
            model: model.to_string(),
            reasoning: super::super::ProductReasoningPreference::Default,
            max_steps: 8,
            context_window: crate::pricing::bundled_context_window(model),
            pricing_source: Some(pricing.source.clone()),
            pricing_version: Some(pricing.version.clone()),
            pricing_currency: Some(pricing.currency.clone()),
            pricing_availability: ProductPricingAvailability::parse(pricing.availability.as_str()),
            per_mtok_prompt: pricing.per_mtok_prompt,
            per_mtok_completion: pricing.per_mtok_completion,
            per_mtok_cache_read: pricing.per_mtok_cache_read,
        }
    }

    #[test]
    fn aggregates_known_model_costs_from_snapshots() {
        let run_one = RunId::new();
        let run_two = RunId::new();
        let loaded = vec![
            LoadedRun {
                ordinal: 1,
                runtime_run_id: run_one,
                report: Some(make_report("claude-sonnet-4-5", 1000, 500, 200)),
                task_state: None,
                snapshot: Some(snapshot_for("claude-sonnet-4-5", run_one, 1)),
            },
            LoadedRun {
                ordinal: 2,
                runtime_run_id: run_two,
                report: Some(make_report("claude-sonnet-4-5", 2000, 1000, 0)),
                task_state: None,
                snapshot: Some(snapshot_for("claude-sonnet-4-5", run_two, 2)),
            },
        ];
        let (totals, totals_cost, _, runs, _) = aggregate_loaded_runs(loaded);
        assert_eq!(totals.prompt_tokens, 3000);
        assert_eq!(totals.completion_tokens, 1500);
        assert_eq!(totals.total_tokens, 4500);
        assert_eq!(totals.cached_tokens, 200);
        assert_eq!(runs.len(), 2);
        let cost = totals_cost.expect("cost");
        assert_eq!(cost.availability, ProductPricingAvailability::Priced);
        assert_eq!(cost.total_usd, Some(0.03096));
    }

    #[test]
    fn unpriced_model_keeps_token_totals_and_marks_cost_unavailable() {
        let run_one = RunId::new();
        let run_two = RunId::new();
        let loaded = vec![
            LoadedRun {
                ordinal: 1,
                runtime_run_id: run_one,
                report: Some(make_report("claude-sonnet-4-5", 1000, 500, 0)),
                task_state: None,
                snapshot: Some(snapshot_for("claude-sonnet-4-5", run_one, 1)),
            },
            LoadedRun {
                ordinal: 2,
                runtime_run_id: run_two,
                report: Some(make_report("mystery-model-x", 1000, 500, 0)),
                task_state: None,
                snapshot: Some(snapshot_for("mystery-model-x", run_two, 2)),
            },
        ];
        let (totals, totals_cost, _, runs, _) = aggregate_loaded_runs(loaded);
        assert_eq!(totals.prompt_tokens, 2000);
        assert!(runs[0].cost.is_some());
        assert!(runs[1].cost.is_none());
        let cost = totals_cost.expect("envelope");
        assert_eq!(cost.availability, ProductPricingAvailability::Unpriced);
        assert!(cost.total_usd.is_none());
    }

    #[test]
    fn fake_model_is_local_zero_not_unavailable() {
        let run_id = RunId::new();
        let loaded = vec![LoadedRun {
            ordinal: 1,
            runtime_run_id: run_id,
            report: Some(make_report("fake-raw", 100, 50, 0)),
            task_state: None,
            snapshot: Some(snapshot_for("fake-raw", run_id, 1)),
        }];
        let (_, totals_cost, _, runs, _) = aggregate_loaded_runs(loaded);
        let cost = totals_cost.expect("local zero");
        assert_eq!(cost.availability, ProductPricingAvailability::LocalZero);
        assert_eq!(cost.total_usd, Some(0.0));
        assert_eq!(
            runs[0].cost.as_ref().map(|value| value.availability),
            Some(ProductPricingAvailability::LocalZero)
        );
    }

    #[test]
    fn missing_report_does_not_double_count_tokens() {
        let run_id = RunId::new();
        let loaded = vec![
            LoadedRun {
                ordinal: 1,
                runtime_run_id: run_id,
                report: Some(make_report("claude-haiku-4-5", 1000, 1000, 0)),
                task_state: None,
                snapshot: Some(snapshot_for("claude-haiku-4-5", run_id, 1)),
            },
            LoadedRun {
                ordinal: 2,
                runtime_run_id: RunId::new(),
                report: None,
                task_state: None,
                snapshot: None,
            },
        ];
        let (totals, totals_cost, _, runs, _) = aggregate_loaded_runs(loaded);
        assert_eq!(totals.prompt_tokens, 1000);
        assert_eq!(runs.len(), 1);
        assert!(totals_cost.expect("partial").total_usd.is_none());
    }

    #[test]
    fn context_projects_frozen_window_and_compaction_checkpoint() {
        use rove_runtime::types::{PromptCheckpoint, PromptCompactionState};

        let run_id = RunId::new();
        let report = make_report("claude-sonnet-4-5", 100, 20, 0);
        let state = TaskState {
            schema_version: 1,
            session_id: report.session_id,
            job_id: report.job_id,
            run_id,
            goal: "test".to_string(),
            step: 2,
            history: Vec::new(),
            summary: Some("summary".to_string()),
            checkpoint: Some(PromptCheckpoint {
                summary: Some("summary".to_string()),
                preserved_tail: Vec::new(),
                session: None,
                plan: None,
                session_memory_pointer: None,
                durable_memory_pointer: None,
                last_step: 2,
                last_event_seq: Some(9),
                token_estimate: 12_345,
                compacted_history_messages: 24,
                compaction: PromptCompactionState {
                    mode: PromptCompactionMode::ModelGenerated,
                    auto_triggered: true,
                    degraded: false,
                    consecutive_failures: 0,
                    circuit_open: false,
                    model: Some("claude-sonnet-4-5".to_string()),
                    prompt_version: Some("rove.compaction.v3".to_string()),
                    source_message_count: 30,
                    last_error: None,
                },
                runtime_identity: None,
                agent_profile: None,
                step_ledger: Default::default(),
                execution_lifecycle: Default::default(),
                message_deliveries: Vec::new(),
            }),
            plan: None,
            runtime_identity: None,
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
        };
        let snapshot = snapshot_for("claude-sonnet-4-5", run_id, 1);
        let context = context_from_sources(&report, Some(&state), Some(&snapshot))
            .expect("checkpoint context");
        assert_eq!(context.token_estimate, 12_345);
        assert_eq!(context.context_window, Some(200_000));
        assert_eq!(context.compaction_mode.as_deref(), Some("model_generated"));
        assert!(context.compaction_auto_triggered);
        assert_eq!(context.compacted_history_messages, 24);
        assert_eq!(context.compaction_source_messages, 30);
        assert_eq!(
            context.compaction_prompt_version.as_deref(),
            Some("rove.compaction.v3")
        );
    }
}
