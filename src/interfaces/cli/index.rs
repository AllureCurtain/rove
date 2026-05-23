use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOptions {
    pub cwd: Option<PathBuf>,
    pub deterministic: bool,
    pub embedding_model: Option<String>,
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
async fn run_impl(options: IndexOptions) -> anyhow::Result<()> {
    use crate::config::AppConfig;
    use crate::core::workspace::Workspace;
    use crate::tools::rag::{DeterministicEmbedder, OpenAiEmbedder, RagIndex};

    let cwd = options
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;

    let config = AppConfig::from_env()?;
    let index = RagIndex::new(workspace.root.clone());
    let count = if options.deterministic || config.api_key.is_empty() {
        let embedder = DeterministicEmbedder;
        index.ingest_workspace(&embedder).await?
    } else {
        let embedder = OpenAiEmbedder::new(
            config.api_base,
            config.api_key,
            options
                .embedding_model
                .or_else(|| std::env::var("ROVE_EMBEDDING_MODEL").ok())
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
        );
        index.ingest_workspace(&embedder).await?
    };

    print!("{}", format_index_result(count, &workspace.root));
    Ok(())
}

#[cfg(not(feature = "rag"))]
async fn run_impl(_options: IndexOptions) -> anyhow::Result<()> {
    anyhow::bail!(
        "`rove index` requires the `rag` feature. Rebuild with `--features rag` or use a `rove-index` binary built with that feature."
    );
}
