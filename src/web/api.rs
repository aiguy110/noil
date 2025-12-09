use axum::{Router, routing::get};

pub fn create_router() -> Router {
    Router::new()
        .route("/health", get(health_check))
    // TODO: Add API routes
}

async fn health_check() -> &'static str {
    "OK"
}
