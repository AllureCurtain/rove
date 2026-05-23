use std::path::PathBuf;

use clap::Parser;
use rove::interfaces::cli::index::{IndexOptions, run};

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
    run(IndexOptions {
        cwd: args.cwd,
        deterministic: args.deterministic,
        embedding_model: args.embedding_model,
    })
    .await
}
