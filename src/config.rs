use config::{Config as ConfigBuilder, ConfigError, Environment};
use serde::Deserialize;
use tracing::debug;

/// Top-level application configuration.
///
/// Loaded from environment variables prefixed with `NOX_CVMS_EXPORTER_AGGREGATOR_`,
/// using `__` as the nesting separator (e.g. `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT=9000`).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// HTTP server settings.
    pub server: ServerConfig,
    /// Base URLs of the per-machine `nox-cvms-exporter` instances to query
    /// (e.g. `http://10.0.0.1:8080`). Provided as a comma-separated list.
    pub exporters: Vec<String>,
    /// Per-request timeout, in seconds, when querying a machine exporter.
    pub request_timeout_secs: u64,
}

/// HTTP server binding configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Host or IP address to bind to. Defaults to `0.0.0.0`.
    pub host: String,
    /// TCP port to listen on. Defaults to `8080`.
    pub port: u16,
}

impl Config {
    /// Loads configuration from environment variables, applying built-in defaults
    /// for any value not explicitly provided.
    pub fn load() -> Result<Self, ConfigError> {
        let config = ConfigBuilder::builder()
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8080)?
            .set_default("exporters", Vec::<String>::new())?
            .set_default("request_timeout_secs", 10)?
            .add_source(
                Environment::with_prefix("NOX_CVMS_EXPORTER_AGGREGATOR")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true)
                    .list_separator(",")
                    .with_list_parse_key("exporters"),
            )
            .build()?;

        config.try_deserialize()
    }

    /// Returns the `host:port` string that the HTTP server should bind to.
    pub fn bind_addr(&self) -> String {
        let addr = format!("{}:{}", self.server.host, self.server.port);
        debug!("Binding server on {}", addr);
        addr
    }
}
