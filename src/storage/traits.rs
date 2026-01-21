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

    /// Close orphaned fibers - fibers that are open in storage but not in the checkpoint.
    /// This prevents duplicate fibers after a crash where fibers were written to storage
    /// but didn't make it into the checkpoint before the crash.
    ///
    /// Takes a set of fiber IDs that should remain open (from checkpoint).
    /// All other fibers with closed=false will be marked as closed.
    async fn close_orphaned_fibers(&self, checkpointed_fiber_ids: &std::collections::HashSet<Uuid>) -> Result<usize, StorageError>;
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

    #[error("DuckDB error: {0}")]
    DuckDb(#[from] duckdb::Error),

    #[error("checkpoint error: {0}")]
    Checkpoint(String),
}
