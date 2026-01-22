use axum::{routing::{get, put}, Router};
use std::sync::Arc;
use tokio::sync::watch;
use tower_http::services::ServeDir;

use crate::config::{types::FiberTypeConfig, WebConfig};
use crate::storage::Storage;

use super::api::{
    get_config_diff, get_config_history, get_config_version, get_current_config, get_fiber,
    get_fiber_logs, get_log, get_log_fibers, health_check, list_fiber_types, list_fibers,
    list_logs, list_sources, update_config, AppState,
};

/// Start the web server with the given storage backend and configuration
pub async fn run_server(
    storage: Arc<dyn Storage>,
    fiber_types: Arc<std::collections::HashMap<String, FiberTypeConfig>>,
    config: WebConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = AppState { storage, fiber_types };

    // API routes
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/api/logs", get(list_logs))
        .route("/api/logs/:id", get(get_log))
        .route("/api/logs/:id/fibers", get(get_log_fibers))
        .route("/api/fibers", get(list_fibers))
        .route("/api/fibers/:id", get(get_fiber))
        .route("/api/fibers/:id/logs", get(get_fiber_logs))
        .route("/api/fiber-types", get(list_fiber_types))
        .route("/api/sources", get(list_sources))
        .route("/api/config/current", get(get_current_config))
        .route("/api/config/history", get(get_config_history))
        .route("/api/config/versions/:hash", get(get_config_version))
        .route("/api/config", put(update_config))
        .route("/api/config/diff/:hash1/:hash2", get(get_config_diff))
        .with_state(app_state);

    // Serve static frontend files
    let frontend_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frontend");
    let serve_dir = ServeDir::new(frontend_dir);

    // Combine routes: API first, then static files
    let app = api_routes.fallback_service(serve_dir);

    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!("Web server listening on {}", config.listen);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.wait_for(|&v| v).await;
            tracing::info!("Web server shutting down gracefully");
        })
        .await?;

    Ok(())
}
