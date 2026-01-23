use crate::config::types::{Config, StorageConfig};
use crate::fiber::processor::ProcessResult;
use crate::fiber::FiberProcessor;
use crate::source::reader::LogRecord;
use crate::storage::traits::{FiberMembership, FiberRecord, Storage, StorageError, StoredLog};
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Errors that can occur during pipeline operation
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("channel send error")]
    ChannelSend,

    #[error("channel receive error")]
    ChannelReceive,

    #[error("fiber processor error: {0}")]
    FiberProcessor(String),

    #[error("sequencer error: {0}")]
    Sequencer(String),
}

/// Update from fiber processor to storage writer
#[derive(Debug)]
pub struct FiberUpdate {
    /// New fiber memberships (log -> fiber)
    pub memberships: Vec<FiberMembership>,
    /// Newly created fibers
    pub new_fibers: Vec<FiberRecord>,
    /// Updated fibers (attributes changed, merged, etc.)
    pub updated_fibers: Vec<FiberRecord>,
    /// IDs of fibers that were closed
    pub closed_fiber_ids: Vec<uuid::Uuid>,
}

impl From<ProcessResult> for FiberUpdate {
    fn from(result: ProcessResult) -> Self {
        Self {
            memberships: result.memberships,
            new_fibers: result.new_fibers,
            updated_fibers: result.updated_fibers,
            closed_fiber_ids: result.closed_fiber_ids,
        }
    }
}

/// Run the fiber processor task.
///
/// Receives log records from the sequencer, writes raw logs to storage,
/// processes logs through fiber rules, and sends fiber updates to the writer.
///
/// The processor is shared via Arc<RwLock<>> to enable hot-reload from the web server.
/// During normal operation, this function holds a write lock while processing each log.
/// Hot-reload requests will wait for the lock between log processing.
pub async fn run_processor(
    mut input: mpsc::Receiver<LogRecord>,
    output: mpsc::Sender<FiberUpdate>,
    processor: Arc<RwLock<FiberProcessor>>,
    config_version: Arc<RwLock<u64>>,
    storage: Arc<dyn Storage>,
    config: &Config,
    shared_fiber_state: Option<crate::storage::checkpoint::SharedFiberProcessorState>,
) -> Result<(), PipelineError> {
    let batch_size = config.storage.batch_size;
    let flush_interval_secs = config.storage.flush_interval_seconds;

    let mut log_batch: Vec<StoredLog> = Vec::with_capacity(batch_size);
    let mut flush_interval = tokio::time::interval(Duration::from_secs(flush_interval_secs));

    info!("Fiber processor started");

    loop {
        tokio::select! {
            result = input.recv() => {
                match result {
                    Some(log) => {
                        debug!(
                            log_id = %log.id,
                            source = %log.source_id,
                            timestamp = %log.timestamp,
                            "Processing log record"
                        );

                        // Read current config version
                        let current_version = *config_version.read().await;

                        // Create stored log
                        let stored_log = StoredLog {
                            log_id: log.id,
                            timestamp: log.timestamp,
                            source_id: log.source_id.clone(),
                            raw_text: log.raw_text.clone(),
                            ingestion_time: Utc::now(),
                            config_version: current_version,
                        };

                        // Add to batch
                        log_batch.push(stored_log);

                        // Flush if batch is full
                        if log_batch.len() >= batch_size {
                            storage.write_logs(&log_batch).await?;
                            debug!(count = log_batch.len(), "Flushed log batch");
                            log_batch.clear();
                        }

                        // Acquire write lock to process log (releases between logs for hot-reload)
                        let mut processor_guard = processor.write().await;

                        // Process for fibers
                        let results = processor_guard.process_log(&log);

                        // Update shared checkpoint state if enabled
                        if let Some(ref shared_state) = shared_fiber_state {
                            if let Ok(mut guard) = shared_state.lock() {
                                *guard = processor_guard.create_checkpoint();
                            }
                        }

                        // Release lock before sending to channel
                        drop(processor_guard);

                        // Send fiber updates
                        for result in results {
                            if !result.memberships.is_empty()
                                || !result.new_fibers.is_empty()
                                || !result.updated_fibers.is_empty()
                                || !result.closed_fiber_ids.is_empty()
                            {
                                let update = FiberUpdate::from(result);
                                if output.send(update).await.is_err() {
                                    warn!("Fiber update channel closed");
                                    break;
                                }
                            }
                        }
                    }
                    None => {
                        // Input channel closed
                        info!("Input channel closed, flushing remaining data");
                        break;
                    }
                }
            }

            _ = flush_interval.tick() => {
                if !log_batch.is_empty() {
                    storage.write_logs(&log_batch).await?;
                    debug!(count = log_batch.len(), "Flushed log batch on interval");
                    log_batch.clear();
                }
            }
        }
    }

    // Final flush of logs
    if !log_batch.is_empty() {
        storage.write_logs(&log_batch).await?;
        info!(count = log_batch.len(), "Final log batch flush");
    }

    // Acquire write lock to flush all open fibers
    let mut processor_guard = processor.write().await;
    let flush_results = processor_guard.flush();

    // Update shared checkpoint state after flush to reflect that all fibers are now closed.
    // This prevents a race condition where the checkpoint is saved (after sequencer completion)
    // before the fiber processor has flushed, leading to stale open fibers in the checkpoint.
    if let Some(ref shared_state) = shared_fiber_state {
        if let Ok(mut guard) = shared_state.lock() {
            *guard = processor_guard.create_checkpoint();
        }
    }

    let open_fibers = processor_guard.total_open_fibers();
    drop(processor_guard);

    for result in flush_results {
        if !result.memberships.is_empty()
            || !result.new_fibers.is_empty()
            || !result.updated_fibers.is_empty()
            || !result.closed_fiber_ids.is_empty()
        {
            let update = FiberUpdate::from(result);
            let _ = output.send(update).await;
        }
    }

    info!(
        open_fibers = open_fibers,
        "Fiber processor shutdown complete"
    );

    Ok(())
}

