use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::Config;
use crate::fiber::processor::FiberProcessor;
use crate::source::reader::LogRecord;
use crate::storage::{Storage, StorageError};

/// Reprocessing state stored in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReprocessState {
    pub task_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub status: ReprocessStatus,
    pub config_version: u64,
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub clear_old_results: bool,
    pub progress: ReprocessProgress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReprocessStatus {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReprocessProgress {
    pub logs_processed: usize,
    pub logs_total: usize,
    pub fibers_created: usize,
    pub memberships_written: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ReprocessError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("processor error: {0}")]
    Processor(String),

    #[error("cancelled")]
    Cancelled,

    #[error("config error: {0}")]
    Config(String),
}

/// Main reprocessing logic
pub async fn run_reprocessing(
    storage: Arc<dyn Storage>,
    config: Config,
    config_version: u64,
    time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    clear_old_results: bool,
    state: Arc<RwLock<ReprocessState>>,
) -> Result<(), ReprocessError> {
    // 1. Optional: Clear old results
    if clear_old_results {
        tracing::info!("Clearing old fiber memberships for config version {}", config_version);
        let deleted_memberships = storage
            .delete_fiber_memberships(
                config_version,
                time_range.map(|(s, _)| s),
                time_range.map(|(_, e)| e),
            )
            .await?;
        tracing::info!("Deleted {} fiber memberships", deleted_memberships);

        tracing::info!("Clearing old fibers for config version {}", config_version);
        let deleted_fibers = storage.delete_fibers(config_version).await?;
        tracing::info!("Deleted {} fibers", deleted_fibers);
    }

    // 2. Create fresh FiberProcessor
    tracing::info!("Creating fiber processor for config version {}", config_version);
    let mut processor = FiberProcessor::from_config(&config, config_version)
        .map_err(|e| ReprocessError::Processor(e.to_string()))?;

    // 3. Query and process logs in batches
    let batch_size = 1000;
    let mut offset = 0;
    let mut total_processed = 0;
    let mut total_fibers_created = 0;
    let mut total_memberships = 0;

    loop {
        // Check if cancelled
        {
            let state_guard = state.read().await;
            if matches!(state_guard.status, ReprocessStatus::Cancelled) {
                tracing::warn!("Reprocessing cancelled by user");
                return Err(ReprocessError::Cancelled);
            }
        }

        // Query batch of logs
        let logs = storage
            .query_logs_for_reprocessing(
                time_range.map(|(s, _)| s),
                time_range.map(|(_, e)| e),
                batch_size,
                offset,
            )
            .await?;

        if logs.is_empty() {
            break;
        }

        // Process each log
        for stored_log in &logs {
            // Convert StoredLog to LogRecord
            let log_record = LogRecord {
                id: stored_log.log_id,
                timestamp: stored_log.timestamp,
                source_id: stored_log.source_id.clone(),
                raw_text: stored_log.raw_text.clone(),
                file_offset: 0, // Not relevant for reprocessing from storage
            };

            let results = processor.process_log(&log_record);

            // Write results to storage
            for result in results {
                // Write new fibers
                for fiber in &result.new_fibers {
                    storage.write_fiber(fiber).await?;
                    total_fibers_created += 1;
                }

                // Update existing fibers
                for fiber in &result.updated_fibers {
                    storage.update_fiber(fiber).await?;
                }

                // Write memberships
                if !result.memberships.is_empty() {
                    storage.write_memberships(&result.memberships).await?;
                    total_memberships += result.memberships.len();
                }
            }

            total_processed += 1;

            // Update progress periodically (every 100 logs)
            if total_processed % 100 == 0 {
                let mut state_guard = state.write().await;
                state_guard.progress.logs_processed = total_processed;
                state_guard.progress.fibers_created = total_fibers_created;
                state_guard.progress.memberships_written = total_memberships;
            }
        }

        offset += logs.len();

        tracing::debug!(
            "Processed batch: {} logs, total processed: {}",
            logs.len(),
            total_processed
        );
    }

    // 4. Flush remaining open fibers
    tracing::info!("Flushing remaining open fibers");
    let flush_results = processor.flush();

    for result in flush_results {
        // Write new fibers
        for fiber in &result.new_fibers {
            storage.write_fiber(fiber).await?;
            total_fibers_created += 1;
        }

        // Update existing fibers
        for fiber in &result.updated_fibers {
            storage.update_fiber(fiber).await?;
        }

        // Write memberships
        if !result.memberships.is_empty() {
            storage.write_memberships(&result.memberships).await?;
            total_memberships += result.memberships.len();
        }
    }

    // Final progress update
    {
        let mut state_guard = state.write().await;
        state_guard.progress.logs_processed = total_processed;
        state_guard.progress.logs_total = total_processed;
        state_guard.progress.fibers_created = total_fibers_created;
        state_guard.progress.memberships_written = total_memberships;
    }

    tracing::info!(
        "Reprocessing complete: {} logs processed, {} fibers created, {} memberships written",
        total_processed,
        total_fibers_created,
        total_memberships
    );

    Ok(())
}
