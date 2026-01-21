use super::checkpoint::Checkpoint;
use super::traits::{FiberMembership, FiberRecord, Storage, StorageError, StoredLog};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use duckdb::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Check if a process with the given PID is still running
fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("ps")
            .arg("-p")
            .arg(pid.to_string())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // On non-Unix systems, assume process is running to be safe
        true
    }
}

/// Extract PID from DuckDB lock error message
fn extract_pid_from_lock_error(error_msg: &str) -> Option<u32> {
    // Error format: "... (PID 12345) ..."
    if let Some(start) = error_msg.find("(PID ") {
        let start = start + 5; // Length of "(PID "
        if let Some(end) = error_msg[start..].find(')') {
            let pid_str = &error_msg[start..start + end];
            return pid_str.parse().ok();
        }
    }
    None
}

/// Remove DuckDB lock files (WAL and lock files)
fn remove_lock_files(db_path: &Path) -> std::io::Result<()> {
    let wal_path = PathBuf::from(format!("{}.wal", db_path.display()));
    let lock_path = PathBuf::from(format!("{}.lock", db_path.display()));

    // Try to remove WAL file
    if wal_path.exists() {
        std::fs::remove_file(&wal_path)?;
        tracing::info!("Removed stale WAL file: {}", wal_path.display());
    }

    // Try to remove lock file if it exists
    if lock_path.exists() {
        std::fs::remove_file(&lock_path)?;
        tracing::info!("Removed stale lock file: {}", lock_path.display());
    }

    Ok(())
}

/// DuckDB implementation of the Storage trait
pub struct DuckDbStorage {
    conn: Arc<Mutex<Connection>>,
}

impl DuckDbStorage {
    /// Create a new DuckDB storage instance
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let path = path.as_ref();

        // First attempt to open the connection
        match Connection::open(path) {
            Ok(conn) => {
                return Ok(Self {
                    conn: Arc::new(Mutex::new(conn)),
                });
            }
            Err(e) => {
                // Check if this is a lock error
                let error_msg = e.to_string();

                if error_msg.contains("Could not set lock") {
                    tracing::warn!("Database lock detected: {}", error_msg);

                    // Try to extract the PID from the error message
                    if let Some(pid) = extract_pid_from_lock_error(&error_msg) {
                        tracing::info!("Lock is held by PID {}, checking if process is running", pid);

                        if !is_process_running(pid) {
                            tracing::warn!("Process {} is not running, removing stale lock files", pid);

                            // Remove stale lock files
                            if let Err(io_err) = remove_lock_files(path) {
                                tracing::error!("Failed to remove lock files: {}", io_err);
                                return Err(e.into());
                            }

                            // Retry opening the connection
                            tracing::info!("Retrying database connection after removing stale locks");
                            let conn = Connection::open(path)?;
                            return Ok(Self {
                                conn: Arc::new(Mutex::new(conn)),
                            });
                        } else {
                            tracing::error!("Process {} is still running, cannot acquire lock", pid);
                        }
                    }
                }

                // If we couldn't handle the error, return it
                Err(e.into())
            }
        }
    }

    /// Create an in-memory DuckDB storage instance (for testing)
    pub fn in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

