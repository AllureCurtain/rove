use std::path::PathBuf;

use clap::Parser;
use rove::bench::{available_suites, resolve_suite, run_benchmark_suite};

#[derive(Debug, Parser)]
#[command(about = "Run deterministic local rove benchmark tasks")]
struct Args {
    /// Suite name (e.g. "dataprep", "agent-smoke")
    #[arg(long, default_value = "dataprep")]
    suite: String,

    /// Profile within the suite ("default" or "stress")
    #[arg(long, default_value = "default")]
    profile: String,

    /// Output directory for evidence packages
    #[arg(long, default_value = "benchmarks/results")]
    output_dir: PathBuf,

    /// List available suites and exit
    #[arg(long)]
    list: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.list {
        println!("Available benchmark suites:\n");
        for info in available_suites() {
            println!("  {} — {}", info.name, info.description);
            println!("    profiles: {}\n", info.profiles.join(", "));
        }
        return Ok(());
    }

    let suite = resolve_suite(&args.suite, &args.profile)?;
    let report = run_benchmark_suite(&suite, &args.output_dir, &args.profile).await?;

    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("\nEvidence package: {}", report.evidence_root.display());
    println!(
        "Result: {}/{} tasks passed",
        report.passed_tasks, report.total_tasks
    );

    if report.passed {
        Ok(())
    } else {
        std::process::exit(1);
    }
}
