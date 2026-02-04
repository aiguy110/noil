use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::diff::create_diff_with_context;
use crate::config::types::{Config, FiberTypeConfig, OperationMode};
use crate::config::version::compute_config_hash;
use crate::fiber::processor::FiberProcessor;
use crate::reprocessing::{ReprocessProgress, ReprocessState, ReprocessStatus};
use crate::storage::traits::{ConfigSource, ConfigVersion, FiberRecord, Storage, StorageError, StoredLog};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub fiber_types: Arc<std::collections::HashMap<String, FiberTypeConfig>>,
    pub fiber_processor: Arc<RwLock<FiberProcessor>>,
    pub config: Arc<RwLock<Config>>,
    pub config_version: Arc<RwLock<u64>>,
    pub config_path: PathBuf,
    pub config_yaml: Arc<RwLock<String>>,
    pub reprocess_state: Arc<RwLock<Option<ReprocessState>>>,
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

#[derive(Debug, Deserialize)]
pub struct LogsBatchRequest {
    pub log_ids: Vec<Uuid>,
    #[serde(default)]
    pub include_fiber_membership: bool,
}

#[derive(Debug, Serialize)]
pub struct LogsBatchResponse {
    pub logs: Vec<LogDto>,
    pub missing_log_ids: Vec<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fiber_memberships: Option<HashMap<Uuid, Vec<FiberDto>>>,
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

// ============================================================================
// Filtered Fiber Query Types
// ============================================================================

fn default_max_fibers() -> usize {
    200
}

#[derive(Debug, Deserialize)]
pub struct FilteredFibersParams {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
    pub closed: Option<bool>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    #[serde(default = "default_max_fibers")]
    pub max_fibers: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Serialize)]
pub struct FilteredFibersResponse {
    pub fibers: Vec<FiberDto>,
    pub total_matching: usize,
    pub truncated: bool,
}

// ============================================================================
// Fiber Membership Summary Types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct LogPoint {
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
}

