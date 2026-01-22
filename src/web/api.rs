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

use crate::config::diff::create_diff_with_context;
use crate::config::types::FiberTypeConfig;
use crate::config::version::compute_config_hash;
use crate::storage::traits::{ConfigSource, ConfigVersion, FiberRecord, Storage, StorageError, StoredLog};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub fiber_types: Arc<std::collections::HashMap<String, FiberTypeConfig>>,
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
pub struct FiberTypeMetadata {
    pub name: String,
    pub is_source_fiber: bool,
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

/// GET /api/fiber-types
pub async fn list_fiber_types(
    State(state): State<AppState>,
) -> Result<Json<Vec<FiberTypeMetadata>>, ApiError> {
    let types = state.storage.get_all_fiber_types().await?;
    let metadata: Vec<FiberTypeMetadata> = types
        .into_iter()
        .map(|name| {
            let is_source_fiber = state
                .fiber_types
                .get(&name)
                .map(|ft| ft.is_source_fiber)
                .unwrap_or(false);
            FiberTypeMetadata {
                name,
                is_source_fiber,
            }
        })
        .collect();
    Ok(Json(metadata))
}

/// GET /api/sources
pub async fn list_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, ApiError> {
    let sources = state.storage.get_all_source_ids().await?;
    Ok(Json(sources))
}

// ============================================================================
// Config Versioning API
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ConfigVersionDto {
    pub version_hash: String,
    pub parent_hash: Option<String>,
    pub yaml_content: String,
    pub created_at: DateTime<Utc>,
    pub source: String,
    pub is_active: bool,
}

impl From<ConfigVersion> for ConfigVersionDto {
    fn from(version: ConfigVersion) -> Self {
        Self {
            version_hash: version.version_hash,
            parent_hash: version.parent_hash,
            yaml_content: version.yaml_content,
            created_at: version.created_at,
            source: version.source.to_string(),
            is_active: version.is_active,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub yaml_content: String,
}

#[derive(Debug, Serialize)]
pub struct ConfigHistoryResponse {
    pub versions: Vec<ConfigVersionDto>,
    pub total: u64,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Serialize)]
pub struct ConfigDiffResponse {
    pub from: ConfigVersionDto,
    pub to: ConfigVersionDto,
    pub diff: String,
}

/// GET /api/config/current
pub async fn get_current_config(
    State(state): State<AppState>,
) -> Result<Json<ConfigVersionDto>, ApiError> {
    let version = state
        .storage
        .get_active_config_version()
        .await?
        .ok_or_else(|| ApiError::NotFound("No active config version found".to_string()))?;

    Ok(Json(ConfigVersionDto::from(version)))
}

/// GET /api/config/history
pub async fn get_config_history(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ConfigHistoryResponse>, ApiError> {
    let versions = state
        .storage
        .list_config_versions(params.limit(), params.offset())
        .await?;

    let total = state.storage.count_config_versions().await?;

    let versions_dto = versions.into_iter().map(ConfigVersionDto::from).collect();

    Ok(Json(ConfigHistoryResponse {
        versions: versions_dto,
        total,
        limit: params.limit(),
        offset: params.offset(),
    }))
}

/// GET /api/config/versions/:hash
pub async fn get_config_version(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Json<ConfigVersionDto>, ApiError> {
    let version = state
        .storage
        .get_config_version(&hash)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Config version not found: {}", hash)))?;

    Ok(Json(ConfigVersionDto::from(version)))
}

/// PUT /api/config
pub async fn update_config(
    State(state): State<AppState>,
    Json(request): Json<UpdateConfigRequest>,
) -> Result<Json<ConfigVersionDto>, ApiError> {
    // Validate YAML
    serde_yaml::from_str::<serde_yaml::Value>(&request.yaml_content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid YAML: {}", e)))?;

    // Compute hash
    let version_hash = compute_config_hash(&request.yaml_content);

    // Get current active version for parent hash
    let parent_hash = state
        .storage
        .get_active_config_version()
        .await?
        .map(|v| v.version_hash);

    // Create new version (not active - requires restart)
    let new_version = ConfigVersion {
        version_hash,
        parent_hash,
        yaml_content: request.yaml_content,
        created_at: Utc::now(),
        source: ConfigSource::UI,
        is_active: false, // Not active until restart
    };

    // Check if this version already exists
    if state
        .storage
        .get_config_version(&new_version.version_hash)
        .await?
        .is_some()
    {
        return Err(ApiError::BadRequest(
            "Config version already exists (no changes made)".to_string(),
        ));
    }

    // Note: For MVP, we validate the config but don't save it to the database.
    // The user needs to manually update the config file and restart.
    // Future enhancement: Add insert_config_version_inactive method to save without activating.

    Ok(Json(ConfigVersionDto::from(new_version)))
}

/// GET /api/config/diff/:hash1/:hash2
pub async fn get_config_diff(
    State(state): State<AppState>,
    Path((hash1, hash2)): Path<(String, String)>,
) -> Result<Json<ConfigDiffResponse>, ApiError> {
    let version1 = state
        .storage
        .get_config_version(&hash1)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Config version not found: {}", hash1)))?;

    let version2 = state
        .storage
        .get_config_version(&hash2)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Config version not found: {}", hash2)))?;

    let diff = create_diff_with_context(
        &version1.yaml_content,
        &version2.yaml_content,
        &format!("version {}", &hash1[..8]),
        &format!("version {}", &hash2[..8]),
    );

    Ok(Json(ConfigDiffResponse {
        from: ConfigVersionDto::from(version1),
        to: ConfigVersionDto::from(version2),
        diff,
    }))
}
