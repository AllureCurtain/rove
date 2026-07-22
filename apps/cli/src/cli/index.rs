use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOptions {
    pub cwd: Option<PathBuf>,
    pub deterministic: bool,
    pub embedding_model: Option<String>,
    pub eval_query: Option<String>,
    pub eval_kind: Option<String>,
    pub eval_limit: usize,
}

pub async fn run(options: IndexOptions) -> anyhow::Result<()> {
    run_impl(options).await
}

pub fn format_index_result(count: usize, workspace_root: &Path) -> String {
    format_index_result_for_db(count, &workspace_root.join(".rove").join("rag.lancedb"))
}

pub fn format_index_result_for_db(count: usize, db_dir: &Path) -> String {
    format!("indexed {count} chunks into {}\n", db_dir.display())
}

#[cfg(feature = "rag")]
pub fn format_eval_result(report: &crate::rag::eval::RetrievalEvalReport, path: &Path) -> String {
    let top_paths = report
        .results
        .iter()
        .take(3)
        .map(|result| result.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "eval query: {}\nreport: {}\nchannels: {}\nresults: {}\ntop: {}\n",
        report.query,
        path.display(),
        report.channels.len(),
        report.results.len(),
        top_paths
    )
}

#[cfg(feature = "rag")]
async fn run_impl(options: IndexOptions) -> anyhow::Result<()> {
    use crate::rag::RagIndex;
    use crate::rag::eval::{run_retrieval_eval_with_reranker, write_eval_report_to_dir};
    use rove_app_bootstrap::{AppConfig, AppConfigOverrides};
    use rove_runtime::workspace::Workspace;

    let cwd = options
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;

    let config = AppConfig::load(&workspace.root, AppConfigOverrides::default())?;
    let state_dir = config.state_dir();
    let index = RagIndex::new_with_state_dir(workspace.root.clone(), state_dir.clone());
    let use_deterministic = options.deterministic || config.rag.deterministic;
    let embedding_model = options
        .embedding_model
        .or_else(|| std::env::var("ROVE_EMBEDDING_MODEL").ok())
        .unwrap_or_else(|| config.rag.embedding_model.clone());

    if let Some(query) = options.eval_query {
        let kind = parse_eval_kind(options.eval_kind.as_deref().unwrap_or("docs"))?;
        let embedder = build_rag_embedder(&config, use_deterministic, embedding_model)?;
        let reranker = build_rag_reranker(&config)?;
        let report = run_retrieval_eval_with_reranker(
            &index,
            embedder.as_ref(),
            reranker.as_ref(),
            kind,
            &query,
            options.eval_limit,
        )
        .await?;
        let path = write_eval_report_to_dir(&state_dir.join("rag_eval"), &report).await?;
        print!("{}", format_eval_result(&report, &path));
        return Ok(());
    }

    let embedder = build_rag_embedder(&config, use_deterministic, embedding_model)?;
    let count = index.ingest_workspace(embedder.as_ref()).await?;

    print!(
        "{}",
        format_index_result_for_db(count, &state_dir.join("rag.lancedb"))
    );
    Ok(())
}

#[cfg(feature = "rag")]
fn build_rag_embedder(
    config: &rove_app_bootstrap::AppConfig,
    use_deterministic: bool,
    embedding_model: String,
) -> anyhow::Result<Box<dyn crate::rag::Embedder>> {
    use crate::rag::{DeterministicEmbedder, OpenAiEmbedder};

    if use_deterministic {
        return Ok(Box::new(DeterministicEmbedder));
    }
    if config.rag.embedding_api_key.trim().is_empty() {
        if config.rag.fallback_to_deterministic {
            return Ok(Box::new(DeterministicEmbedder));
        }
        anyhow::bail!(
            "rag.embedding_api_key is required when rag.deterministic=false and fallback_to_deterministic=false"
        );
    }
    Ok(Box::new(OpenAiEmbedder::new(
        config.rag.embedding_api_base.clone(),
        config.rag.embedding_api_key.clone(),
        embedding_model,
    )))
}

#[cfg(feature = "rag")]
pub fn build_rag_reranker(
    config: &rove_app_bootstrap::AppConfig,
) -> anyhow::Result<Box<dyn crate::rag::Reranker>> {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::rag::{DashScopeReranker, NoopReranker, RoutingReranker};
    use rove_models::health::{HealthConfig, ModelHealthStore};

    let Some(provider) = config.rag.rerank_provider.as_deref() else {
        return Ok(Box::new(NoopReranker));
    };
    let Some(model) = config.rag.rerank_model.as_deref() else {
        return Ok(Box::new(NoopReranker));
    };
    let Some(api_key) = config
        .rag
        .rerank_api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
    else {
        if config.rag.fallback_to_deterministic {
            tracing::warn!(
                "rag.rerank_api_key is missing; falling back to deterministic noop reranker"
            );
            return Ok(Box::new(NoopReranker));
        }
        anyhow::bail!(
            "rag.rerank_api_key is required when remote rerank is configured and fallback_to_deterministic=false"
        );
    };
    if !matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "dashscope" | "bailian" | "aliyun"
    ) {
        anyhow::bail!("unsupported rag.rerank_provider `{provider}`");
    }

    let remote = DashScopeReranker::with_timeout(
        config.rag.embedding_api_base.clone(),
        api_key.to_string(),
        model.to_string(),
        Duration::from_millis(config.rag.timeout_ms),
    );
    if config.rag.fallback_to_deterministic {
        let health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: config.routing.failure_threshold,
            open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
        }));
        Ok(Box::new(RoutingReranker::new(
            vec![Box::new(remote), Box::new(NoopReranker)],
            health,
        )))
    } else {
        Ok(Box::new(remote))
    }
}

#[cfg(feature = "rag")]
fn parse_eval_kind(kind: &str) -> anyhow::Result<crate::rag::RetrieveKind> {
    use crate::rag::RetrieveKind;

    match kind {
        "code" => Ok(RetrieveKind::Code),
        "docs" => Ok(RetrieveKind::Docs),
        other => anyhow::bail!("invalid eval kind `{other}`; expected `docs` or `code`"),
    }
}

#[cfg(not(feature = "rag"))]
async fn run_impl(_options: IndexOptions) -> anyhow::Result<()> {
    anyhow::bail!(
        "`rove index` requires the `rag` feature. Rebuild with `--features rag` or use a `rove-index` binary built with that feature."
    );
}