#[derive(Debug, Serialize)]
pub struct FiberMembershipSummary {
    pub fiber_id: Uuid,
    pub log_points: Vec<LogPoint>,
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
// Fiber Type Management Types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct FiberTypeResponse {
    pub name: String,
    pub yaml_content: String,
    pub is_source_fiber: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFiberTypeRequest {
    pub yaml_content: String,
}

#[derive(Debug, Serialize)]
pub struct UpdateFiberTypeResponse {
    pub new_version_hash: String,
    pub validation_warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFiberTypeRequest {
    pub name: String,
    pub yaml_content: String,
}

#[derive(Debug, Serialize)]
pub struct CreateFiberTypeResponse {
    pub name: String,
    pub new_version_hash: String,
}

#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub message: String,
}

// ============================================================================
// Working Set Testing Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct TestWorkingSetRequest {
    pub log_ids: Vec<Uuid>,
    pub yaml_content: String,
    pub include_margin: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TestWorkingSetResponse {
    pub expected_logs: Vec<LogDto>,
    pub time_window: TimeWindowDto,
    pub fibers_generated: Vec<FiberMatchResult>,
    pub best_match_index: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct FiberMatchResult {
    pub fiber_id: Uuid,
    pub iou: f64,
    pub matching_logs: Vec<Uuid>,
    pub missing_logs: Vec<Uuid>,
    pub extra_log_ids: Vec<Uuid>,
    pub logs: Vec<LogDto>,
}

#[derive(Debug, Serialize)]
pub struct TimeWindowDto {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

// ============================================================================
// Reprocessing Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct StartReprocessRequest {
    pub time_range: Option<TimeRange>,
    pub clear_old_results: bool,
}

#[derive(Debug, Serialize)]
pub struct StartReprocessResponse {
    pub task_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ReprocessStatusResponse {
    pub status: String,
    pub progress: Option<ReprocessProgress>,
    pub error: Option<String>,
}

// ============================================================================
// Error Handling
// ============================================================================

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
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

/// POST /api/logs/batch
pub async fn get_logs_batch(
    State(state): State<AppState>,
    Json(request): Json<LogsBatchRequest>,
) -> Result<Json<LogsBatchResponse>, ApiError> {
    if request.log_ids.is_empty() {
        return Err(ApiError::BadRequest("log_ids cannot be empty".to_string()));
    }

    if request.log_ids.len() > 100 {
        return Err(ApiError::BadRequest(
            "log_ids cannot exceed 100".to_string(),
        ));
    }

    let logs = state.storage.get_logs_by_ids(&request.log_ids).await?;
    let mut log_map = HashMap::new();
    for log in logs {
        log_map.insert(log.log_id, log);
    }

    let mut ordered_logs = Vec::new();
    let mut missing_log_ids = Vec::new();
    for log_id in &request.log_ids {
        if let Some(log) = log_map.get(log_id) {
            ordered_logs.push(LogDto::from(log.clone()));
        } else {
            missing_log_ids.push(*log_id);
        }
    }

    let fiber_memberships = if request.include_fiber_membership {
        let mut memberships = HashMap::new();
        for log_id in &request.log_ids {
            if !log_map.contains_key(log_id) {
                continue;
            }
            let fiber_ids = state.storage.get_log_fibers(*log_id).await?;
            let mut fibers = Vec::new();
            for fiber_id in fiber_ids {
                if let Some(fiber) = state.storage.get_fiber(fiber_id).await? {
                    fibers.push(FiberDto::from(fiber));
                }
            }
            memberships.insert(*log_id, fibers);
        }
        Some(memberships)
    } else {
        None
    };

    Ok(Json(LogsBatchResponse {
        logs: ordered_logs,
        missing_log_ids,
        fiber_memberships,
    }))
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

/// POST /api/fibers/query - Filtered fiber query with time overlap, attribute filters, etc.
pub async fn query_fibers_filtered(
    State(state): State<AppState>,
    Json(params): Json<FilteredFibersParams>,
) -> Result<Json<FilteredFibersResponse>, ApiError> {
    let fiber_types = if params.types.is_empty() {
        None
    } else {
        Some(params.types.as_slice())
    };

    let max_fibers = params.max_fibers.min(1000);

    let (fibers, total_matching) = state
        .storage
        .query_fibers_filtered(
            fiber_types,
            &params.attributes,
            params.closed,
            params.start_time,
            params.end_time,
            max_fibers,
            params.offset,
        )
        .await?;

    let truncated = total_matching > params.offset + fibers.len();

    let fibers_dto: Vec<FiberDto> = fibers.into_iter().map(FiberDto::from).collect();

    Ok(Json(FilteredFibersResponse {
        fibers: fibers_dto,
        total_matching,
        truncated,
    }))
}

/// POST /api/fibers/membership-summaries - Get simplified log points for fibers
pub async fn get_fiber_membership_summaries(
    State(state): State<AppState>,
    Json(fiber_ids): Json<Vec<Uuid>>,
) -> Result<Json<Vec<FiberMembershipSummary>>, ApiError> {
    if fiber_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    if fiber_ids.len() > 500 {
        return Err(ApiError::BadRequest(
            "fiber_ids cannot exceed 500".to_string(),
        ));
    }

    let log_points_map = state.storage.get_fiber_log_points(&fiber_ids).await?;

    let summaries: Vec<FiberMembershipSummary> = fiber_ids
        .into_iter()
        .map(|fiber_id| {
            let points = log_points_map
                .get(&fiber_id)
                .cloned()
                .unwrap_or_default();
            let simplified = simplify_log_points(points);
            FiberMembershipSummary {
                fiber_id,
                log_points: simplified,
            }
        })
        .collect();

    Ok(Json(summaries))
}

/// Simplify log points to only include start, end, and transition points
fn simplify_log_points(points: Vec<(DateTime<Utc>, String)>) -> Vec<LogPoint> {
    if points.is_empty() {
        return vec![];
    }

    if points.len() == 1 {
        return vec![LogPoint {
            timestamp: points[0].0,
            source_id: points[0].1.clone(),
        }];
    }

    fn push_unique(result: &mut Vec<LogPoint>, timestamp: DateTime<Utc>, source_id: &str) {
        if result
            .last()
            .map(|point| point.timestamp == timestamp && point.source_id == source_id)
            .unwrap_or(false)
        {
            return;
        }
        result.push(LogPoint {
            timestamp,
            source_id: source_id.to_string(),
        });
    }

    let mut result = Vec::new();
    push_unique(&mut result, points[0].0, &points[0].1);

    for i in 1..points.len() {
        let prev = &points[i - 1];
        let curr = &points[i];
        if curr.1 != prev.1 {
            // Source changed - include both sides of the transition
            push_unique(&mut result, prev.0, &prev.1);
            push_unique(&mut result, curr.0, &curr.1);
        }
    }

    // Always include last point
    let last = points.last().unwrap();
    push_unique(&mut result, last.0, &last.1);

    result
}

#[cfg(test)]
mod tests {
    use super::simplify_log_points;
    use chrono::{DateTime, Utc};

    fn ts(micros: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_micros(micros).unwrap()
    }

    #[test]
    fn simplify_log_points_includes_both_sides_of_transitions() {
        let points = vec![
            (ts(1), "source_a".to_string()),
            (ts(2), "source_a".to_string()),
            (ts(3), "source_b".to_string()),
            (ts(4), "source_b".to_string()),
            (ts(5), "source_c".to_string()),
        ];

        let simplified = simplify_log_points(points);
        let simplified_pairs: Vec<(i64, String)> = simplified
            .iter()
            .map(|point| (point.timestamp.timestamp_micros(), point.source_id.clone()))
            .collect();

        assert_eq!(
            simplified_pairs,
            vec![
                (1, "source_a".to_string()),
                (2, "source_a".to_string()),
                (3, "source_b".to_string()),
                (4, "source_b".to_string()),
                (5, "source_c".to_string())
            ]
        );
    }
}

/// GET /api/fiber-types
pub async fn list_fiber_types(
    State(state): State<AppState>,
) -> Result<Json<Vec<FiberTypeMetadata>>, ApiError> {
    // Return fiber types from configuration, not from storage
    // This ensures fiber types are visible even before any fibers are created
    let config = state.config.read().await;
    let mut metadata: Vec<FiberTypeMetadata> = config
        .fiber_types
        .iter()
        .map(|(name, ft)| FiberTypeMetadata {
            name: name.clone(),
            is_source_fiber: ft.is_source_fiber,
        })
        .collect();

    // In parent mode (or when auto_source_fibers is enabled),
    // also include source fiber types for all sources that have sent logs.
    // This ensures that source fibers from collectors are visible in the UI,
    // even if they're not explicitly defined in the parent config.
    if config.auto_source_fibers || config.mode == OperationMode::Parent {
        // Get all unique source IDs from the database
        let source_ids = state.storage.get_all_source_ids().await?;

        // Create synthetic source fiber metadata for sources not already in config
        for source_id in source_ids {
            if !config.fiber_types.contains_key(&source_id) {
                metadata.push(FiberTypeMetadata {
                    name: source_id,
                    is_source_fiber: true,
                });
            }
        }
    }

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

    // Save the config version to the database
    // Note: This creates a new inactive version. To activate it, use the activate endpoint.
    // Global config changes (sources, pipeline, storage, web) require activation to take effect.
    // Fiber_types can be individually hot-reloaded or activated as a batch.
    state.storage.insert_config_version(&new_version).await?;

    // Also update the in-memory YAML for consistency
    {
        let mut config_yaml_guard = state.config_yaml.write().await;
        *config_yaml_guard = new_version.yaml_content.clone();
    }

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

/// POST /api/config/activate/:hash
/// Activates a config version by hash, marking it active and hot-reloading all fiber types
pub async fn activate_config_version(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    // 1. Check if reprocessing is running
    {
        let reprocess_guard = state.reprocess_state.read().await;
        if let Some(reprocess) = reprocess_guard.as_ref() {
            if matches!(reprocess.status, ReprocessStatus::Running) {
                return Err(ApiError::Conflict(
                    "Cannot activate config while reprocessing is running".to_string(),
                ));
            }
        }
    }

    // 2. Get the config version to activate
    let config_version = state
        .storage
        .get_config_version(&hash)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Config version not found: {}", hash)))?;

    // 3. Parse the YAML into a Config struct to validate it
    let new_config: Config = serde_yaml::from_str(&config_version.yaml_content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid config YAML: {}", e)))?;

    // 4. Acquire write locks (blocks in-flight log processing)
    let mut processor_guard = state.fiber_processor.write().await;
    let mut version_guard = state.config_version.write().await;
    let mut config_guard = state.config.write().await;

    // 5. Flush old processor (closes all open fibers)
    let flush_results = processor_guard.flush();

    // Write flush results to storage
    for result in flush_results {
        for fiber in &result.new_fibers {
            state.storage.write_fiber(fiber).await?;
        }
        for fiber in &result.updated_fibers {
            state.storage.update_fiber(fiber).await?;
        }
        if !result.memberships.is_empty() {
            state.storage.write_memberships(&result.memberships).await?;
        }
    }

    // 6. Compute new version number from hash
    let new_version = hash
        .parse::<u64>()
        .unwrap_or_else(|_| config_version.yaml_content.as_bytes().iter().map(|&b| b as u64).sum());

    // 7. Create new processor with the new config
    let new_processor = FiberProcessor::from_config(&new_config, new_version)
        .map_err(|e| ApiError::Internal(format!("Failed to create processor: {}", e)))?;

    // 8. Replace processor, config, and version
    *processor_guard = new_processor;
    *config_guard = new_config;
    *version_guard = new_version;

    // 9. Update in-memory YAML
    {
        let mut config_yaml_guard = state.config_yaml.write().await;
        *config_yaml_guard = config_version.yaml_content.clone();
    }

    // 10. Mark this version as active in database
    state.storage.mark_config_active(&hash).await?;

    Ok(Json(SuccessResponse {
        message: format!("Config version {} activated successfully", &hash[..8]),
    }))
}

// ============================================================================
// Fiber Type Management API
// ============================================================================

/// GET /api/fiber-types/:name
pub async fn get_fiber_type(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<FiberTypeResponse>, ApiError> {
    let config = state.config.read().await;
    let fiber_type = config
        .fiber_types
        .get(&name)
        .ok_or_else(|| ApiError::NotFound(format!("Fiber type not found: {}", name)))?;

    let is_source_fiber = fiber_type.is_source_fiber;

    // Get the YAML from the active database version (source of truth after UI changes)
    // This ensures we show the latest config, not the file version from startup
    let active_version = state
        .storage
        .get_active_config_version()
        .await?
        .ok_or_else(|| ApiError::Internal("No active config version found".to_string()))?;

    let yaml = extract_fiber_type_yaml_with_name(&active_version.yaml_content, &name)?;

    Ok(Json(FiberTypeResponse {
        name,
        yaml_content: yaml,
        is_source_fiber,
    }))
}

/// GET /api/config/versions/:hash/fiber_types/:name
/// Get a fiber type from a specific config version (preserving original YAML)
pub async fn get_fiber_type_from_version(
    State(state): State<AppState>,
    Path((hash, name)): Path<(String, String)>,
) -> Result<Json<FiberTypeResponse>, ApiError> {
    // Get the specified config version
    let version = state
        .storage
        .get_config_version(&hash)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Config version '{}' not found", hash)))?;

    // Extract the fiber type YAML while preserving formatting
    let yaml = extract_fiber_type_yaml_with_name(&version.yaml_content, &name)?;

    // Parse to check if it's a source fiber (just for the flag, not for modification)
    let config: Config = serde_yaml::from_str(&version.yaml_content)
        .map_err(|e| ApiError::Internal(format!("Failed to parse config: {}", e)))?;

    let is_source_fiber = config
        .fiber_types
        .get(&name)
        .map(|ft| ft.is_source_fiber)
        .unwrap_or(false);

    Ok(Json(FiberTypeResponse {
        name,
        yaml_content: yaml,
        is_source_fiber,
    }))
}

/// PUT /api/fiber-types/:name
pub async fn update_fiber_type(
    State(state): State<AppState>,
    Path(original_name): Path<String>,
    Json(req): Json<UpdateFiberTypeRequest>,
) -> Result<Json<UpdateFiberTypeResponse>, ApiError> {
    // 1. Check if this is an auto-generated source fiber
    let config = state.config.read().await;
    if let Some(fiber_type) = config.fiber_types.get(&original_name) {
        if fiber_type.is_source_fiber {
            return Err(ApiError::BadRequest(
                "Cannot edit auto-generated source fiber types. These are automatically created from sources.".to_string()
            ));
        }
    } else {
        return Err(ApiError::NotFound(format!("Fiber type '{}' not found in config", original_name)));
    }
    drop(config);

    // 2. Parse the incoming YAML to extract the fiber type name and config
    // The YAML should be in the format "fiber_name:\n  description: ..."
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&req.yaml_content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid YAML: {}", e)))?;

    let yaml_map = yaml_value.as_mapping()
        .ok_or_else(|| ApiError::BadRequest("YAML must be a mapping with one fiber type".to_string()))?;

    if yaml_map.len() != 1 {
        return Err(ApiError::BadRequest(
            "YAML must contain exactly one fiber type definition".to_string()
        ));
    }

    let (new_name_value, fiber_config_value) = yaml_map.iter().next().unwrap();
    let new_name = new_name_value.as_str()
        .ok_or_else(|| ApiError::BadRequest("Fiber type name must be a string".to_string()))?
        .to_string();

    // Validate the fiber type configuration
    let new_fiber_type: FiberTypeConfig = serde_yaml::from_value(fiber_config_value.clone())
        .map_err(|e| ApiError::BadRequest(format!("Invalid fiber type config: {}", e)))?;

    // 3. Get the current config YAML string
    let mut config_yaml_guard = state.config_yaml.write().await;
    let current_yaml = config_yaml_guard.clone();

    // 4. Handle rename vs update
    let updated_yaml = if new_name != original_name {
        // Rename: delete old fiber type and add new one
        tracing::info!(
            old_name = %original_name,
            new_name = %new_name,
            "Renaming fiber type"
        );

        // Check if target name already exists
        let temp_config: Config = serde_yaml::from_str(&current_yaml)
            .map_err(|e| ApiError::Internal(format!("Failed to parse config: {}", e)))?;
        if temp_config.fiber_types.contains_key(&new_name) {
            return Err(ApiError::BadRequest(format!(
                "Cannot rename: fiber type '{}' already exists",
                new_name
            )));
        }

        let yaml_after_delete = delete_fiber_type_from_yaml(&current_yaml, &original_name)?;
        add_fiber_type_to_yaml(&yaml_after_delete, &new_name, &req.yaml_content)?
    } else {
        // Update: replace existing fiber type
        update_fiber_type_in_yaml(&current_yaml, &original_name, &req.yaml_content)?
    };

    // 5. Parse the updated YAML to validate it
    let _updated_config: Config = serde_yaml::from_str(&updated_yaml)
        .map_err(|e| ApiError::Internal(format!("Failed to parse updated config: {}", e)))?;

    // 6. Compute new version hash
    let new_hash = compute_config_hash(&updated_yaml);

    // 7. Update in-memory state
    *config_yaml_guard = updated_yaml.clone();
    let mut config = state.config.write().await;

    if new_name != original_name {
        // Remove old name, add new name
        config.fiber_types.remove(&original_name);
    }
    config.fiber_types.insert(new_name.clone(), new_fiber_type);

    // 8. Write new config version to database (DB-only, no file write)
    // Check if this version already exists (same YAML hash)
    if state.storage.get_config_version(&new_hash).await?.is_some() {
        // Version already exists - just update its timestamp
        state.storage.touch_config_version(&new_hash).await?;
    } else {
        // New version - insert it
        let parent_hash = state
            .storage
            .get_active_config_version()
            .await?
            .map(|v| v.version_hash);

        let config_version = ConfigVersion {
            version_hash: new_hash.clone(),
            parent_hash,
            yaml_content: updated_yaml,
            created_at: Utc::now(),
            source: ConfigSource::UI,
            is_active: false, // Not active until hot-reload
        };

        state.storage.insert_config_version(&config_version).await?;
    }

    Ok(Json(UpdateFiberTypeResponse {
        new_version_hash: new_hash,
        validation_warnings: vec![],
    }))
}

/// POST /api/fiber-types/:name/hot-reload
pub async fn hot_reload_fiber_type(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    // 1. Check if reprocessing is running
    {
        let reprocess_guard = state.reprocess_state.read().await;
        if let Some(reprocess) = reprocess_guard.as_ref() {
            if matches!(reprocess.status, ReprocessStatus::Running) {
                return Err(ApiError::Conflict(
                    "Cannot hot-reload while reprocessing is running".to_string(),
                ));
            }
        }
    }

    // 2. Acquire write locks (blocks in-flight log processing)
    let mut processor_guard = state.fiber_processor.write().await;
    let mut version_guard = state.config_version.write().await;
    let config_guard = state.config.read().await;

    // 3. Flush old processor (closes all open fibers)
    let flush_results = processor_guard.flush();

    // Write flush results to storage
    for result in flush_results {
        for fiber in &result.new_fibers {
            state.storage.write_fiber(fiber).await?;
        }
        for fiber in &result.updated_fibers {
            state.storage.update_fiber(fiber).await?;
        }
        if !result.memberships.is_empty() {
            state.storage.write_memberships(&result.memberships).await?;
        }
    }

    // 4. Get the current in-memory config YAML (which contains any UI changes made since startup)
    // CRITICAL: We must use the in-memory YAML string that was updated by update_fiber_type,
    // NOT fetch from the database, because the database still has the old active version.
    // The in-memory YAML is the source of truth for pending changes.
    // See specs/CONFIG_SYSTEM.md for details on the YAML-as-ground-truth paradigm.
    let yaml = {
        let config_yaml_guard = state.config_yaml.read().await;
        config_yaml_guard.clone()
    };

    // Compute hash from the in-memory YAML
    let new_hash = compute_config_hash(&yaml);

    // Verify this version exists in the database (it should, because update_fiber_type saved it)
    if state.storage.get_config_version(&new_hash).await?.is_none() {
        return Err(ApiError::Internal(format!(
            "Config version {} not found in database. Save changes before hot reloading.",
            &new_hash[..8]
        )));
    }
    let new_version = new_hash
        .parse::<u64>()
        .unwrap_or_else(|_| yaml.as_bytes().iter().map(|&b| b as u64).sum());

    // 5. Create new processor
    let new_processor = FiberProcessor::from_config(&*config_guard, new_version)
        .map_err(|e| ApiError::Internal(format!("Failed to create processor: {}", e)))?;

    // 6. Replace processor and version
    *processor_guard = new_processor;
    *version_guard = new_version;

    // 7. Mark new version as active in database
    state.storage.mark_config_active(&new_hash).await?;

    Ok(Json(SuccessResponse {
        message: format!("Hot reload complete for fiber type '{}'", name),
    }))
}

/// DELETE /api/fiber-types/:name
pub async fn delete_fiber_type(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    // 1. Check if fiber type exists
    let config = state.config.read().await;
    if !config.fiber_types.contains_key(&name) {
        return Err(ApiError::NotFound(format!(
            "Fiber type not found: {}",
            name
        )));
    }

    // Check if it's a source fiber (auto-generated)
    if let Some(fiber_type) = config.fiber_types.get(&name) {
        if fiber_type.is_source_fiber {
            return Err(ApiError::BadRequest(
                "Cannot delete auto-generated source fiber types".to_string(),
            ));
        }
    }
    drop(config);

    // 2. Get the current config YAML string
    let mut config_yaml_guard = state.config_yaml.write().await;
    let current_yaml = config_yaml_guard.clone();

    // 3. Delete the fiber type from the YAML string (preserves comments)
    let updated_yaml = delete_fiber_type_from_yaml(&current_yaml, &name)?;

    // 4. Parse the updated YAML to validate it
    let _updated_config: Config = serde_yaml::from_str(&updated_yaml)
        .map_err(|e| ApiError::Internal(format!("Failed to parse updated config: {}", e)))?;

    // 5. Compute new version hash
    let new_hash = compute_config_hash(&updated_yaml);

    // 6. Update in-memory state
    *config_yaml_guard = updated_yaml.clone();
    let mut config = state.config.write().await;
    config.fiber_types.remove(&name);

    // 7. Write new config version to database (DB-only, no file write)
    // Check if this version already exists (same YAML hash)
    if state.storage.get_config_version(&new_hash).await?.is_some() {
        // Version already exists - just update its timestamp
        state.storage.touch_config_version(&new_hash).await?;
    } else {
        // New version - insert it
        let parent_hash = state
            .storage
            .get_active_config_version()
            .await?
            .map(|v| v.version_hash);

        let config_version = ConfigVersion {
            version_hash: new_hash.clone(),
            parent_hash,
            yaml_content: updated_yaml,
            created_at: Utc::now(),
            source: ConfigSource::UI,
            is_active: false,
        };

        state.storage.insert_config_version(&config_version).await?;
    }

    Ok(Json(SuccessResponse {
        message: format!("Fiber type '{}' deleted (hot-reload required)", name),
    }))
}

/// POST /api/fiber-types
pub async fn create_fiber_type(
    State(state): State<AppState>,
    Json(req): Json<CreateFiberTypeRequest>,
) -> Result<Json<CreateFiberTypeResponse>, ApiError> {
    // 1. Parse and validate the incoming fiber type YAML
    let new_fiber_type: FiberTypeConfig = serde_yaml::from_str(&req.yaml_content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid YAML: {}", e)))?;

    // 2. Check if fiber type already exists
    let config = state.config.read().await;
    if config.fiber_types.contains_key(&req.name) {
        return Err(ApiError::Conflict(format!(
            "Fiber type '{}' already exists",
            req.name
        )));
    }
    drop(config);

    // 3. Get the current config YAML string
    let mut config_yaml_guard = state.config_yaml.write().await;
    let current_yaml = config_yaml_guard.clone();

    // 4. Build full YAML with name line included (indent content by 2 spaces)
    let indented_content = req.yaml_content
        .lines()
        .map(|line| if line.is_empty() { String::new() } else { format!("  {}", line) })
        .collect::<Vec<_>>()
        .join("\n");
    let full_yaml = format!("{}:\n{}", req.name, indented_content);

    // 5. Add the fiber type to the YAML string (preserves comments)
    let updated_yaml = add_fiber_type_to_yaml(&current_yaml, &req.name, &full_yaml)?;

    // 5. Parse the updated YAML to validate it
    let _updated_config: Config = serde_yaml::from_str(&updated_yaml)
        .map_err(|e| ApiError::Internal(format!("Failed to parse updated config: {}", e)))?;

    // 6. Compute new version hash
    let new_hash = compute_config_hash(&updated_yaml);

    // 7. Update in-memory state
    *config_yaml_guard = updated_yaml.clone();
    let mut config = state.config.write().await;
    config.fiber_types.insert(req.name.clone(), new_fiber_type);

    // 8. Write new config version to database (DB-only, no file write)
    // Check if this version already exists (same YAML hash)
    if state.storage.get_config_version(&new_hash).await?.is_some() {
        // Version already exists - just update its timestamp
        state.storage.touch_config_version(&new_hash).await?;
    } else {
        // New version - insert it
        let parent_hash = state
            .storage
            .get_active_config_version()
            .await?
            .map(|v| v.version_hash);

        let config_version = ConfigVersion {
            version_hash: new_hash.clone(),
            parent_hash,
            yaml_content: updated_yaml,
            created_at: Utc::now(),
            source: ConfigSource::UI,
            is_active: false,
        };

        state.storage.insert_config_version(&config_version).await?;
    }

    Ok(Json(CreateFiberTypeResponse {
        name: req.name,
        new_version_hash: new_hash,
    }))
}

// ============================================================================
// Reprocessing API
// ============================================================================

/// POST /api/reprocess
pub async fn start_reprocessing(
    State(state): State<AppState>,
    Json(req): Json<StartReprocessRequest>,
) -> Result<Json<StartReprocessResponse>, ApiError> {
    use crate::reprocessing::run_reprocessing;

    // 1. Check if reprocessing is already active
    let mut reprocess_state_guard = state.reprocess_state.write().await;
    if let Some(existing) = reprocess_state_guard.as_ref() {
        if matches!(existing.status, ReprocessStatus::Running) {
            return Err(ApiError::Conflict(
                "Reprocessing already in progress".to_string(),
            ));
        }
    }

    // 2. Create reprocess state
    let task_id = Uuid::new_v4();
    let mut config = state.config.read().await.clone();
    let version = *state.config_version.read().await;

    let time_range = req.time_range.map(|r| (r.start, r.end));

    if config.auto_source_fibers || config.mode == OperationMode::Parent {
        let source_ids = state.storage.get_all_source_ids().await?;
        if !source_ids.is_empty() {
            crate::config::parse::add_auto_source_fibers_from_list(&mut config, &source_ids);
        }
    }

    let reprocess_state = Arc::new(RwLock::new(ReprocessState {
        task_id,
        started_at: Utc::now(),
        status: ReprocessStatus::Running,
        config_version: version,
        time_range,
        clear_old_results: req.clear_old_results,
        progress: ReprocessProgress::default(),
    }));

    *reprocess_state_guard = Some((*reprocess_state.read().await).clone());
    drop(reprocess_state_guard); // Release lock before spawning

    // 3. Spawn background task
    let storage = Arc::clone(&state.storage);
    let processor_lock = Arc::clone(&state.fiber_processor);
    let state_lock = Arc::clone(&state.reprocess_state);

    tokio::spawn(async move {
        // Acquire write lock on processor (pauses live ingestion)
        let _processor_guard = processor_lock.write().await;

        // Run reprocessing
        let result = run_reprocessing(
            storage,
            config,
            version,
            time_range,
            req.clear_old_results,
            reprocess_state,
        )
        .await;

        // Update state with result
        let mut state_guard = state_lock.write().await;
        if let Some(s) = state_guard.as_mut() {
            match result {
                Ok(_) => s.status = ReprocessStatus::Completed,
                Err(e) => s.status = ReprocessStatus::Failed(e.to_string()),
            }
        }
    });

    Ok(Json(StartReprocessResponse { task_id }))
}

/// GET /api/reprocess/status
pub async fn get_reprocess_status(
    State(state): State<AppState>,
) -> Result<Json<ReprocessStatusResponse>, ApiError> {
    let state_guard = state.reprocess_state.read().await;

    match state_guard.as_ref() {
        Some(s) => Ok(Json(ReprocessStatusResponse {
            status: match s.status {
                ReprocessStatus::Running => "running".to_string(),
                ReprocessStatus::Completed => "completed".to_string(),
                ReprocessStatus::Failed(_) => "failed".to_string(),
                ReprocessStatus::Cancelled => "cancelled".to_string(),
            },
            progress: Some(s.progress.clone()),
            error: match &s.status {
                ReprocessStatus::Failed(e) => Some(e.clone()),
                _ => None,
            },
        })),
        None => Ok(Json(ReprocessStatusResponse {
            status: "none".to_string(),
            progress: None,
            error: None,
        })),
    }
}

/// POST /api/reprocess/cancel
pub async fn cancel_reprocessing(
    State(state): State<AppState>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let mut state_guard = state.reprocess_state.write().await;

    if let Some(s) = state_guard.as_mut() {
        if matches!(s.status, ReprocessStatus::Running) {
            s.status = ReprocessStatus::Cancelled;
            Ok(Json(SuccessResponse {
                message: "Reprocessing cancelled".to_string(),
            }))
        } else {
            Err(ApiError::BadRequest(
                "No active reprocessing to cancel".to_string(),
            ))
        }
    } else {
        Err(ApiError::NotFound("No reprocessing found".to_string()))
    }
}

// ============================================================================
// Working Set Testing API
// ============================================================================

/// POST /api/fiber-types/:name/test-working-set
pub async fn test_working_set(
    State(state): State<AppState>,
    Path(fiber_type_name): Path<String>,
    Json(request): Json<TestWorkingSetRequest>,
) -> Result<Json<TestWorkingSetResponse>, ApiError> {
    use crate::config::types::Config;
    use crate::fiber::processor::FiberProcessor;
    use crate::source::reader::LogRecord;
    use std::collections::{HashMap, HashSet};

    // 1. Validate that we have log IDs
    if request.log_ids.is_empty() {
        return Err(ApiError::BadRequest("log_ids cannot be empty".to_string()));
    }

    // 2. Query logs by IDs
    let mut expected_logs = Vec::new();
    for log_id in &request.log_ids {
        let log = state
            .storage
            .get_log(*log_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("Log not found: {}", log_id)))?;
        expected_logs.push(log);
    }

    // 3. Calculate time window: [min(timestamps) - max_gap, max(timestamps) + max_gap]
    let min_timestamp = expected_logs
        .iter()
        .map(|l| l.timestamp)
        .min()
        .ok_or_else(|| ApiError::Internal("No logs found".to_string()))?;
    let max_timestamp = expected_logs
        .iter()
        .map(|l| l.timestamp)
        .max()
        .ok_or_else(|| ApiError::Internal("No logs found".to_string()))?;

    // 4. Parse fiber type config from yaml_content
    // The YAML should be in the format "fiber_name:\n  description: ..."
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&request.yaml_content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid YAML: {}", e)))?;

    let yaml_map = yaml_value.as_mapping()
        .ok_or_else(|| ApiError::BadRequest("YAML must be a mapping with one fiber type".to_string()))?;

    if yaml_map.len() != 1 {
        return Err(ApiError::BadRequest(
            "YAML must contain exactly one fiber type definition".to_string()
        ));
    }

    let (yaml_name, fiber_config_value) = yaml_map.iter().next().unwrap();
    let yaml_fiber_name = yaml_name.as_str()
        .ok_or_else(|| ApiError::BadRequest("Fiber type name must be a string".to_string()))?;

    // Validate that the fiber type name in YAML matches the path parameter
    if yaml_fiber_name != fiber_type_name {
        return Err(ApiError::BadRequest(format!(
            "Fiber type name in YAML ('{}') does not match path parameter ('{}')",
            yaml_fiber_name, fiber_type_name
        )));
    }

    let fiber_type_config: crate::config::types::FiberTypeConfig = serde_yaml::from_value(fiber_config_value.clone())
        .map_err(|e| ApiError::BadRequest(format!("Invalid fiber type config: {}", e)))?;

    // Determine the margin (max_gap) for the time window
    let margin = if request.include_margin.unwrap_or(true) {
        fiber_type_config.temporal.max_gap
            .map(|d| chrono::Duration::from_std(d).unwrap_or_else(|_| chrono::Duration::seconds(60)))
            .unwrap_or_else(|| chrono::Duration::hours(1)) // Default 1 hour for infinite
    } else {
        chrono::Duration::seconds(0)
    };

    let time_window_start = min_timestamp - margin;
    let time_window_end = max_timestamp + margin;

    // 5. Create temporary Config with ONLY this fiber type
    let mut temp_fiber_types = HashMap::new();
    temp_fiber_types.insert(fiber_type_name.clone(), fiber_type_config);

    let temp_config = Config {
        mode: state.config.read().await.mode,
        collector: None,
        parent: None,
        sources: HashMap::new(), // Not needed for test processing
        fiber_types: temp_fiber_types,
        auto_source_fibers: false,
        pipeline: state.config.read().await.pipeline.clone(),
        sequencer: state.config.read().await.sequencer.clone(),
        storage: state.config.read().await.storage.clone(),
        web: state.config.read().await.web.clone(),
    };

    // 6. Create temporary FiberProcessor
    let temp_version = 999999u64; // Temporary version number
    let mut temp_processor = FiberProcessor::from_config(&temp_config, temp_version)
        .map_err(|e| ApiError::Internal(format!("Failed to create temporary processor: {}", e)))?;

    // 7. Query all logs in time window (limit to reasonable amount, e.g., 10,000)
    let window_logs = state
        .storage
        .query_logs_by_time(time_window_start, time_window_end, 10000, 0)
        .await?;

    // 8. Convert StoredLog to LogRecord and process through temporary processor
    let mut fiber_memberships: HashMap<Uuid, Vec<Uuid>> = HashMap::new(); // fiber_id -> [log_ids]

    for stored_log in &window_logs {
        let log_record = LogRecord {
            id: stored_log.log_id,
            timestamp: stored_log.timestamp,
            source_id: stored_log.source_id.clone(),
            raw_text: stored_log.raw_text.clone(),
            file_offset: 0, // Not needed for testing
        };

        let results = temp_processor.process_log(&log_record);

        // Collect memberships
        for result in results {
            for membership in result.memberships {
                fiber_memberships
                    .entry(membership.fiber_id)
                    .or_insert_with(Vec::new)
                    .push(membership.log_id);
            }
        }
    }

    // 9. Compute IoU for each generated fiber against expected log_ids
    let expected_log_set: HashSet<Uuid> = request.log_ids.iter().copied().collect();

    let mut fiber_results = Vec::new();

    for (fiber_id, fiber_log_ids) in fiber_memberships {
        let fiber_log_set: HashSet<Uuid> = fiber_log_ids.iter().copied().collect();

        let iou = calculate_iou(&expected_log_set, &fiber_log_set);

        let matching_logs: Vec<Uuid> = expected_log_set.intersection(&fiber_log_set).copied().collect();
        let missing_logs: Vec<Uuid> = expected_log_set.difference(&fiber_log_set).copied().collect();
        let extra_log_ids: Vec<Uuid> = fiber_log_set.difference(&expected_log_set).copied().collect();

        // Fetch full log objects for this fiber
        let mut fiber_logs = Vec::new();
        for log_id in &fiber_log_ids {
            if let Some(stored_log) = window_logs.iter().find(|l| l.log_id == *log_id) {
                fiber_logs.push(LogDto::from(stored_log.clone()));
            }
        }

        fiber_results.push(FiberMatchResult {
            fiber_id,
            iou,
            matching_logs,
            missing_logs,
            extra_log_ids,
            logs: fiber_logs,
        });
    }

    // 10. Sort results by IoU descending
    fiber_results.sort_by(|a, b| b.iou.partial_cmp(&a.iou).unwrap_or(std::cmp::Ordering::Equal));

    // Find best match (highest IoU) after sorting
    let best_match_index = if fiber_results.is_empty() {
        None
    } else {
        Some(0) // First element after sorting has highest IoU
    };

    // Convert expected_logs to DTOs
    let expected_logs_dto = expected_logs
        .into_iter()
        .map(LogDto::from)
        .collect();

    Ok(Json(TestWorkingSetResponse {
        expected_logs: expected_logs_dto,
        time_window: TimeWindowDto {
            start: time_window_start,
            end: time_window_end,
        },
        fibers_generated: fiber_results,
        best_match_index,
    }))
}

/// Calculate Intersection over Union (IoU) for two sets
fn calculate_iou(expected: &HashSet<Uuid>, actual: &HashSet<Uuid>) -> f64 {
    let intersection = expected.intersection(actual).count();
    let union = expected.union(actual).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Update a fiber type in the config YAML string while preserving comments
/// The new_fiber_yaml should include the fiber type name line (e.g., "request_trace:\n  description: ...")
fn update_fiber_type_in_yaml(yaml: &str, name: &str, new_fiber_yaml: &str) -> Result<String, ApiError> {
    // Find the fiber_types section and the specific fiber type
    let lines: Vec<&str> = yaml.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut in_fiber_types = false;
    let mut in_target_fiber = false;
    let mut found_target = false;
    let mut fiber_indent;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Check if we're entering fiber_types section
        if trimmed.starts_with("fiber_types:") {
            in_fiber_types = true;
            tracing::debug!(line_num = i, "Found fiber_types section");
            result.push(line.to_string());
            i += 1;
            continue;
        }

        // If we're in fiber_types, look for our target fiber
        if in_fiber_types && !in_target_fiber {
            // Check if this is a top-level key in fiber_types (2-space indent typically)
            let current_indent = line.len() - trimmed.len();

            // Log lines we're checking within fiber_types
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                tracing::trace!(
                    line_num = i,
                    line = %trimmed,
                    indent = current_indent,
                    looking_for = %name,
                    "Checking line in fiber_types section"
                );
            }

            // Check if this line is the start of our target fiber type (check BEFORE exit condition)
            if trimmed.starts_with(&format!("{}:", name)) && !trimmed.starts_with(&format!("{}::", name)) {
                tracing::info!(
                    line_num = i,
                    fiber_name = %name,
                    current_indent,
                    "Found target fiber type"
                );
                in_target_fiber = true;
                found_target = true;
                fiber_indent = current_indent;

                // Insert the new fiber type YAML (already includes name line)
                for new_line in new_fiber_yaml.lines() {
                    result.push(format!("{}{}", " ".repeat(fiber_indent), new_line));
                }

                // Skip lines until we hit the next fiber type or end of fiber_types
                i += 1;
                while i < lines.len() {
                    let next_line = lines[i];
                    let next_trimmed = next_line.trim_start();
                    let next_indent = next_line.len() - next_trimmed.len();

                    // Stop if we hit another top-level key or same-level fiber type
                    if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') {
                        if next_indent <= fiber_indent {
                            in_target_fiber = false;
                            break;
                        }
                    }
                    i += 1;
                }
                continue;
            }

            // If we hit another top-level section, we're done with fiber_types
            if !trimmed.is_empty() && !trimmed.starts_with('#') && current_indent == 0 {
                tracing::debug!(line_num = i, line = %trimmed, "Exiting fiber_types section");
                in_fiber_types = false;
            }
        }

        result.push(line.to_string());
        i += 1;
    }

    if !found_target {
        // Collect fiber types found in the fiber_types section only
        let fiber_types_in_section: Vec<String> = {
            let lines: Vec<&str> = yaml.lines().collect();
            let mut found_types = Vec::new();
            let mut in_ft_section = false;

            for line in lines {
                let trimmed = line.trim_start();
                if trimmed.starts_with("fiber_types:") {
                    in_ft_section = true;
                    continue;
                }

                if in_ft_section {
                    let indent = line.len() - trimmed.len();

                    // Exit fiber_types section when we hit a top-level key
                    if !trimmed.is_empty() && !trimmed.starts_with('#') && indent == 0 {
                        break;
                    }

                    // Collect fiber type names (2-space indent, ends with colon)
                    if indent == 2 && trimmed.ends_with(':') && !trimmed.starts_with('#') {
                        found_types.push(trimmed.trim_end_matches(':').to_string());
                    }
                }
            }
            found_types
        };

        tracing::error!(
            fiber_name = %name,
            yaml_length = yaml.len(),
            in_fiber_types = in_fiber_types,
            fiber_types_found = ?fiber_types_in_section,
            "Failed to find fiber type '{}' in YAML. Fiber types in fiber_types section: {:?}",
            name,
            fiber_types_in_section
        );
        return Err(ApiError::NotFound(format!(
            "Fiber type '{}' not found in config YAML. Found fiber types: {:?}. Check that the fiber type exists and is properly formatted.",
            name,
            fiber_types_in_section
        )));
    }

    Ok(result.join("\n"))
}

/// Add a new fiber type to the config YAML string while preserving comments
/// The fiber_yaml should include the fiber type name line (e.g., "request_trace:\n  description: ...")
fn add_fiber_type_to_yaml(yaml: &str, _name: &str, fiber_yaml: &str) -> Result<String, ApiError> {
    // Find the fiber_types section and add the new fiber type at the end
    let lines: Vec<&str> = yaml.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut found_fiber_types = false;
    let mut fiber_types_indent;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Check if we're at fiber_types section
        if trimmed.starts_with("fiber_types:") {
            found_fiber_types = true;
            result.push(line.to_string());
            i += 1;

            // Find the indent level of fiber type entries
            if i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim_start();
                if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') {
                    fiber_types_indent = next_line.len() - next_trimmed.len();
                } else {
                    fiber_types_indent = 2; // Default 2-space indent
                }
            } else {
                fiber_types_indent = 2;
            }

            // Keep adding lines until we hit the next top-level section
            let mut inserted = false;
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim_start();
                let next_indent = next_line.len() - next_trimmed.len();

                // If we hit a top-level section (0 indent), insert the new fiber type here
                if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') && next_indent == 0 {
                    // Add the new fiber type (already includes name line)
                    for new_line in fiber_yaml.lines() {
                        result.push(format!("{}{}", " ".repeat(fiber_types_indent), new_line));
                    }
                    result.push(String::new()); // Add blank line
                    inserted = true;
                    break;
                }

                result.push(next_line.to_string());
                i += 1;
            }

            // If we reached the end of the file without inserting, add at the end
            if !inserted {
                result.push(String::new()); // Add blank line before new fiber type
                for new_line in fiber_yaml.lines() {
                    result.push(format!("{}{}", " ".repeat(fiber_types_indent), new_line));
                }
            }
            continue;
        }

        result.push(line.to_string());
        i += 1;
    }

