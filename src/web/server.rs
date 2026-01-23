use axum::{routing::{get, post, put}, Router};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};
use tower_http::services::ServeDir;

use crate::config::{types::Config, WebConfig};
use crate::fiber::processor::FiberProcessor;
use crate::reprocessing::ReprocessState;
use crate::storage::Storage;

use super::api::{
    cancel_reprocessing, create_fiber_type, delete_fiber_type, get_config_diff,
    get_config_history, get_config_version, get_current_config, get_fiber, get_fiber_logs,
    get_fiber_type, get_log, get_log_fibers, get_reprocess_status, health_check,
    hot_reload_fiber_type, list_fiber_types, list_fibers, list_logs, list_sources,
    start_reprocessing, update_config, update_fiber_type, AppState,
};

/// Start the web server with the given storage backend and configuration
pub async fn run_server(
    storage: Arc<dyn Storage>,
    fiber_processor: Arc<RwLock<FiberProcessor>>,
    config: Arc<RwLock<Config>>,
    config_version: Arc<RwLock<u64>>,
    reprocess_state: Arc<RwLock<Option<ReprocessState>>>,
    config_path: PathBuf,
    config_yaml: Arc<RwLock<String>>,
    web_config: WebConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Extract fiber_types from config for backwards compatibility
    let fiber_types = {
        let config_guard = config.read().await;
        Arc::new(config_guard.fiber_types.clone())
    };

    let app_state = AppState {
        storage,
        fiber_types,
        fiber_processor,
        config,
        config_version,
        config_path,
        config_yaml,
        reprocess_state,
    };

    // API routes
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/api/logs", get(list_logs))
        .route("/api/logs/:id", get(get_log))
        .route("/api/logs/:id/fibers", get(get_log_fibers))
        .route("/api/fibers", get(list_fibers))
        .route("/api/fibers/:id", get(get_fiber))
        .route("/api/fibers/:id/logs", get(get_fiber_logs))
        .route("/api/fiber-types", get(list_fiber_types).post(create_fiber_type))
        .route("/api/fiber-types/:name", get(get_fiber_type).put(update_fiber_type).delete(delete_fiber_type))
        .route("/api/fiber-types/:name/hot-reload", post(hot_reload_fiber_type))
        .route("/api/sources", get(list_sources))
        .route("/api/config/current", get(get_current_config))
        .route("/api/config/history", get(get_config_history))
        .route("/api/config/versions/:hash", get(get_config_version))
        .route("/api/config", put(update_config))
        .route("/api/config/diff/:hash1/:hash2", get(get_config_diff))
        .route("/api/reprocess", post(start_reprocessing))
        .route("/api/reprocess/status", get(get_reprocess_status))
        .route("/api/reprocess/cancel", post(cancel_reprocessing))
        .with_state(app_state);

    // Serve static frontend files
    let frontend_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frontend");
    let serve_dir = ServeDir::new(frontend_dir);

    // Combine routes: API first, then static files
    let app = api_routes.fallback_service(serve_dir);

    let listener = tokio::net::TcpListener::bind(&web_config.listen).await?;
    tracing::info!("Web server listening on {}", web_config.listen);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.wait_for(|&v| v).await;
            tracing::info!("Web server shutting down gracefully");
        })
        .await?;

    Ok(())
}
