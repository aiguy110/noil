# 06: Storage Layer

Implement the storage trait and DuckDB backend.

## Location

`src/storage/traits.rs`, `src/storage/duckdb.rs`

## Storage Trait

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    async fn init_schema(&self) -> Result<(), StorageError>;

    // Raw logs
    async fn write_logs(&self, logs: &[LogRecord]) -> Result<(), StorageError>;
    async fn get_log(&self, log_id: Uuid) -> Result<Option<StoredLog>, StorageError>;
    async fn query_logs_by_time(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError>;

    // Fibers
    async fn write_fiber(&self, fiber: &FiberRecord) -> Result<(), StorageError>;
    async fn update_fiber(&self, fiber: &FiberRecord) -> Result<(), StorageError>;
    async fn get_fiber(&self, fiber_id: Uuid) -> Result<Option<FiberRecord>, StorageError>;
    async fn query_fibers_by_type(
        &self,
        fiber_type: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<FiberRecord>, StorageError>;

    // Memberships
    async fn write_memberships(&self, memberships: &[FiberMembership]) -> Result<(), StorageError>;
    async fn get_log_fibers(&self, log_id: Uuid) -> Result<Vec<Uuid>, StorageError>;
    async fn get_fiber_logs(
        &self,
        fiber_id: Uuid,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError>;
}
```

## Data Types

```rust
pub struct StoredLog {
    pub log_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
    pub raw_text: String,
    pub ingestion_time: DateTime<Utc>,
    pub config_version: u64,
}

pub struct FiberRecord {
    pub fiber_id: Uuid,
    pub fiber_type: String,
    pub config_version: u64,
    pub attributes: serde_json::Value,
    pub first_activity: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub closed: bool,
}

pub struct FiberMembership {
    pub log_id: Uuid,
    pub fiber_id: Uuid,
    pub config_version: u64,
}
```

## DuckDB Implementation

```rust
pub struct DuckDbStorage {
    conn: Connection,  // duckdb::Connection
    config_version: u64,
}
```

### Schema

Execute on `init_schema()`:

```sql
CREATE TABLE IF NOT EXISTS raw_logs (
    log_id UUID PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    source_id VARCHAR NOT NULL,
    raw_text VARCHAR NOT NULL,
    ingestion_time TIMESTAMPTZ DEFAULT now(),
    config_version UBIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_raw_logs_timestamp ON raw_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_raw_logs_source ON raw_logs(source_id);

CREATE TABLE IF NOT EXISTS fibers (
    fiber_id UUID PRIMARY KEY,
    fiber_type VARCHAR NOT NULL,
    config_version UBIGINT NOT NULL,
    attributes JSON,
    first_activity TIMESTAMPTZ NOT NULL,
    last_activity TIMESTAMPTZ NOT NULL,
    closed BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS idx_fibers_type ON fibers(fiber_type);

CREATE TABLE IF NOT EXISTS fiber_memberships (
    log_id UUID NOT NULL,
    fiber_id UUID NOT NULL,
    config_version UBIGINT NOT NULL,
    PRIMARY KEY (log_id, fiber_id)
);
CREATE INDEX IF NOT EXISTS idx_memberships_fiber ON fiber_memberships(fiber_id);
```

### Threading

DuckDB connections are not thread-safe. Options:

1. Use `Mutex<Connection>` (simple, may bottleneck)
2. Use connection pool (overkill for MVP)
3. Use DuckDB's thread-safe appender for bulk writes

For MVP, use `Mutex<Connection>` with `spawn_blocking` for queries.

### Batching

The storage writer (see pipeline ticket) handles batching. The trait methods accept slices for bulk operations.

## Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] duckdb::Error),

    #[error("record not found: {0}")]
    NotFound(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

## Config Version

All writes include `config_version` from the current config. This enables:
- Identifying which config version processed each record
- Selective reprocessing by config version

For MVP, compute config version as hash of config file contents.

## Acceptance Criteria

- Schema creates tables and indexes
- CRUD operations work for logs, fibers, memberships
- Queries return correct results with pagination
- JSON attributes serialize/deserialize correctly
- Concurrent access doesn't corrupt data