#[async_trait]
impl Storage for DuckDbStorage {
    async fn init_schema(&self) -> Result<(), StorageError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Create raw_logs table
            conn.execute(
                "CREATE TABLE IF NOT EXISTS raw_logs (
                    log_id UUID PRIMARY KEY,
                    timestamp TIMESTAMPTZ NOT NULL,
                    source_id VARCHAR NOT NULL,
                    raw_text VARCHAR NOT NULL,
                    ingestion_time TIMESTAMPTZ NOT NULL,
                    config_version UBIGINT NOT NULL
                )",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_raw_logs_timestamp ON raw_logs(timestamp)",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_raw_logs_source ON raw_logs(source_id)",
                [],
            )?;

            // Create fibers table
            conn.execute(
                "CREATE TABLE IF NOT EXISTS fibers (
                    fiber_id UUID PRIMARY KEY,
                    fiber_type VARCHAR NOT NULL,
                    config_version UBIGINT NOT NULL,
                    attributes JSON,
                    first_activity TIMESTAMPTZ NOT NULL,
                    last_activity TIMESTAMPTZ NOT NULL,
                    closed BOOLEAN NOT NULL DEFAULT FALSE
                )",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_fibers_type ON fibers(fiber_type)",
                [],
            )?;

            // Create fiber_memberships table
            conn.execute(
                "CREATE TABLE IF NOT EXISTS fiber_memberships (
                    log_id UUID NOT NULL,
                    fiber_id UUID NOT NULL,
                    config_version UBIGINT NOT NULL,
                    PRIMARY KEY (log_id, fiber_id)
                )",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_memberships_fiber ON fiber_memberships(fiber_id)",
                [],
            )?;

            // Create checkpoints table
            conn.execute(
                "CREATE TABLE IF NOT EXISTS checkpoints (
                    id INTEGER PRIMARY KEY DEFAULT 1,
                    checkpoint_data TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL,
                    CHECK (id = 1)
                )",
                [],
            )?;

            Ok::<(), StorageError>(())
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn write_logs(&self, logs: &[StoredLog]) -> Result<(), StorageError> {
        if logs.is_empty() {
            return Ok(());
        }

        let conn = self.conn.clone();
        let logs = logs.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "INSERT INTO raw_logs (log_id, timestamp, source_id, raw_text, ingestion_time, config_version)
                 VALUES (?, to_timestamp(? / 1000000.0), ?, ?, to_timestamp(? / 1000000.0), ?)",
            )?;

            for log in logs {
                stmt.execute(duckdb::params![
                    log.log_id.to_string(),
                    log.timestamp.timestamp_micros(),
                    log.source_id,
                    log.raw_text,
                    log.ingestion_time.timestamp_micros(),
                    log.config_version,
                ])?;
            }

            Ok::<(), StorageError>(())
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn get_log(&self, log_id: Uuid) -> Result<Option<StoredLog>, StorageError> {
        let conn = self.conn.clone();
        let log_id_str = log_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT log_id, epoch_us(timestamp), source_id, raw_text, epoch_us(ingestion_time), config_version
                 FROM raw_logs WHERE log_id = ?",
            )?;

            let mut rows = stmt.query(duckdb::params![log_id_str])?;

            if let Some(row) = rows.next()? {
                let log = StoredLog {
                    log_id: Uuid::parse_str(&row.get::<_, String>(0)?)
                        .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                            0,
                            duckdb::types::Type::Text,
                            Box::new(e),
                        ))?,
                    timestamp: DateTime::from_timestamp_micros(row.get::<_, i64>(1)?)
                        .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                            1,
                            duckdb::types::Type::BigInt,
                            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                        ))?,
                    source_id: row.get(2)?,
                    raw_text: row.get(3)?,
                    ingestion_time: DateTime::from_timestamp_micros(row.get::<_, i64>(4)?)
                        .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                            4,
                            duckdb::types::Type::BigInt,
                            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                        ))?,
                    config_version: row.get(5)?,
                };
                Ok(Some(log))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn query_logs_by_time(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError> {
        let conn = self.conn.clone();
        let start_micros = start.timestamp_micros();
        let end_micros = end.timestamp_micros();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT log_id, epoch_us(timestamp), source_id, raw_text, epoch_us(ingestion_time), config_version
                 FROM raw_logs
                 WHERE timestamp >= to_timestamp(? / 1000000.0) AND timestamp <= to_timestamp(? / 1000000.0)
                 ORDER BY timestamp
                 LIMIT ? OFFSET ?",
            )?;

            let rows = stmt.query_map(
                duckdb::params![start_micros, end_micros, limit as i64, offset as i64],
                |row| {
                    Ok(StoredLog {
                        log_id: Uuid::parse_str(&row.get::<_, String>(0)?)
                            .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                                0,
                                duckdb::types::Type::Text,
                                Box::new(e),
                            ))?,
                        timestamp: DateTime::from_timestamp_micros(row.get::<_, i64>(1)?)
                            .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                                1,
                                duckdb::types::Type::BigInt,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                            ))?,
                        source_id: row.get(2)?,
                        raw_text: row.get(3)?,
                        ingestion_time: DateTime::from_timestamp_micros(row.get::<_, i64>(4)?)
                            .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                                4,
                                duckdb::types::Type::BigInt,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                            ))?,
                        config_version: row.get(5)?,
                    })
                },
            )?;

            let mut logs = Vec::new();
            for row in rows {
                logs.push(row?);
            }
            Ok(logs)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn write_fiber(&self, fiber: &FiberRecord) -> Result<(), StorageError> {
        let conn = self.conn.clone();
        let fiber = fiber.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let attributes_json = serde_json::to_string(&fiber.attributes)?;

            conn.execute(
                "INSERT INTO fibers (fiber_id, fiber_type, config_version, attributes, first_activity, last_activity, closed)
                 VALUES (?, ?, ?, ?, to_timestamp(? / 1000000.0), to_timestamp(? / 1000000.0), ?)",
                duckdb::params![
                    fiber.fiber_id.to_string(),
                    fiber.fiber_type,
                    fiber.config_version,
                    attributes_json,
                    fiber.first_activity.timestamp_micros(),
                    fiber.last_activity.timestamp_micros(),
                    fiber.closed,
                ],
            )?;

            Ok::<(), StorageError>(())
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn update_fiber(&self, fiber: &FiberRecord) -> Result<(), StorageError> {
        let conn = self.conn.clone();
        let fiber = fiber.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let attributes_json = serde_json::to_string(&fiber.attributes)?;

            conn.execute(
                "UPDATE fibers
                 SET fiber_type = ?, config_version = ?, attributes = ?, first_activity = to_timestamp(? / 1000000.0), last_activity = to_timestamp(? / 1000000.0), closed = ?
                 WHERE fiber_id = ?",
                duckdb::params![
                    fiber.fiber_type,
                    fiber.config_version,
                    attributes_json,
                    fiber.first_activity.timestamp_micros(),
                    fiber.last_activity.timestamp_micros(),
                    fiber.closed,
                    fiber.fiber_id.to_string(),
                ],
            )?;

            Ok::<(), StorageError>(())
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn get_fiber(&self, fiber_id: Uuid) -> Result<Option<FiberRecord>, StorageError> {
        let conn = self.conn.clone();
        let fiber_id_str = fiber_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fiber_id, fiber_type, config_version, attributes, epoch_us(first_activity), epoch_us(last_activity), closed
                 FROM fibers WHERE fiber_id = ?",
            )?;

            let mut rows = stmt.query(duckdb::params![fiber_id_str])?;

            if let Some(row) = rows.next()? {
                let fiber = FiberRecord {
                    fiber_id: Uuid::parse_str(&row.get::<_, String>(0)?)
                        .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                            0,
                            duckdb::types::Type::Text,
                            Box::new(e),
                        ))?,
                    fiber_type: row.get(1)?,
                    config_version: row.get(2)?,
                    attributes: serde_json::from_str(&row.get::<_, String>(3)?)?,
                    first_activity: DateTime::from_timestamp_micros(row.get::<_, i64>(4)?)
                        .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                            4,
                            duckdb::types::Type::BigInt,
                            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                        ))?,
                    last_activity: DateTime::from_timestamp_micros(row.get::<_, i64>(5)?)
                        .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                            5,
                            duckdb::types::Type::BigInt,
                            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                        ))?,
                    closed: row.get(6)?,
                };
                Ok(Some(fiber))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn query_fibers_by_type(
        &self,
        fiber_type: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<FiberRecord>, StorageError> {
        let conn = self.conn.clone();
        let fiber_type = fiber_type.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fiber_id, fiber_type, config_version, attributes, epoch_us(first_activity), epoch_us(last_activity), closed
                 FROM fibers
                 WHERE fiber_type = ?
                 ORDER BY first_activity
                 LIMIT ? OFFSET ?",
            )?;

            let rows = stmt.query_map(
                duckdb::params![fiber_type, limit as i64, offset as i64],
                |row| {
                    Ok(FiberRecord {
                        fiber_id: Uuid::parse_str(&row.get::<_, String>(0)?)
                            .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                                0,
                                duckdb::types::Type::Text,
                                Box::new(e),
                            ))?,
                        fiber_type: row.get(1)?,
                        config_version: row.get(2)?,
                        attributes: serde_json::from_str(&row.get::<_, String>(3)?)
                            .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                                3,
                                duckdb::types::Type::Text,
                                Box::new(e),
                            ))?,
                        first_activity: DateTime::from_timestamp_micros(row.get::<_, i64>(4)?)
                            .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                                4,
                                duckdb::types::Type::BigInt,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                            ))?,
                        last_activity: DateTime::from_timestamp_micros(row.get::<_, i64>(5)?)
                            .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                                5,
                                duckdb::types::Type::BigInt,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                            ))?,
                        closed: row.get(6)?,
                    })
                },
            )?;

            let mut fibers = Vec::new();
            for row in rows {
                fibers.push(row?);
            }
            Ok(fibers)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn write_memberships(&self, memberships: &[FiberMembership]) -> Result<(), StorageError> {
        if memberships.is_empty() {
            return Ok(());
        }

        let conn = self.conn.clone();
        let memberships = memberships.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "INSERT INTO fiber_memberships (log_id, fiber_id, config_version)
                 VALUES (?, ?, ?)",
            )?;

            for membership in memberships {
                stmt.execute(duckdb::params![
                    membership.log_id.to_string(),
                    membership.fiber_id.to_string(),
                    membership.config_version,
                ])?;
            }

            Ok::<(), StorageError>(())
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn get_log_fibers(&self, log_id: Uuid) -> Result<Vec<Uuid>, StorageError> {
        let conn = self.conn.clone();
        let log_id_str = log_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fiber_id FROM fiber_memberships WHERE log_id = ?",
            )?;

            let rows = stmt.query_map(duckdb::params![log_id_str], |row| {
                let fiber_id_str: String = row.get(0)?;
                Uuid::parse_str(&fiber_id_str)
                    .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                        0,
                        duckdb::types::Type::Text,
                        Box::new(e),
                    ))
            })?;

            let mut fiber_ids = Vec::new();
            for row in rows {
                fiber_ids.push(row?);
            }
            Ok(fiber_ids)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn get_fiber_logs(
        &self,
        fiber_id: Uuid,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StoredLog>, StorageError> {
        let conn = self.conn.clone();
        let fiber_id_str = fiber_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT l.log_id, epoch_us(l.timestamp), l.source_id, l.raw_text, epoch_us(l.ingestion_time), l.config_version
                 FROM raw_logs l
                 INNER JOIN fiber_memberships m ON l.log_id = m.log_id
                 WHERE m.fiber_id = ?
                 ORDER BY l.timestamp
                 LIMIT ? OFFSET ?",
            )?;

            let rows = stmt.query_map(
                duckdb::params![fiber_id_str, limit as i64, offset as i64],
                |row| {
                    Ok(StoredLog {
                        log_id: Uuid::parse_str(&row.get::<_, String>(0)?)
                            .map_err(|e| duckdb::Error::FromSqlConversionFailure(
                                0,
                                duckdb::types::Type::Text,
                                Box::new(e),
                            ))?,
                        timestamp: DateTime::from_timestamp_micros(row.get::<_, i64>(1)?)
                            .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                                1,
                                duckdb::types::Type::BigInt,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                            ))?,
                        source_id: row.get(2)?,
                        raw_text: row.get(3)?,
                        ingestion_time: DateTime::from_timestamp_micros(row.get::<_, i64>(4)?)
                            .ok_or_else(|| duckdb::Error::FromSqlConversionFailure(
                                4,
                                duckdb::types::Type::BigInt,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid timestamp")),
                            ))?,
                        config_version: row.get(5)?,
                    })
                },
            )?;

            let mut logs = Vec::new();
            for row in rows {
                logs.push(row?);
            }
            Ok(logs)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn get_all_fiber_types(&self) -> Result<Vec<String>, StorageError> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT DISTINCT fiber_type FROM fibers ORDER BY fiber_type",
            )?;

            let rows = stmt.query_map([], |row| {
                row.get::<_, String>(0)
            })?;

            let mut types = Vec::new();
            for row in rows {
                types.push(row?);
            }
            Ok(types)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn get_all_source_ids(&self) -> Result<Vec<String>, StorageError> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT DISTINCT source_id FROM raw_logs ORDER BY source_id",
            )?;

            let rows = stmt.query_map([], |row| {
                row.get::<_, String>(0)
            })?;

            let mut sources = Vec::new();
            for row in rows {
                sources.push(row?);
            }
            Ok(sources)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn load_checkpoint(&self) -> Result<Option<Checkpoint>, StorageError> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT checkpoint_data FROM checkpoints WHERE id = 1",
            )?;

            let mut rows = stmt.query([])?;

            if let Some(row) = rows.next()? {
                let checkpoint_json: String = row.get(0)?;
                let checkpoint: Checkpoint = serde_json::from_str(&checkpoint_json)
                    .map_err(|e| StorageError::Checkpoint(format!("Failed to deserialize checkpoint: {}", e)))?;
                Ok(Some(checkpoint))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<(), StorageError> {
        let conn = self.conn.clone();
        let checkpoint = checkpoint.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let checkpoint_json = serde_json::to_string(&checkpoint)
                .map_err(|e| StorageError::Checkpoint(format!("Failed to serialize checkpoint: {}", e)))?;

            let now = Utc::now();

            conn.execute(
                "INSERT OR REPLACE INTO checkpoints (id, checkpoint_data, created_at)
                 VALUES (1, ?, to_timestamp(? / 1000000.0))",
                duckdb::params![checkpoint_json, now.timestamp_micros()],
            )?;

            Ok::<(), StorageError>(())
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }

    async fn close_orphaned_fibers(&self, checkpointed_fiber_ids: &std::collections::HashSet<Uuid>) -> Result<usize, StorageError> {
        let conn = self.conn.clone();
        let checkpointed_ids = checkpointed_fiber_ids.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Query all open fibers
            let mut stmt = conn.prepare(
                "SELECT fiber_id FROM fibers WHERE closed = FALSE"
            )?;

            let mut rows = stmt.query([])?;
            let mut open_fiber_ids = Vec::new();

            while let Some(row) = rows.next()? {
                let fiber_id_str: String = row.get(0)?;
                if let Ok(fiber_id) = Uuid::parse_str(&fiber_id_str) {
                    open_fiber_ids.push(fiber_id);
                }
            }

            // Drop the statement to release the borrow on conn
            drop(rows);
            drop(stmt);

            // Find orphaned fibers (open in storage but not in checkpoint)
            let mut closed_count = 0;
            for fiber_id in open_fiber_ids {
                if !checkpointed_ids.contains(&fiber_id) {
                    // This fiber is orphaned - close it
                    conn.execute(
                        "UPDATE fibers SET closed = TRUE WHERE fiber_id = ?",
                        duckdb::params![fiber_id.to_string()],
                    )?;
                    closed_count += 1;
                }
            }

            Ok::<usize, StorageError>(closed_count)
        })
        .await
        .map_err(|e| StorageError::Database(format!("Task join error: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    async fn setup_storage() -> DuckDbStorage {
        let storage = DuckDbStorage::in_memory().unwrap();
        storage.init_schema().await.unwrap();
        storage
    }

    #[tokio::test]
    async fn test_schema_initialization() {
        let storage = DuckDbStorage::in_memory().unwrap();
        assert!(storage.init_schema().await.is_ok());
    }

    #[tokio::test]
    async fn test_write_and_get_log() {
        let storage = setup_storage().await;
        let log_id = Uuid::new_v4();
        let timestamp = Utc::now();

        let log = StoredLog {
            log_id,
            timestamp,
            source_id: "test_source".to_string(),
            raw_text: "test log line".to_string(),
            ingestion_time: timestamp,
            config_version: 1,
        };

        storage.write_logs(&[log.clone()]).await.unwrap();
        let retrieved = storage.get_log(log_id).await.unwrap();

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.log_id, log_id);
        assert_eq!(retrieved.source_id, "test_source");
        assert_eq!(retrieved.raw_text, "test log line");
    }

    #[tokio::test]
    async fn test_write_multiple_logs() {
        let storage = setup_storage().await;
        let timestamp = Utc::now();

        let logs = vec![
            StoredLog {
                log_id: Uuid::new_v4(),
                timestamp,
                source_id: "source1".to_string(),
                raw_text: "log 1".to_string(),
                ingestion_time: timestamp,
                config_version: 1,
            },
            StoredLog {
                log_id: Uuid::new_v4(),
                timestamp,
                source_id: "source2".to_string(),
                raw_text: "log 2".to_string(),
                ingestion_time: timestamp,
                config_version: 1,
            },
        ];

        storage.write_logs(&logs).await.unwrap();

        for log in &logs {
            let retrieved = storage.get_log(log.log_id).await.unwrap();
            assert!(retrieved.is_some());
        }
    }

    #[tokio::test]
    async fn test_query_logs_by_time() {
        let storage = setup_storage().await;
        let now = Utc::now();
        let earlier = now - chrono::Duration::hours(1);
        let later = now + chrono::Duration::hours(1);

        let logs = vec![
            StoredLog {
                log_id: Uuid::new_v4(),
                timestamp: now,
                source_id: "test".to_string(),
                raw_text: "log 1".to_string(),
                ingestion_time: now,
                config_version: 1,
            },
            StoredLog {
                log_id: Uuid::new_v4(),
                timestamp: now + chrono::Duration::minutes(30),
                source_id: "test".to_string(),
                raw_text: "log 2".to_string(),
                ingestion_time: now,
                config_version: 1,
            },
        ];

        storage.write_logs(&logs).await.unwrap();

        let results = storage
            .query_logs_by_time(earlier, later, 10, 0)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].raw_text, "log 1");
        assert_eq!(results[1].raw_text, "log 2");
    }

    #[tokio::test]
    async fn test_write_and_get_fiber() {
        let storage = setup_storage().await;
        let fiber_id = Uuid::new_v4();
        let timestamp = Utc::now();

        let attributes = serde_json::json!({
            "key1": "value1",
            "key2": 42
        });

        let fiber = FiberRecord {
            fiber_id,
            fiber_type: "test_fiber".to_string(),
            config_version: 1,
            attributes,
            first_activity: timestamp,
            last_activity: timestamp,
            closed: false,
        };

        storage.write_fiber(&fiber).await.unwrap();
        let retrieved = storage.get_fiber(fiber_id).await.unwrap();

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.fiber_id, fiber_id);
        assert_eq!(retrieved.fiber_type, "test_fiber");
        assert_eq!(retrieved.closed, false);
        assert_eq!(retrieved.attributes["key1"], "value1");
        assert_eq!(retrieved.attributes["key2"], 42);
    }

    #[tokio::test]
    async fn test_update_fiber() {
        let storage = setup_storage().await;
        let fiber_id = Uuid::new_v4();
        let timestamp = Utc::now();

        let mut fiber = FiberRecord {
            fiber_id,
            fiber_type: "test_fiber".to_string(),
            config_version: 1,
            attributes: serde_json::json!({"key": "value"}),
            first_activity: timestamp,
            last_activity: timestamp,
            closed: false,
        };

        storage.write_fiber(&fiber).await.unwrap();

        fiber.closed = true;
        fiber.attributes = serde_json::json!({"key": "updated"});
        storage.update_fiber(&fiber).await.unwrap();

        let retrieved = storage.get_fiber(fiber_id).await.unwrap().unwrap();
        assert_eq!(retrieved.closed, true);
        assert_eq!(retrieved.attributes["key"], "updated");
    }

    #[tokio::test]
    async fn test_query_fibers_by_type() {
        let storage = setup_storage().await;
        let timestamp = Utc::now();

        let fibers = vec![
            FiberRecord {
                fiber_id: Uuid::new_v4(),
                fiber_type: "type_a".to_string(),
                config_version: 1,
                attributes: serde_json::json!({}),
                first_activity: timestamp,
                last_activity: timestamp,
                closed: false,
            },
            FiberRecord {
                fiber_id: Uuid::new_v4(),
                fiber_type: "type_a".to_string(),
                config_version: 1,
                attributes: serde_json::json!({}),
                first_activity: timestamp + chrono::Duration::minutes(1),
                last_activity: timestamp,
                closed: false,
            },
            FiberRecord {
                fiber_id: Uuid::new_v4(),
                fiber_type: "type_b".to_string(),
                config_version: 1,
                attributes: serde_json::json!({}),
                first_activity: timestamp,
                last_activity: timestamp,
                closed: false,
            },
        ];

        for fiber in &fibers {
            storage.write_fiber(fiber).await.unwrap();
        }

        let results = storage
            .query_fibers_by_type("type_a", 10, 0)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.fiber_type, "type_a");
        }
    }

    #[tokio::test]
    async fn test_write_memberships_and_query() {
        let storage = setup_storage().await;
        let log_id = Uuid::new_v4();
        let fiber_id1 = Uuid::new_v4();
        let fiber_id2 = Uuid::new_v4();
        let timestamp = Utc::now();

        // Write a log
        let log = StoredLog {
            log_id,
            timestamp,
            source_id: "test".to_string(),
            raw_text: "test log".to_string(),
            ingestion_time: timestamp,
            config_version: 1,
        };
        storage.write_logs(&[log]).await.unwrap();

        // Write fibers
        let fiber1 = FiberRecord {
            fiber_id: fiber_id1,
            fiber_type: "test".to_string(),
            config_version: 1,
            attributes: serde_json::json!({}),
            first_activity: timestamp,
            last_activity: timestamp,
            closed: false,
        };
        let fiber2 = FiberRecord {
            fiber_id: fiber_id2,
            fiber_type: "test".to_string(),
            config_version: 1,
            attributes: serde_json::json!({}),
            first_activity: timestamp,
            last_activity: timestamp,
            closed: false,
        };
        storage.write_fiber(&fiber1).await.unwrap();
        storage.write_fiber(&fiber2).await.unwrap();

        // Write memberships
        let memberships = vec![
            FiberMembership {
                log_id,
                fiber_id: fiber_id1,
                config_version: 1,
            },
            FiberMembership {
                log_id,
                fiber_id: fiber_id2,
                config_version: 1,
            },
        ];
        storage.write_memberships(&memberships).await.unwrap();

        // Query log's fibers
        let fiber_ids = storage.get_log_fibers(log_id).await.unwrap();
        assert_eq!(fiber_ids.len(), 2);
        assert!(fiber_ids.contains(&fiber_id1));
        assert!(fiber_ids.contains(&fiber_id2));
    }

    #[tokio::test]
    async fn test_get_fiber_logs() {
        let storage = setup_storage().await;
        let fiber_id = Uuid::new_v4();
        let timestamp = Utc::now();

        // Write fiber
        let fiber = FiberRecord {
            fiber_id,
            fiber_type: "test".to_string(),
            config_version: 1,
            attributes: serde_json::json!({}),
            first_activity: timestamp,
            last_activity: timestamp,
            closed: false,
        };
        storage.write_fiber(&fiber).await.unwrap();

        // Write logs
        let log_ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        let logs: Vec<StoredLog> = log_ids
            .iter()
            .enumerate()
            .map(|(i, &log_id)| StoredLog {
                log_id,
                timestamp: timestamp + chrono::Duration::minutes(i as i64),
                source_id: "test".to_string(),
                raw_text: format!("log {}", i),
                ingestion_time: timestamp,
                config_version: 1,
            })
            .collect();
        storage.write_logs(&logs).await.unwrap();

        // Write memberships
        let memberships: Vec<FiberMembership> = log_ids
            .iter()
            .map(|&log_id| FiberMembership {
                log_id,
                fiber_id,
                config_version: 1,
            })
            .collect();
        storage.write_memberships(&memberships).await.unwrap();

        // Query fiber's logs
        let fiber_logs = storage.get_fiber_logs(fiber_id, 10, 0).await.unwrap();
        assert_eq!(fiber_logs.len(), 3);
        assert_eq!(fiber_logs[0].raw_text, "log 0");
        assert_eq!(fiber_logs[1].raw_text, "log 1");
        assert_eq!(fiber_logs[2].raw_text, "log 2");
    }

    #[tokio::test]
    async fn test_pagination() {
        let storage = setup_storage().await;
        let timestamp = Utc::now();
        let earlier = timestamp - chrono::Duration::hours(1);
        let later = timestamp + chrono::Duration::hours(1);

        // Write 5 logs
        let logs: Vec<StoredLog> = (0..5)
            .map(|i| StoredLog {
                log_id: Uuid::new_v4(),
                timestamp: timestamp + chrono::Duration::minutes(i),
                source_id: "test".to_string(),
                raw_text: format!("log {}", i),
                ingestion_time: timestamp,
                config_version: 1,
            })
            .collect();
        storage.write_logs(&logs).await.unwrap();

        // Test pagination
        let page1 = storage
            .query_logs_by_time(earlier, later, 2, 0)
            .await
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].raw_text, "log 0");
        assert_eq!(page1[1].raw_text, "log 1");

        let page2 = storage
            .query_logs_by_time(earlier, later, 2, 2)
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].raw_text, "log 2");
        assert_eq!(page2[1].raw_text, "log 3");
    }

    #[test]
    fn test_extract_pid_from_lock_error() {
        let error_msg = "IO Error: Could not set lock on file \"/path/to/db.duckdb\": Conflicting lock is held in /path/to/binary (deleted) (PID 12345). See also https://duckdb.org/docs/stable/connect/concurrency";
        assert_eq!(extract_pid_from_lock_error(error_msg), Some(12345));

        // Test with no PID
        let error_msg_no_pid = "Some other error";
        assert_eq!(extract_pid_from_lock_error(error_msg_no_pid), None);

        // Test with malformed PID
        let error_msg_malformed = "Error (PID abc)";
        assert_eq!(extract_pid_from_lock_error(error_msg_malformed), None);
    }

    #[test]
    #[cfg(unix)]
    fn test_is_process_running() {
        use std::process;

        // Test with current process (should be running)
        let current_pid = process::id();
        assert!(is_process_running(current_pid));

        // Test with a PID that definitely doesn't exist
        // Using a very high PID number that's unlikely to exist
        assert!(!is_process_running(999999));
    }
}
