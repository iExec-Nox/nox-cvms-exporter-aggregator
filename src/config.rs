use std::borrow::Cow;
use std::collections::HashMap;

use config::{Config as ConfigBuilder, ConfigError, Environment};
use serde::Deserialize;
use tracing::debug;
use validator::{Validate, ValidationError};

/// Top-level application configuration.
///
/// Loaded from environment variables prefixed with `NOX_CVMS_EXPORTER_AGGREGATOR_`,
/// using `__` as the nesting separator (e.g. `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT=9000`).
///
/// Field-level `#[validate(...)]` attributes let [`validator`] reject a
/// configuration that would misbehave at runtime; call [`Validate::validate`]
/// after loading (see `main`).
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Config {
    /// HTTP server settings.
    #[validate(nested)]
    pub server: ServerConfig,
    /// Base URLs of the per-machine `nox-cvms-exporter` instances to query
    /// (e.g. `http://10.0.0.1:8080`). Provided as a comma-separated list.
    #[validate(custom(function = "validate_exporters"))]
    pub exporters: Vec<String>,
    /// Per-request timeout, in seconds, when querying a machine exporter.
    #[validate(range(min = 1, message = "request_timeout_secs must be at least 1"))]
    pub request_timeout_secs: u64,
    /// Max instances enriched concurrently on `POST /cvms/attestations` (each
    /// enrichment issues two requests: `/quote` + `/info`). Bounds the load on
    /// the CVM nodes.
    #[validate(range(min = 1, message = "max_inflight must be at least 1"))]
    pub max_inflight: usize,
    /// Port of the quote service exposed by every CVM. Used, together with the
    /// per-machine URL suffix, to rebuild each CVM's base URL locally.
    #[validate(range(min = 1, message = "quote_service_port must be a valid port (1-65535)"))]
    pub quote_service_port: u16,
    /// Per-machine URL suffixes, as `machine_id=suffix_url` pairs (comma-separated).
    #[validate(custom(function = "validate_machines"))]
    pub machines: Vec<String>,
}

/// HTTP server binding configuration.
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct ServerConfig {
    /// Host or IP address to bind to. Defaults to `0.0.0.0`.
    #[validate(length(min = 1, message = "server host must not be empty"))]
    pub host: String,
    /// TCP port to listen on. Defaults to `8080`.
    #[validate(range(min = 1, message = "server port must be a valid port (1-65535)"))]
    pub port: u16,
}

/// Rejects an empty exporter list, or any entry that is not an `http(s)://` URL:
/// the aggregator has nothing to aggregate without exporters, and a URL it cannot
/// reach would only fail later at request time.
#[allow(clippy::ptr_arg)]
fn validate_exporters(exporters: &Vec<String>) -> Result<(), ValidationError> {
    if exporters.is_empty() {
        return Err(
            ValidationError::new("exporters_empty").with_message(Cow::from(
                "at least one exporter must be configured (NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS)",
            )),
        );
    }
    for url in exporters {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(
                ValidationError::new("exporter_invalid_url").with_message(Cow::from(format!(
                    "invalid exporter URL {url:?}: must start with http:// or https://"
                ))),
            );
        }
    }
    Ok(())
}

/// Rejects any `machines` entry that is not a well-formed `machine_id=suffix_url`
/// pair (missing `=`, empty key, or empty value): such an entry would leave its
/// CVMs unaddressable, so we stop at startup rather than silently skip it.
#[allow(clippy::ptr_arg)]
fn validate_machines(machines: &Vec<String>) -> Result<(), ValidationError> {
    for entry in machines {
        let well_formed = entry
            .split_once('=')
            .is_some_and(|(id, suffix)| !id.trim().is_empty() && !suffix.trim().is_empty());
        if !well_formed {
            return Err(
                ValidationError::new("invalid_machines_entry").with_message(Cow::from(format!(
                    "invalid `machines` entry {entry:?}: expected `machine_id=suffix_url`"
                ))),
            );
        }
    }
    Ok(())
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
            .build()?;

        config.try_deserialize()
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
    /// Malformed entries are rejected upstream by [`validate_machines`], so the
    /// defensive `filter_map` never actually skips anything once the config has
    /// been validated.
    pub fn machine_suffixes(&self) -> HashMap<String, String> {
        self.machines
            .iter()
            .filter_map(|entry| entry.split_once('='))
            .map(|(id, suffix)| (id.trim().to_owned(), suffix.trim().to_owned()))
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
        let map = config(&[], &["m-a = node-a.example ", "m-b=node-b.example"]).machine_suffixes();

        assert_eq!(map.get("m-a").map(String::as_str), Some("node-a.example"));
        assert_eq!(map.get("m-b").map(String::as_str), Some("node-b.example"));
        assert_eq!(map.len(), 2);
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
            err.errors().contains_key("exporters"),
            "error should name the offending field: {err}"
        );
    }

    #[test]
    fn validate_rejects_a_machines_entry_without_separator() {
        let err = config(&["http://node-a:8080"], &["no-separator"])
            .validate()
            .unwrap_err();

        assert!(
            err.errors().contains_key("machines"),
            "error should name the offending field: {err}"
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

    #[test]
    fn validate_rejects_an_exporter_without_a_scheme() {
        let err = config(&["node-a:8080"], &["m-a=node-a.example"])
            .validate()
            .unwrap_err();

        assert!(
            err.errors().contains_key("exporters"),
            "error should name the offending field: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_request_timeout() {
        let mut config = config(&["http://node-a:8080"], &["m-a=node-a.example"]);
        config.request_timeout_secs = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_max_inflight() {
        let mut config = config(&["http://node-a:8080"], &["m-a=node-a.example"]);
        config.max_inflight = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_quote_service_port() {
        let mut config = config(&["http://node-a:8080"], &["m-a=node-a.example"]);
        config.quote_service_port = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_an_empty_server_host() {
        let mut config = config(&["http://node-a:8080"], &["m-a=node-a.example"]);
        config.server.host = String::new();

        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_a_zero_server_port() {
        let mut config = config(&["http://node-a:8080"], &["m-a=node-a.example"]);
        config.server.port = 0;

        assert!(config.validate().is_err());
    }
}