    if !found_fiber_types {
        return Err(ApiError::Internal("fiber_types section not found in config".to_string()));
    }

    Ok(result.join("\n"))
}

/// Extract a fiber type's YAML content from the config while preserving formatting
/// Returns the content WITH the fiber type name line included (e.g., "request_trace:\n  description: ...")
fn extract_fiber_type_yaml_with_name(yaml: &str, name: &str) -> Result<String, ApiError> {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut in_fiber_types = false;
    let mut found_target = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Check if we're entering fiber_types section
        if trimmed.starts_with("fiber_types:") {
            in_fiber_types = true;
            i += 1;
            continue;
        }

        // If we're in fiber_types, look for our target fiber
        if in_fiber_types {
            let current_indent = line.len() - trimmed.len();

            // If we hit another top-level section, we're done
            if !trimmed.is_empty() && !trimmed.starts_with('#') && current_indent == 0 {
                break;
            }

            // Check if this is our target fiber type
            if trimmed.starts_with(&format!("{}:", name)) && !trimmed.starts_with(&format!("{}::", name)) {
                found_target = true;
                let fiber_indent = current_indent;

                // Add the fiber type name line (without base indentation)
                result.push(trimmed.to_string());

                // Collect all child lines (lines with greater indentation)
                i += 1;
                while i < lines.len() {
                    let next_line = lines[i];
                    let next_trimmed = next_line.trim_start();
                    let next_indent = next_line.len() - next_trimmed.len();

                    // If we hit a blank line, check what comes after
                    if next_trimmed.is_empty() {
                        // Look ahead to find the next non-blank line
                        let mut peek_idx = i + 1;
                        while peek_idx < lines.len() && lines[peek_idx].trim_start().is_empty() {
                            peek_idx += 1;
                        }

                        if peek_idx < lines.len() {
                            let peek_line = lines[peek_idx];
                            let peek_trimmed = peek_line.trim_start();
                            let peek_indent = peek_line.len() - peek_trimmed.len();

                            // If the next non-blank line is at same or lower indent, stop
                            if peek_indent <= fiber_indent {
                                break;
                            }
                        }

                        // Otherwise, include this blank line
                        result.push(String::new());
                        i += 1;
                        continue;
                    }

                    // Stop if we hit another same-level or higher-level key
                    if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') && next_indent <= fiber_indent {
                        break;
                    }

                    // Add the line, preserving relative indentation
                    // Content should be indented 2 spaces relative to the fiber type name
                    if next_indent > fiber_indent {
                        // Keep 2-space indent under fiber type name, plus any additional indentation
                        let relative_indent = next_indent - fiber_indent;
                        result.push(format!("{}{}", " ".repeat(relative_indent), next_trimmed));
                    } else {
                        // Comments at the same level
                        result.push(next_trimmed.to_string());
                    }

                    i += 1;
                }
                break;
            }
        }

        i += 1;
    }

    if !found_target {
        return Err(ApiError::NotFound(format!("Fiber type '{}' not found in config", name)));
    }

    Ok(result.join("\n"))
}

