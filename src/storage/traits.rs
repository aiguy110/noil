use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::checkpoint::Checkpoint;

/// Stored log record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredLog {
    pub log_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
    pub raw_text: String,
    pub ingestion_time: DateTime<Utc>,
    pub config_version: u64,
}

/// Fiber record for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiberRecord {
    pub fiber_id: Uuid,
    pub fiber_type: String,
    pub config_version: u64,
    pub attributes: serde_json::Value,
    pub first_activity: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub closed: bool,
}

/// Many-to-many relationship between logs and fibers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiberMembership {
    pub log_id: Uuid,
    pub fiber_id: Uuid,
    pub config_version: u64,
}

/// Config version record for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigVersion {
    pub version_hash: String,
    pub parent_hash: Option<String>,
    pub yaml_content: String,
    pub created_at: DateTime<Utc>,
    pub source: ConfigSource,
    pub is_active: bool,
}

/// Source of config changes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    File,
    UI,
    Merge,
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigSource::File => write!(f, "file"),
            ConfigSource::UI => write!(f, "ui"),
            ConfigSource::Merge => write!(f, "merge"),
        }
    }
}

/// Config state tracking conflicts and hashes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigState {
    pub has_conflict: bool,
    pub conflict_file_path: Option<String>,
    pub file_version_hash: Option<String>,
    pub db_version_hash: Option<String>,
}

/// Storage trait for persisting logs, fibers, and memberships
#[async_trait]
pub trait Storage: Send + Sync {
    /// Initialize database schema (tables, indexes)
    async fn init_schema(&self) -> Result<(), StorageError>;

    // Raw logs
    /// Write multiple log records in bulk
    async fn write_logs(&self, logs: &[StoredLog]) -> Result<(), StorageError>;

    /// Get a single log by ID
    async fn get_log(&self, log_id: Uuid) -> Result<Option<StoredLog>, StorageError>;

    /// Query logs within a time range with pagination
    async fn query_logs_by_time(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError>;

    // Fibers
    /// Write a new fiber record
    async fn write_fiber(&self, fiber: &FiberRecord) -> Result<(), StorageError>;

    /// Update an existing fiber record
    async fn update_fiber(&self, fiber: &FiberRecord) -> Result<(), StorageError>;

    /// Get a single fiber by ID
    async fn get_fiber(&self, fiber_id: Uuid) -> Result<Option<FiberRecord>, StorageError>;

    /// Query fibers by type with pagination
    async fn query_fibers_by_type(
        &self,
        fiber_type: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<FiberRecord>, StorageError>;

    // Memberships
    /// Write multiple fiber memberships in bulk
    async fn write_memberships(&self, memberships: &[FiberMembership]) -> Result<(), StorageError>;

    /// Get all fiber IDs that a log belongs to
    async fn get_log_fibers(&self, log_id: Uuid) -> Result<Vec<Uuid>, StorageError>;

    /// Get all logs that belong to a fiber with pagination
    async fn get_fiber_logs(
        &self,
        fiber_id: Uuid,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError>;

    /// Get all unique fiber types
    async fn get_all_fiber_types(&self) -> Result<Vec<String>, StorageError>;

    /// Get all unique source IDs
    async fn get_all_source_ids(&self) -> Result<Vec<String>, StorageError>;

    // Checkpoints
    /// Load the latest checkpoint from storage
    async fn load_checkpoint(&self) -> Result<Option<Checkpoint>, StorageError>;

    /// Save a checkpoint to storage
    async fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<(), StorageError>;

    /// Load the latest collector checkpoint from storage (for collector mode)
    async fn load_collector_checkpoint(&self) -> Result<Option<String>, StorageError>;

    /// Save a collector checkpoint to storage (for collector mode)
    async fn save_collector_checkpoint(&self, json: &str) -> Result<(), StorageError>;

    /// Load the latest parent checkpoint from storage (for parent mode)
    async fn load_parent_checkpoint(&self) -> Result<Option<String>, StorageError>;

    /// Save a parent checkpoint to storage (for parent mode)
    async fn save_parent_checkpoint(&self, json: &str) -> Result<(), StorageError>;

    /// Close orphaned fibers - fibers that are open in storage but not in the checkpoint.
    /// This prevents duplicate fibers after a crash where fibers were written to storage
    /// but didn't make it into the checkpoint before the crash.
    ///
    /// Takes a set of fiber IDs that should remain open (from checkpoint).
    /// All other fibers with closed=false will be marked as closed.
    async fn close_orphaned_fibers(&self, checkpointed_fiber_ids: &std::collections::HashSet<Uuid>) -> Result<usize, StorageError>;

    // Config versioning
    /// Get the currently active config version
    async fn get_active_config_version(&self) -> Result<Option<ConfigVersion>, StorageError>;

    /// Insert a new config version and mark it as active (deactivates all others)
    async fn insert_config_version(&self, version: &ConfigVersion) -> Result<(), StorageError>;

    /// Get a specific config version by hash
    async fn get_config_version(&self, hash: &str) -> Result<Option<ConfigVersion>, StorageError>;

    /// List config versions with pagination (most recent first)
    async fn list_config_versions(&self, limit: usize, offset: usize) -> Result<Vec<ConfigVersion>, StorageError>;

    /// Count total number of config versions
    async fn count_config_versions(&self) -> Result<u64, StorageError>;

    /// Check if ancestor_hash is an ancestor of descendant_hash
    async fn is_ancestor(&self, ancestor_hash: &str, descendant_hash: &str) -> Result<bool, StorageError>;

    /// Get config state
    async fn get_config_state(&self) -> Result<Option<ConfigState>, StorageError>;

    /// Update config state
    async fn update_config_state(&self, state: &ConfigState) -> Result<(), StorageError>;

    // Reprocessing support
    /// Query logs for reprocessing with optional time range filter
    async fn query_logs_for_reprocessing(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        batch_size: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError>;

    /// Delete fiber memberships for a config version with optional time range
    async fn delete_fiber_memberships(
        &self,
        config_version: u64,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<u64, StorageError>;

    /// Delete fibers for a config version
    async fn delete_fibers(
        &self,
        config_version: u64,
    ) -> Result<u64, StorageError>;

    /// Mark a config version as active (deactivates all others)
    async fn mark_config_active(&self, version_hash: &str) -> Result<(), StorageError>;

    /// Update the created_at timestamp of an existing config version (used when re-saving the same config)
    async fn touch_config_version(&self, version_hash: &str) -> Result<(), StorageError>;
}

/// Storage errors
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(String),

    #[error("record not found: {0}")]
    NotFound(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error("DuckDB error: {0}")]
    DuckDb(#[from] duckdb::Error),

    #[error("checkpoint error: {0}")]
    Checkpoint(String),
}
