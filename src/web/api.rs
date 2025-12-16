use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::storage::{Storage, StorageError, StoredLog, FiberRecord};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListLogsParams {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub source: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl ListLogsParams {
    pub fn limit(&self) -> usize {
        self.limit.min(1000)
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

#[derive(Debug, Deserialize)]
pub struct ListFibersParams {
    #[serde(rename = "type")]
    pub fiber_type: Option<String>,
    pub closed: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl ListFibersParams {
    pub fn limit(&self) -> usize {
        self.limit.min(1000)
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl PaginationParams {
    pub fn limit(&self) -> usize {
        self.limit.min(1000)
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

fn default_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub logs: Vec<LogDto>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Serialize)]
pub struct FibersResponse {
    pub fibers: Vec<FiberDto>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Serialize)]
pub struct LogDto {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
    pub raw_text: String,
    pub ingestion_time: DateTime<Utc>,
}

impl From<StoredLog> for LogDto {
    fn from(log: StoredLog) -> Self {
        Self {
            id: log.log_id,
            timestamp: log.timestamp,
            source_id: log.source_id,
            raw_text: log.raw_text,
            ingestion_time: log.ingestion_time,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FiberDto {
    pub id: Uuid,
    pub fiber_type: String,
    pub attributes: serde_json::Value,
    pub first_activity: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub closed: bool,
}

impl From<FiberRecord> for FiberDto {
    fn from(fiber: FiberRecord) -> Self {
        Self {
            id: fiber.fiber_id,
            fiber_type: fiber.fiber_type,
            attributes: fiber.attributes,
            first_activity: fiber.first_activity,
            last_activity: fiber.last_activity,
            closed: fiber.closed,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

// ============================================================================
// Error Handling
// ============================================================================

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", msg),
        };

        let body = Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message,
            },
        });

        (status, body).into_response()
    }
}

impl From<StorageError> for ApiError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound(msg) => ApiError::NotFound(msg),
            _ => ApiError::Internal(err.to_string()),
        }
    }
}

// ============================================================================
// API Handlers
// ============================================================================

/// GET /health
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// GET /api/logs
pub async fn list_logs(
    State(state): State<AppState>,
    Query(params): Query<ListLogsParams>,
) -> Result<Json<LogsResponse>, ApiError> {
    // For now, we'll use a simple time range query
    // If no start/end provided, use a reasonable default
    let start = params.start.unwrap_or_else(|| Utc::now() - chrono::Duration::days(1));
    let end = params.end.unwrap_or_else(|| Utc::now());

    let logs = state
        .storage
        .query_logs_by_time(start, end, params.limit(), params.offset())
        .await?;

    let total = logs.len();
    let logs_dto = logs.into_iter().map(LogDto::from).collect();

    Ok(Json(LogsResponse {
        logs: logs_dto,
        total,
        limit: params.limit(),
        offset: params.offset(),
    }))
}

/// GET /api/logs/:id
pub async fn get_log(
    State(state): State<AppState>,
    Path(log_id): Path<Uuid>,
) -> Result<Json<LogDto>, ApiError> {
    let log = state
        .storage
        .get_log(log_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Log not found: {}", log_id)))?;

    Ok(Json(LogDto::from(log)))
}

/// GET /api/logs/:id/fibers
pub async fn get_log_fibers(
    State(state): State<AppState>,
    Path(log_id): Path<Uuid>,
) -> Result<Json<FibersResponse>, ApiError> {
    // First verify the log exists
    let _log = state
        .storage
        .get_log(log_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Log not found: {}", log_id)))?;

    // Get fiber IDs for this log
    let fiber_ids = state.storage.get_log_fibers(log_id).await?;

    // Fetch each fiber
    let mut fibers = Vec::new();
    for fiber_id in fiber_ids {
        if let Some(fiber) = state.storage.get_fiber(fiber_id).await? {
            fibers.push(FiberDto::from(fiber));
        }
    }

    let total = fibers.len();

    Ok(Json(FibersResponse {
        fibers,
        total,
        limit: usize::MAX,
        offset: 0,
    }))
}

/// GET /api/fibers
pub async fn list_fibers(
    State(state): State<AppState>,
    Query(params): Query<ListFibersParams>,
) -> Result<Json<FibersResponse>, ApiError> {
    // If no fiber type specified, we need a method to list all fibers
    // For now, return an error if no type is specified
    let fiber_type = params
        .fiber_type
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("fiber type parameter required".to_string()))?;

    let fibers = state
        .storage
        .query_fibers_by_type(fiber_type, params.limit(), params.offset())
        .await?;

    // Apply closed filter if specified
    let fibers: Vec<FiberDto> = fibers
        .into_iter()
        .map(FiberDto::from)
        .filter(|f| params.closed.map_or(true, |closed| f.closed == closed))
        .collect();

    let total = fibers.len();

    Ok(Json(FibersResponse {
        fibers,
        total,
        limit: params.limit(),
        offset: params.offset(),
    }))
}

/// GET /api/fibers/:id
pub async fn get_fiber(
    State(state): State<AppState>,
    Path(fiber_id): Path<Uuid>,
) -> Result<Json<FiberDto>, ApiError> {
    let fiber = state
        .storage
        .get_fiber(fiber_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Fiber not found: {}", fiber_id)))?;

    Ok(Json(FiberDto::from(fiber)))
}

/// GET /api/fibers/:id/logs
pub async fn get_fiber_logs(
    State(state): State<AppState>,
    Path(fiber_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<LogsResponse>, ApiError> {
    // First verify the fiber exists
    let _fiber = state
        .storage
        .get_fiber(fiber_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Fiber not found: {}", fiber_id)))?;

    let logs = state
        .storage
        .get_fiber_logs(fiber_id, params.limit(), params.offset())
        .await?;

    let total = logs.len();
    let logs_dto = logs.into_iter().map(LogDto::from).collect();

    Ok(Json(LogsResponse {
        logs: logs_dto,
        total,
        limit: params.limit(),
        offset: params.offset(),
    }))
}
