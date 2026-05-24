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
    format!(
        "indexed {count} chunks into {}\n",
        workspace_root.join(".rove").join("rag.lancedb").display()
    )
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
    use crate::tools::rag::eval::{run_retrieval_eval, write_eval_report};
    use crate::tools::rag::{DeterministicEmbedder, Embedder, OpenAiEmbedder, RagIndex};

    let cwd = options
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;

    let config = AppConfig::load(&workspace.root, AppConfigOverrides::default())?;
    let index = RagIndex::new(workspace.root.clone());
    let use_deterministic = options.deterministic || config.provider.api_key.is_empty();
    let openai_model = options
        .embedding_model
        .or_else(|| std::env::var("ROVE_EMBEDDING_MODEL").ok())
        .unwrap_or_else(|| "text-embedding-3-small".to_string());

    if let Some(query) = options.eval_query {
        let kind = parse_eval_kind(options.eval_kind.as_deref().unwrap_or("docs"))?;
        let embedder: Box<dyn Embedder> = if use_deterministic {
            Box::new(DeterministicEmbedder)
        } else {
            Box::new(OpenAiEmbedder::new(
                config.provider.api_base,
                config.provider.api_key,
                openai_model,
            ))
        };
        let report =
            run_retrieval_eval(&index, embedder.as_ref(), kind, &query, options.eval_limit).await?;
        let path = write_eval_report(&workspace.root, &report).await?;
        print!("{}", format_eval_result(&report, &path));
        return Ok(());
    }

    let count = if use_deterministic {
        let embedder = DeterministicEmbedder;
        index.ingest_workspace(&embedder).await?
    } else {
        let embedder = OpenAiEmbedder::new(
            config.provider.api_base,
            config.provider.api_key,
            openai_model,
        );
        index.ingest_workspace(&embedder).await?
    };

    print!("{}", format_index_result(count, &workspace.root));
    Ok(())
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