/// Remove a fiber type from the config YAML string while preserving comments
fn delete_fiber_type_from_yaml(yaml: &str, name: &str) -> Result<String, ApiError> {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut in_fiber_types = false;
    let mut found_target = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Check if we're entering fiber_types section
        if trimmed.starts_with("fiber_types:") {
            in_fiber_types = true;
            result.push(line.to_string());
            i += 1;
            continue;
        }

        // If we're in fiber_types, look for our target fiber
        if in_fiber_types {
            let current_indent = line.len() - trimmed.len();

            // If we hit another top-level section, we're done
            if !trimmed.is_empty() && !trimmed.starts_with('#') && current_indent == 0 {
                in_fiber_types = false;
            }

            // Check if this is our target fiber type
            if trimmed.starts_with(&format!("{}:", name)) && !trimmed.starts_with(&format!("{}::", name)) {
                found_target = true;
                let fiber_indent = current_indent;

                // Skip this fiber type and all its children
                i += 1;
                while i < lines.len() {
                    let next_line = lines[i];
                    let next_trimmed = next_line.trim_start();
                    let next_indent = next_line.len() - next_trimmed.len();

                    // Stop if we hit another same-level or higher-level key
                    if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') && next_indent <= fiber_indent {
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }

        result.push(line.to_string());
        i += 1;
    }

    if !found_target {
        return Err(ApiError::NotFound(format!("Fiber type '{}' not found in config", name)));
    }

    Ok(result.join("\n"))
}
