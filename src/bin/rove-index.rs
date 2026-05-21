use std::path::PathBuf;

use clap::Parser;
use rove::config::AppConfig;
use rove::core::workspace::Workspace;
use rove::tools::rag::{DeterministicEmbedder, OpenAiEmbedder, RagIndex};

#[derive(Debug, Parser)]
#[command(
    name = "rove-index",
    about = "Index a workspace for rove RAG retrieval"
)]
struct Args {
    /// Working directory to index.
    #[arg(short = 'C', long)]
    cwd: Option<PathBuf>,

    /// Use deterministic local embeddings instead of the OpenAI embedding API.
    #[arg(long)]
    deterministic: bool,

    /// OpenAI-compatible embedding model.
    #[arg(long)]
    embedding_model: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cwd = args
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;

    let config = AppConfig::from_env()?;
    let index = RagIndex::new(workspace.root.clone());
    let count = if args.deterministic || config.api_key.is_empty() {
        let embedder = DeterministicEmbedder;
        index.ingest_workspace(&embedder).await?
    } else {
        let embedder = OpenAiEmbedder::new(
            config.api_base,
            config.api_key,
            args.embedding_model
                .or_else(|| std::env::var("ROVE_EMBEDDING_MODEL").ok())
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
        );
        index.ingest_workspace(&embedder).await?
    };

    println!(
        "indexed {count} chunks into {}",
        workspace.root.join(".rove").join("rag.lancedb").display()
    );
    Ok(())
}
