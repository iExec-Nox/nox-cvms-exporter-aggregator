use std::collections::HashMap;

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
    /// Max instances enriched concurrently on `POST /cvms/attestations` (each
    /// enrichment issues two requests: `/quote` + `/info`). Bounds the load on
    /// the CVM nodes.
    pub max_inflight: usize,
    /// Port of the quote service exposed by every CVM. Used, together with the
    /// per-machine URL suffix, to rebuild each CVM's base URL locally.
    pub quote_service_port: u16,
    /// Per-machine URL suffixes, as `machine_id=suffix_url` pairs (comma-separated).
    /// A CVM's base URL is rebuilt as `https://<instance_id>-<quote_service_port>.<suffix_url>`,
    /// where `suffix_url` is looked up by the instance's `machine_id`.
    pub machines: Vec<String>,
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
        let config: Self = ConfigBuilder::builder()
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8080)?
            .set_default("exporters", Vec::<String>::new())?
            .set_default("request_timeout_secs", 10)?
            .set_default("max_inflight", 2)?
            .set_default("quote_service_port", 9999)?
            .set_default("machines", Vec::<String>::new())?
            .add_source(
                Environment::with_prefix("NOX_CVMS_EXPORTER_AGGREGATOR")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true)
                    .list_separator(",")
                    .with_list_parse_key("exporters")
                    .with_list_parse_key("machines"),
            )
            .build()?
            .try_deserialize()?;

        config.validate()?;

        Ok(config)
    }

    /// Rejects a configuration that would silently misbehave at runtime instead
    /// of degrading quietly: no exporter to aggregate, or a malformed `machines`
    /// entry whose CVMs would then be unaddressable. Better to stop at startup
    /// than to ignore a value and carry on as if nothing were wrong.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.exporters.is_empty() {
            return Err(ConfigError::Message(
                "no exporters configured: set NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS".to_owned(),
            ));
        }

        // Parse the machine map to surface any malformed `machines` entry at
        // startup; `machine_suffixes` is the single source of truth for what a
        // well-formed entry is.
        self.machine_suffixes()?;

        Ok(())
    }

    /// Returns the `host:port` string that the HTTP server should bind to.
    pub fn bind_addr(&self) -> String {
        let addr = format!("{}:{}", self.server.host, self.server.port);
        debug!("Binding server on {}", addr);
        addr
    }

    /// Parses `machines` (`machine_id=suffix_url` pairs) into a lookup map,
    /// trimming whitespace around each key and value.
    ///
    /// Returns an error on any entry that is not a well-formed
    /// `machine_id=suffix_url` pair (missing `=`, empty key, or empty value): a
    /// malformed entry surfaces here rather than being silently skipped. The
    /// check is self-contained — it does not lean on `validate` having run.
    pub fn machine_suffixes(&self) -> Result<HashMap<String, String>, ConfigError> {
        self.machines
            .iter()
            .map(|entry| match entry.split_once('=') {
                Some((id, suffix)) if !id.trim().is_empty() && !suffix.trim().is_empty() => {
                    Ok((id.trim().to_owned(), suffix.trim().to_owned()))
                }
                _ => Err(ConfigError::Message(format!(
                    "invalid `machines` entry {entry:?}: expected `machine_id=suffix_url`"
                ))),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(exporters: &[&str], machines: &[&str]) -> Config {
        Config {
            server: ServerConfig {
                host: "0.0.0.0".to_owned(),
                port: 8080,
            },
            exporters: exporters.iter().map(|s| (*s).to_owned()).collect(),
            request_timeout_secs: 10,
            max_inflight: 2,
            quote_service_port: 9999,
            machines: machines.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn machine_suffixes_parses_pairs_and_trims() {
        let map = config(&[], &["m-a = node-a.example ", "m-b=node-b.example"])
            .machine_suffixes()
            .expect("well-formed entries should parse");

        assert_eq!(map.get("m-a").map(String::as_str), Some("node-a.example"));
        assert_eq!(map.get("m-b").map(String::as_str), Some("node-b.example"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn machine_suffixes_errors_on_a_malformed_entry() {
        let err = config(&[], &["no-separator"])
            .machine_suffixes()
            .unwrap_err();

        assert!(
            err.to_string().contains("no-separator"),
            "error should name the offending entry: {err}"
        );
    }

    #[test]
    fn validate_accepts_a_well_formed_config() {
        let config = config(&["http://node-a:8080"], &["m-a=node-a.example"]);

        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_exporters() {
        let err = config(&[], &["m-a=node-a.example"]).validate().unwrap_err();

        assert!(
            err.to_string().contains("exporters"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_rejects_a_machines_entry_without_separator() {
        let err = config(&["http://node-a:8080"], &["no-separator"])
            .validate()
            .unwrap_err();

        assert!(
            err.to_string().contains("no-separator"),
            "error should name the offending entry: {err}"
        );
    }

    #[test]
    fn validate_rejects_a_machines_entry_with_an_empty_value() {
        assert!(
            config(&["http://node-a:8080"], &["m-a="])
                .validate()
                .is_err()
        );
    }
}
