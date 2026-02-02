use crate::collector::batch::LogBatch;
use crate::collector::batch_buffer::BufferStats;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for the collector API
pub struct CollectorState {
    pub collector_id: String,
    pub version: String,
    pub start_time: std::time::Instant,
    pub buffer_stats: Arc<RwLock<BufferStats>>,
    pub watermark: Arc<RwLock<Option<DateTime<Utc>>>>,
    pub source_watermarks: Arc<RwLock<Vec<SourceInfo>>>,
    pub batches_fn: Arc<dyn Fn(Option<u64>, usize) -> Vec<LogBatch> + Send + Sync>,
    pub acknowledge_fn: Arc<dyn Fn(Vec<u64>) -> usize + Send + Sync>,
    pub rewind_fn: Arc<dyn Fn(Option<u64>, bool) -> RewindResult + Send + Sync>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub watermark: Option<DateTime<Utc>>,
    pub active: bool,
}

// API response types
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub collector_id: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub buffer_status: BufferStatusInfo,
    pub watermark: Option<DateTime<Utc>>,
    pub sources: Vec<SourceInfo>,
}

#[derive(Debug, Serialize)]
pub struct BufferStatusInfo {
    pub current_epochs: usize,
    pub max_epochs: usize,
    pub oldest_sequence: u64,
    pub newest_sequence: u64,
}

#[derive(Debug, Serialize)]
pub struct BatchesResponse {
    pub batches: Vec<LogBatch>,
    pub has_more: bool,
    pub next_sequence: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BatchesQuery {
    #[serde(default)]
    pub after: Option<u64>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Deserialize)]
pub struct AcknowledgeRequest {
    pub sequence_nums: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct AcknowledgeResponse {
    pub acknowledged_count: usize,
    pub freed_buffer_space: usize,
}

#[derive(Debug, Deserialize)]
pub struct RewindRequest {
    pub target_sequence: Option<u64>,
    #[serde(default)]
    pub preserve_buffer: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct RewindResult {
    pub old_sequence: u64,
    pub new_sequence: u64,
    pub buffer_cleared: bool,
}

#[derive(Debug, Serialize)]
pub struct CheckpointResponse {
    pub message: String,
}

/// GET /collector/status
pub async fn get_status(
    State(state): State<Arc<CollectorState>>,
) -> Result<Json<StatusResponse>, ApiError> {
    let buffer_stats = state.buffer_stats.read().await.clone();
    let watermark = *state.watermark.read().await;
    let source_watermarks = state.source_watermarks.read().await.clone();

    let uptime = state.start_time.elapsed().as_secs();

    Ok(Json(StatusResponse {
        collector_id: state.collector_id.clone(),
        version: state.version.clone(),
        uptime_seconds: uptime,
        buffer_status: BufferStatusInfo {
            current_epochs: buffer_stats.current_epochs,
            max_epochs: buffer_stats.max_epochs,
            oldest_sequence: buffer_stats.oldest_sequence,
            newest_sequence: buffer_stats.newest_sequence,
        },
        watermark,
        sources: source_watermarks,
    }))
}

/// GET /collector/batches?after=N&limit=M
/// If 'after' is None, returns batches from the beginning (sequence_num >= 0)
/// If 'after' is Some(n), returns batches with sequence_num > n
pub async fn get_batches(
    State(state): State<Arc<CollectorState>>,
    axum::extract::Query(query): axum::extract::Query<BatchesQuery>,
) -> Result<Json<BatchesResponse>, ApiError> {
    // Validate limit
    let limit = query.limit.min(100).max(1);

    // Get batches using the provided function (None means from beginning)
    let batches = (state.batches_fn)(query.after, limit);

    // Determine if there are more batches
    let has_more = batches.len() == limit;
    let next_sequence = batches.last().map(|b| b.sequence_num);

    Ok(Json(BatchesResponse {
        batches,
        has_more,
        next_sequence,
    }))
}

/// POST /collector/acknowledge
pub async fn acknowledge(
    State(state): State<Arc<CollectorState>>,
    Json(request): Json<AcknowledgeRequest>,
) -> Result<Json<AcknowledgeResponse>, ApiError> {
    if request.sequence_nums.is_empty() {
        return Err(ApiError::BadRequest(
            "sequence_nums cannot be empty".to_string(),
        ));
    }

    let count = (state.acknowledge_fn)(request.sequence_nums);

    Ok(Json(AcknowledgeResponse {
        acknowledged_count: count,
        freed_buffer_space: count, // After compaction, this many will be freed
    }))
}

/// POST /collector/rewind
pub async fn rewind(
    State(state): State<Arc<CollectorState>>,
    Json(request): Json<RewindRequest>,
) -> Result<Json<RewindResult>, ApiError> {
    let result = (state.rewind_fn)(request.target_sequence, request.preserve_buffer);

    Ok(Json(result))
}

/// GET /collector/checkpoint
pub async fn get_checkpoint(
    State(_state): State<Arc<CollectorState>>,
) -> Result<Json<CheckpointResponse>, ApiError> {
    // TODO(phase-4): Implement checkpoint retrieval
    // This is a placeholder for Phase 4 checkpoint support
    Ok(Json(CheckpointResponse {
        message: "Checkpoint support not yet implemented".to_string(),
    }))
}

// Error handling
#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    InternalError(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::InternalError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
