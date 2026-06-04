pub mod aggregation;
pub mod application;
pub mod config;
pub mod error;
pub mod handlers;

use tracing::{debug, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::application::Application;
use crate::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::load().inspect_err(|e| error!("Failed to load configuration: {e}"))?;
    debug!("Configuration loaded");

    Application::new(config).run().await?;

    Ok(())
}
