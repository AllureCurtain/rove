use std::path::PathBuf;

use clap::Parser;
use rove::bench::{load_benchmark_suite, run_benchmark_suite};

#[derive(Debug, Parser)]
#[command(about = "Run deterministic local rove benchmark tasks")]
struct Args {
    #[arg(long, default_value = "benchmarks/agent-smoke.json")]
    suite: PathBuf,

    #[arg(long, default_value = ".rove/bench")]
    output_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let suite = load_benchmark_suite(&args.suite).await?;
    let report = run_benchmark_suite(&suite, &args.output_dir).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);

    if report.passed {
        Ok(())
    } else {
        std::process::exit(1);
    }
}
