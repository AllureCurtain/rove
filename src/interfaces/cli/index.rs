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
pub fn format_eval_result(
    report: &crate::tools::rag::eval::RetrievalEvalReport,
    path: &Path,
) -> String {
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
    use crate::config::{AppConfig, AppConfigOverrides};
    use crate::core::workspace::Workspace;
    use crate::tools::rag::RagIndex;
    use crate::tools::rag::eval::{run_retrieval_eval, write_eval_report_to_dir};

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
        let report =
            run_retrieval_eval(&index, embedder.as_ref(), kind, &query, options.eval_limit).await?;
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
    config: &crate::config::AppConfig,
    use_deterministic: bool,
    embedding_model: String,
) -> anyhow::Result<Box<dyn crate::tools::rag::Embedder>> {
    use crate::tools::rag::{DeterministicEmbedder, OpenAiEmbedder};

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
fn parse_eval_kind(kind: &str) -> anyhow::Result<crate::tools::rag::RetrieveKind> {
    use crate::tools::rag::RetrieveKind;

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
