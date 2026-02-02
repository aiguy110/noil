use crate::collector::api::{
    acknowledge, get_batches, get_checkpoint, get_status, rewind, CollectorState,
};
use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

/// Start the collector HTTP server
pub async fn start_server(
    listen_addr: SocketAddr,
    state: Arc<CollectorState>,
) -> Result<(), std::io::Error> {
    let app = Router::new()
        .route("/collector/status", get(get_status))
        .route("/collector/batches", get(get_batches))
        .route("/collector/acknowledge", post(acknowledge))
        .route("/collector/rewind", post(rewind))
        .route("/collector/checkpoint", get(get_checkpoint))
        .with_state(state);

    info!(addr = %listen_addr, "Starting collector HTTP server");

    let listener = TcpListener::bind(listen_addr).await?;
    axum::serve(listener, app).await
}
