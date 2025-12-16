use axum::{routing::get, Router};
use std::sync::Arc;
use tokio::sync::watch;

use crate::config::WebConfig;
use crate::storage::Storage;

use super::api::{
    health_check, list_logs, get_log, get_log_fibers,
    list_fibers, get_fiber, get_fiber_logs, AppState,
};

/// Start the web server with the given storage backend and configuration
pub async fn run_server(
    storage: Arc<dyn Storage>,
    config: WebConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = AppState { storage };

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/logs", get(list_logs))
        .route("/api/logs/:id", get(get_log))
        .route("/api/logs/:id/fibers", get(get_log_fibers))
        .route("/api/fibers", get(list_fibers))
        .route("/api/fibers/:id", get(get_fiber))
        .route("/api/fibers/:id/logs", get(get_fiber_logs))
        .with_state(app_state);

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
