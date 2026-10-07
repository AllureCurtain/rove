use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use rove_api::serve;

#[derive(Debug, Parser)]
#[command(name = "rove-api", about = "Serve the rove HTTP API")]
struct Args {
    /// Address to bind. Overrides api.bind_addr from `.rove/config.toml` and env.
    #[arg(long)]
    addr: Option<SocketAddr>,

    /// Working directory for jobs.
    #[arg(short = 'C', long)]
    cwd: Option<PathBuf>,

    /// Directory containing the Web console static bundle (the `pnpm
    /// build:web` output under apps/web). When set, this process also serves
    /// the console at the same origin. Falls back to `ROVE_WEB_DIST`.
    #[arg(long, value_name = "DIR")]
    web_dist: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rove_api=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();
    let cwd = args
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    serve(args.addr, cwd, rove_api::web_console_root(args.web_dist)).await
}