/// Run the storage writer task.
///
/// Receives fiber updates and writes them to storage with batching.
pub async fn run_writer(
    mut input: mpsc::Receiver<FiberUpdate>,
    storage: Arc<dyn Storage>,
    config: &StorageConfig,
) -> Result<(), PipelineError> {
    let batch_size = config.batch_size;
    let flush_interval_secs = config.flush_interval_seconds;

    let mut membership_batch: Vec<FiberMembership> = Vec::with_capacity(batch_size);
    let mut flush_interval = tokio::time::interval(Duration::from_secs(flush_interval_secs));

    info!("Storage writer started");

    loop {
        tokio::select! {
            result = input.recv() => {
                match result {
                    Some(update) => {
                        // Write new fibers immediately
                        for fiber in &update.new_fibers {
                            if let Err(e) = storage.write_fiber(fiber).await {
                                error!(fiber_id = %fiber.fiber_id, error = %e, "Failed to write new fiber");
                            } else {
                                debug!(fiber_id = %fiber.fiber_id, "Wrote new fiber");
                            }
                        }

                        // Update existing fibers
                        for fiber in &update.updated_fibers {
                            if let Err(e) = storage.update_fiber(fiber).await {
                                error!(fiber_id = %fiber.fiber_id, error = %e, "Failed to update fiber");
                            }
                        }

                        // Handle closed fibers - mark them as closed
                        for fiber_id in &update.closed_fiber_ids {
                            // Fetch current fiber, mark as closed, and update
                            if let Ok(Some(mut fiber)) = storage.get_fiber(*fiber_id).await {
                                fiber.closed = true;
                                if let Err(e) = storage.update_fiber(&fiber).await {
                                    error!(fiber_id = %fiber_id, error = %e, "Failed to mark fiber as closed");
                                } else {
                                    debug!(fiber_id = %fiber_id, "Marked fiber as closed");
                                }
                            }
                        }

                        // Batch memberships
                        membership_batch.extend(update.memberships);

                        // Flush memberships if batch is full
                        if membership_batch.len() >= batch_size {
                            if let Err(e) = storage.write_memberships(&membership_batch).await {
                                error!(error = %e, "Failed to write memberships batch");
                            } else {
                                debug!(count = membership_batch.len(), "Wrote membership batch");
                            }
                            membership_batch.clear();
                        }
                    }
                    None => {
                        // Input channel closed
                        info!("Input channel closed, flushing remaining memberships");
                        break;
                    }
                }
            }

            _ = flush_interval.tick() => {
                if !membership_batch.is_empty() {
                    if let Err(e) = storage.write_memberships(&membership_batch).await {
                        error!(error = %e, "Failed to write memberships batch on interval");
                    } else {
                        debug!(count = membership_batch.len(), "Wrote membership batch on interval");
                    }
                    membership_batch.clear();
                }
            }
        }
    }

    // Final flush
    if !membership_batch.is_empty() {
        if let Err(e) = storage.write_memberships(&membership_batch).await {
            error!(error = %e, "Failed to write final membership batch");
        } else {
            info!(count = membership_batch.len(), "Final membership batch flush");
        }
    }

    info!("Storage writer shutdown complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{
        AttributeConfig, AttributeType, BackpressureConfig, BackpressureStrategy, CheckpointConfig,
        ErrorConfig, FiberSourceConfig, FiberTypeConfig, GapMode, ParseErrorStrategy,
        PatternConfig, PipelineConfig, SequencerConfig, SourceConfig, SourceType,
        StorageConfig, TemporalConfig, TimestampConfig, ReadConfig, ReadStart, WebConfig,
    };
    use crate::storage::duckdb::DuckDbStorage;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn make_test_config() -> Config {
        let mut sources = HashMap::new();
        sources.insert(
            "test_source".to_string(),
            SourceConfig {
                source_type: SourceType::File,
                path: PathBuf::from("/tmp/test.log"),
                timestamp: TimestampConfig {
                    pattern: r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)".to_string(),
                    format: "iso8601".to_string(),
                },
                read: ReadConfig {
                    start: ReadStart::Beginning,
                    follow: false,
                },
            },
        );

        let mut fiber_sources = HashMap::new();
        fiber_sources.insert(
            "test_source".to_string(),
            FiberSourceConfig {
                patterns: vec![PatternConfig {
                    regex: r"thread-(?P<thread_id>\d+)".to_string(),
                    release_matching_peer_keys: vec![],
                    release_self_keys: vec![],
                    close: false,
                }],
            },
        );

        let mut fiber_types = HashMap::new();
        fiber_types.insert(
            "test_fiber".to_string(),
            FiberTypeConfig {
                description: Some("Test fiber".to_string()),
                temporal: TemporalConfig {
                    max_gap: Some(Duration::from_secs(5)),
                    gap_mode: GapMode::Session,
                },
                attributes: vec![AttributeConfig {
                    name: "thread_id".to_string(),
                    attr_type: AttributeType::String,
                    key: true,
                    derived: None,
                }],
                sources: fiber_sources,
                is_source_fiber: false,
            },
        );

        Config {
            sources,
            fiber_types,
            auto_source_fibers: true,
            pipeline: PipelineConfig {
                backpressure: BackpressureConfig {
                    strategy: BackpressureStrategy::Block,
                    buffer_limit: 1000,
                },
                errors: ErrorConfig {
                    on_parse_error: ParseErrorStrategy::Drop,
                },
                checkpoint: CheckpointConfig {
                    enabled: false,
                    interval_seconds: 30,
                },
            },
            sequencer: SequencerConfig {
                batch_epoch_duration: None,
                watermark_safety_margin: Some(Duration::from_secs(1)),
            },
            storage: StorageConfig {
                path: PathBuf::from("/tmp/test.duckdb"),
                batch_size: 100,
                flush_interval_seconds: 5,
            },
            web: WebConfig {
                listen: "127.0.0.1:8080".to_string(),
            },
        }
    }

    fn make_log(source: &str, timestamp: &str, text: &str) -> LogRecord {
        LogRecord {
            id: Uuid::new_v4(),
            timestamp: timestamp.parse().unwrap(),
            source_id: source.to_string(),
            raw_text: text.to_string(),
            file_offset: 0,
        }
    }

    #[tokio::test]
    async fn test_processor_writes_logs() {
        let config = make_test_config();
        let storage = Arc::new(DuckDbStorage::in_memory().unwrap());
        storage.init_schema().await.unwrap();

        let processor = Arc::new(tokio::sync::RwLock::new(
            FiberProcessor::from_config(&config, 1).unwrap()
        ));
        let config_version = Arc::new(tokio::sync::RwLock::new(1u64));

        let (input_tx, input_rx) = mpsc::channel(100);
        let (output_tx, mut output_rx) = mpsc::channel(100);

        // Spawn processor task
        let storage_clone = storage.clone();
        let config_clone = config.clone();
        let processor_clone = Arc::clone(&processor);
        let version_clone = Arc::clone(&config_version);
        let processor_handle = tokio::spawn(async move {
            run_processor(input_rx, output_tx, processor_clone, version_clone, storage_clone, &config_clone, None).await
        });

        // Send a log
        let log = make_log(
            "test_source",
            "2025-12-04T10:00:00Z",
            "thread-5 doing stuff",
        );
        let log_id = log.id;
        input_tx.send(log).await.unwrap();

        // Receive fiber update
        let update = output_rx.recv().await.unwrap();
        assert_eq!(update.new_fibers.len(), 1);
        assert_eq!(update.memberships.len(), 1);

        // Close channels and wait for shutdown
        drop(input_tx);
        drop(output_rx);
        processor_handle.await.unwrap().unwrap();

        // Verify log was written
        let stored_log = storage.get_log(log_id).await.unwrap();
        assert!(stored_log.is_some());
        assert_eq!(stored_log.unwrap().raw_text, "thread-5 doing stuff");
    }

    #[tokio::test]
    async fn test_writer_writes_fibers_and_memberships() {
        let config = make_test_config();
        let storage = Arc::new(DuckDbStorage::in_memory().unwrap());
        storage.init_schema().await.unwrap();

        let (input_tx, input_rx) = mpsc::channel(100);

        // Spawn writer task
        let storage_clone = storage.clone();
        let storage_config = config.storage.clone();
        let writer_handle = tokio::spawn(async move {
            run_writer(input_rx, storage_clone, &storage_config).await
        });

        // Send a fiber update
        let fiber_id = Uuid::new_v4();
        let log_id = Uuid::new_v4();
        let timestamp = Utc::now();

        let update = FiberUpdate {
            memberships: vec![FiberMembership {
                log_id,
                fiber_id,
                config_version: 1,
            }],
            new_fibers: vec![FiberRecord {
                fiber_id,
                fiber_type: "test".to_string(),
                config_version: 1,
                attributes: serde_json::json!({}),
                first_activity: timestamp,
                last_activity: timestamp,
                closed: false,
            }],
            updated_fibers: vec![],
            closed_fiber_ids: vec![],
        };

        input_tx.send(update).await.unwrap();

        // Give writer time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Close channel and wait for shutdown
        drop(input_tx);
        writer_handle.await.unwrap().unwrap();

        // Verify fiber was written
        let stored_fiber = storage.get_fiber(fiber_id).await.unwrap();
        assert!(stored_fiber.is_some());
        assert_eq!(stored_fiber.unwrap().fiber_type, "test");

        // Verify membership was written
        let fiber_ids = storage.get_log_fibers(log_id).await.unwrap();
        assert!(fiber_ids.contains(&fiber_id));
    }
}
