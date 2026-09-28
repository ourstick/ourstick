//! `ourstick` CLI: share / ls / get.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    println!(
        "ourstick {} — a USB stick you can send",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}
