use axum::{
    Router,
    routing::get,
};
use tokio::{net::TcpListener, signal};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::handlers;

/// Shared application state injected into every Axum handler via `State`.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
}

/// Top-level application builder and entry point.
///
/// Call `Application::new` with a loaded `Config`, then `Application::run`
/// to initialise all dependencies and start the HTTP server.
pub struct Application {
    config: Config,
}

impl Application {
    /// Creates a new application instance from the provided configuration.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Builds the Axum `Router` with all routes, middleware layers, and shared state.
    fn build_router(state: AppState) -> Router {
        debug!("Building application router");

        let cors = CorsLayer::new()
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ])
            .allow_origin(tower_http::cors::Any);

        Router::new()
            .route("/", get(handlers::root))
            .route("/health", get(handlers::health_check))
            .fallback(handlers::not_found)
            .with_state(state)
            .layer(TraceLayer::new_for_http())
            .layer(cors)
    }

    /// Initialises all dependencies and runs the HTTP server until a shutdown signal.
    pub async fn run(self) -> anyhow::Result<()> {
        let address = self.config.bind_addr();
        let state = AppState {
            config: self.config,
        };

        info!("Starting nox-cvms-exporter-aggregator on {address}");
        let listener = TcpListener::bind(&address).await?;
        axum::serve(listener, Self::build_router(state))
            .with_graceful_shutdown(Self::shutdown_signal())
            .await?;

        Ok(())
    }

    /// Resolves when `SIGTERM` or `Ctrl+C` is received, triggering graceful shutdown.
    async fn shutdown_signal() {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install signal handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                info!("Received Ctrl+C, shutting down gracefully...");
            },
            _ = terminate => {
                info!("Received SIGTERM, shutting down gracefully...");
            },
        }

        warn!("Shutdown signal received, cleaning up...");
    }
}
