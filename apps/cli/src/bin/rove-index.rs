use std::path::PathBuf;

use clap::Parser;
use rove_cli::cli::index::{IndexOptions, run};

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

    /// Run a pure retrieval eval query instead of indexing.
    #[arg(long)]
    eval: Option<String>,

    /// Retrieval kind for --eval: docs or code.
    #[arg(long, default_value = "docs")]
    kind: String,

    /// Retrieval result limit for --eval.
    #[arg(long, default_value_t = 8)]
    limit: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    run(IndexOptions {
        cwd: args.cwd,
        deterministic: args.deterministic,
        embedding_model: args.embedding_model,
        eval_query: args.eval,
        eval_kind: Some(args.kind),
        eval_limit: args.limit,
    })
    .await
}
