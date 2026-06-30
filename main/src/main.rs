use anyhow::Result;
use clap::Parser;

use cntx::app;
use cntx::cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cntx=warn".into()),
        )
        .with_target(false)
        .compact()
        .init();

    app::run(Cli::parse()).await
}
