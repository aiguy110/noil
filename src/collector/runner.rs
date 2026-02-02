use crate::collector::api::{CollectorState, RewindResult, SourceInfo};
use crate::collector::batch::LogBatch;
use crate::collector::batch_buffer::{BatchBuffer, BufferError, BufferStats};
use crate::collector::epoch_batcher::EpochBatcher;
use crate::collector::server::start_server;
use crate::config::types::{CollectorConfig, Config};
use crate::sequencer::merge::{run_sequencer, SequencerRunConfig};
use crate::source::reader::{LogRecord, SourceReader};
use crate::storage::checkpoint::{
    BatchBufferCheckpoint, CheckpointManager, CollectorCheckpoint, EpochBatcherCheckpoint,
    SequencerCheckpoint, SourceCheckpoint,
};
use crate::storage::traits::Storage;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("config error: {0}")]
    Config(String),

    #[error("source reader error: {0}")]
    SourceReader(#[from] crate::source::reader::ReaderError),

    #[error("sequencer error: {0}")]
    Sequencer(#[from] crate::sequencer::merge::SequencerError),

    #[error("server error: {0}")]
    Server(#[from] std::io::Error),

    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("checkpoint error: {0}")]
    Checkpoint(#[from] crate::storage::checkpoint::CheckpointError),

    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::traits::StorageError),
}

pub struct CollectorRunner {
    config: Config,
    collector_config: CollectorConfig,
    config_version: u64,
}

impl CollectorRunner {
    pub fn new(config: Config, config_version: u64) -> Result<Self, CollectorError> {
        let collector_config = config
            .collector
            .as_ref()
            .ok_or_else(|| {
                CollectorError::Config("collector config section missing".to_string())
            })?
            .clone();

        Ok(Self {
            config,
            collector_config,
            config_version,
        })
    }

    pub async fn run(self, storage: Arc<dyn Storage>) -> Result<(), CollectorError> {
        info!("Starting collector mode");

        // Initialize storage schema
        storage.init_schema().await?;

        // Generate collector ID from hostname or use default
        let collector_id = hostname::get()
            .ok()
            .and_then(|h| h.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "collector".to_string());

        info!(collector_id = %collector_id, "Collector ID set");

        // Create checkpoint manager
        let checkpoint_interval = if self.collector_config.checkpoint.enabled {
            Duration::from_secs(self.collector_config.checkpoint.interval_seconds)
        } else {
            Duration::from_secs(30)
        };

        let checkpoint_manager = CheckpointManager::new(storage.clone(), checkpoint_interval);

        // Try to load checkpoint
        let checkpoint_opt = checkpoint_manager.load_collector().await?;
        if let Some(checkpoint) = &checkpoint_opt {
            info!(
                "Loaded collector checkpoint with {} sources, sequence counter: {}",
                checkpoint.sources.len(),
                checkpoint.epoch_batcher.sequence_counter
            );
        }

        // Create epoch batcher (restored from checkpoint if available)
        let batcher = Arc::new(Mutex::new(if let Some(checkpoint) = &checkpoint_opt {
            let mut batcher = EpochBatcher::new(
                collector_id.clone(),
                self.collector_config.epoch_duration,
                self.config_version,
            );
            // Restore sequence counter and generation
            batcher.restore_from_checkpoint(
                checkpoint.epoch_batcher.sequence_counter,
                checkpoint.epoch_batcher.rewind_generation,
            );
            info!(
                "Restored epoch batcher from checkpoint: sequence={}, generation={}",
                checkpoint.epoch_batcher.sequence_counter,
                checkpoint.epoch_batcher.rewind_generation
            );
            batcher
        } else {
            EpochBatcher::new(
                collector_id.clone(),
                self.collector_config.epoch_duration,
                self.config_version,
            )
        }));

        // Create batch buffer
        let buffer = Arc::new(Mutex::new(BatchBuffer::new(
            self.collector_config.buffer.max_epochs,
            self.collector_config.buffer.strategy,
        )));

        // Create source readers
        let mut readers = Vec::new();
        let source_states: Arc<RwLock<HashMap<String, Arc<Mutex<SourceState>>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        for (source_id, source_config) in &self.config.sources {
            info!(source_id = %source_id, path = %source_config.path.display(), "Creating source reader");

            let reader = SourceReader::new(
                source_id.clone(),
                source_config,
                self.config.pipeline.errors.on_parse_error,
            )?;

            // Create shared state for this source
            let state = Arc::new(Mutex::new(SourceState {
                watermark: None,
                active: true,
            }));

            source_states
                .write()
                .await
                .insert(source_id.clone(), state);

            readers.push(reader);
        }

        if readers.is_empty() {
            return Err(CollectorError::Config(
                "No sources configured".to_string(),
            ));
        }

        // Create channels
        let (seq_tx, seq_rx) = mpsc::channel::<LogRecord>(1000);
        let (batch_tx, batch_rx) = mpsc::channel::<LogBatch>(100);

        // Global watermark tracker
        let global_watermark: Arc<RwLock<Option<DateTime<Utc>>>> =
            Arc::new(RwLock::new(None));

        // Start sequencer
        info!("Starting sequencer");
        let sequencer_config = SequencerRunConfig::from(&self.config.sequencer);
        let mut sequencer_handle = run_sequencer(readers, seq_tx, sequencer_config).await?;

        // Start epoch batcher task
        info!("Starting epoch batcher task");
        let batcher_clone = Arc::clone(&batcher);
        let buffer_for_batcher = Arc::clone(&buffer);
        let global_watermark_clone = Arc::clone(&global_watermark);
        let source_states_clone = Arc::clone(&source_states);
        let strategy = self.collector_config.buffer.strategy;

        let batcher_handle = tokio::spawn(async move {
            run_epoch_batcher(
                seq_rx,
                batch_tx,
                batcher_clone,
                buffer_for_batcher,
                global_watermark_clone,
                source_states_clone,
                strategy,
            )
            .await
        });

        // Start buffer compaction task
        info!("Starting buffer compaction task");
        let buffer_compaction = Arc::clone(&buffer);
        let compaction_handle = tokio::spawn(async move {
            run_compaction_task(buffer_compaction).await;
        });

        // Create shared state for HTTP API
        let buffer_for_get = Arc::clone(&buffer);
        let buffer_for_ack = Arc::clone(&buffer);
        let buffer_for_rewind = Arc::clone(&buffer);
        let batcher_for_rewind = Arc::clone(&batcher);

        let api_state = Arc::new(CollectorState {
            collector_id: collector_id.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            start_time: std::time::Instant::now(),
            buffer_stats: Arc::new(RwLock::new(BufferStats {
                current_epochs: 0,
                max_epochs: self.collector_config.buffer.max_epochs,
                oldest_sequence: 0,
                newest_sequence: 0,
                acknowledged_count: 0,
            })),
            watermark: Arc::clone(&global_watermark),
            source_watermarks: Arc::new(RwLock::new(Vec::new())),
            batches_fn: Arc::new(move |after: Option<u64>, limit| {
                buffer_for_get
                    .lock()
                    .unwrap()
                    .get_batches(after, limit)
            }),
            acknowledge_fn: Arc::new(move |seq_nums| {
                buffer_for_ack.lock().unwrap().acknowledge(seq_nums)
            }),
            rewind_fn: Arc::new(move |target_seq, preserve_buffer| {
                let mut b = batcher_for_rewind.lock().unwrap();
                let old_seq = 0; // TODO: track current sequence
                let new_seq = target_seq.unwrap_or(0);
                b.rewind(new_seq);

                if !preserve_buffer {
                    buffer_for_rewind.lock().unwrap().clear();
                }

                RewindResult {
                    old_sequence: old_seq,
                    new_sequence: new_seq,
                    buffer_cleared: !preserve_buffer,
                }
            }),
        });

        // Start periodic stats updater
        let stats_state = Arc::clone(&api_state);
        let stats_buffer = Arc::clone(&buffer);
        let stats_sources = Arc::clone(&source_states);
        tokio::spawn(async move {
            run_stats_updater(stats_state, stats_buffer, stats_sources).await;
        });

        // Start periodic checkpoint saving task
        if self.collector_config.checkpoint.enabled {
            let checkpoint_batcher = Arc::clone(&batcher);
            let checkpoint_buffer = Arc::clone(&buffer);
            let checkpoint_config = self.config.clone();
            let checkpoint_collector_id = collector_id.clone();
            let checkpoint_config_version = self.config_version;
            tokio::spawn(async move {
                run_checkpoint_task(
                    checkpoint_manager,
                    checkpoint_batcher,
                    checkpoint_buffer,
                    checkpoint_config,
                    checkpoint_collector_id,
                    checkpoint_config_version,
                )
                .await;
            });
        }

        // Parse listen address
        let listen_addr: std::net::SocketAddr = self
            .collector_config
            .listen
            .parse()
            .map_err(|e| CollectorError::Config(format!("Invalid listen address: {}", e)))?;

        // Start HTTP server
        info!(addr = %listen_addr, "Starting HTTP server");
        let server_state = Arc::clone(&api_state);
        let server_handle = tokio::spawn(async move {
            start_server(listen_addr, server_state)
                .await
                .map_err(CollectorError::Server)
        });

        // Start batch receiver task (consumes batches from batcher and puts in buffer)
        let batch_buffer = Arc::clone(&buffer);
        let batch_receiver_handle = tokio::spawn(async move {
            run_batch_receiver(batch_rx, batch_buffer).await;
        });

        info!("Collector running, waiting for tasks to complete");

        // Wait for sequencer to complete
        match sequencer_handle.wait().await {
            Ok(()) => info!("Sequencer completed successfully"),
            Err(e) => error!(error = %e, "Sequencer error"),
        }

        // Wait for other tasks
        match batcher_handle.await {
            Ok(()) => info!("Batcher completed successfully"),
            Err(e) => error!(error = %e, "Batcher task join error"),
        }

        match batch_receiver_handle.await {
            Ok(()) => info!("Batch receiver completed successfully"),
            Err(e) => error!(error = %e, "Batch receiver task join error"),
        }

        compaction_handle.abort();

        // Server continues running, wait for it to finish (e.g., on Ctrl+C)
        match server_handle.await {
            Ok(Ok(())) => info!("Server stopped gracefully"),
            Ok(Err(e)) => error!(error = %e, "Server error"),
            Err(e) => error!(error = %e, "Server task join error"),
        }

        Ok(())
    }
}

struct SourceState {
    watermark: Option<DateTime<Utc>>,
    active: bool,
}

async fn run_epoch_batcher(
    mut seq_rx: mpsc::Receiver<LogRecord>,
    batch_tx: mpsc::Sender<LogBatch>,
    batcher: Arc<Mutex<EpochBatcher>>,
    _buffer: Arc<Mutex<BatchBuffer>>,
    global_watermark: Arc<RwLock<Option<DateTime<Utc>>>>,
    source_states: Arc<RwLock<HashMap<String, Arc<Mutex<SourceState>>>>>,
    _strategy: crate::config::types::BufferStrategy,
) {

    while let Some(record) = seq_rx.recv().await {
        // Update source watermark
        if let Some(source_state) = source_states.read().await.get(&record.source_id) {
            if let Ok(mut state) = source_state.lock() {
                state.watermark = Some(record.timestamp);
            }
        }

        // Compute minimum watermark across all sources
        let min_watermark = {
            let states = source_states.read().await;
            states
                .values()
                .filter_map(|s| s.lock().ok()?.watermark)
                .min()
        };

        if let Some(min_wm) = min_watermark {
            *global_watermark.write().await = Some(min_wm);
        }

        // Push to batcher
        let completed_batch = batcher.lock().unwrap().push(record);

        // If batch completed, try to add to buffer
        if let Some(batch) = completed_batch {
            // Try to send to batch receiver
            if batch_tx.send(batch).await.is_err() {
                error!("Batch receiver channel closed");
                break;
            }
        }
    }

    // Sequencer done, flush current epoch
    info!("Sequencer complete, flushing current epoch");
    let watermark = global_watermark.read().await.unwrap_or_else(Utc::now);
    let maybe_batch = {
        let mut b = batcher.lock().unwrap();
        b.flush_current(watermark)
    };
    if let Some(batch) = maybe_batch {
        let _ = batch_tx.send(batch).await;
    }
}

async fn run_batch_receiver(
    mut batch_rx: mpsc::Receiver<LogBatch>,
    buffer: Arc<Mutex<BatchBuffer>>,
) {
    while let Some(batch) = batch_rx.recv().await {
        loop {
            let result = {
                let mut b = buffer.lock().unwrap();
                b.push(batch.clone())
            };

            match result {
                Ok(()) => {
                    break;
                }
                Err(BufferError::BufferFull) => {
                    // Backpressure: wait a bit and retry
                    warn!("Buffer full, applying backpressure");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

async fn run_compaction_task(buffer: Arc<Mutex<BatchBuffer>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        interval.tick().await;
        let removed = buffer.lock().unwrap().compact();
        if removed > 0 {
            info!(removed = removed, "Compacted buffer");
        }
    }
}

async fn run_stats_updater(
    state: Arc<CollectorState>,
    buffer: Arc<Mutex<BatchBuffer>>,
    source_states: Arc<RwLock<HashMap<String, Arc<Mutex<SourceState>>>>>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;

        // Update buffer stats
        let stats = buffer.lock().unwrap().stats();
        *state.buffer_stats.write().await = stats;

        // Update source watermarks
        let sources = source_states.read().await;
        let mut source_infos = Vec::new();
        for (id, state) in sources.iter() {
            if let Ok(s) = state.lock() {
                source_infos.push(SourceInfo {
                    id: id.clone(),
                    watermark: s.watermark,
                    active: s.active,
                });
            }
        }
        *state.source_watermarks.write().await = source_infos;
    }
}

async fn run_checkpoint_task(
    mut manager: CheckpointManager,
    batcher: Arc<Mutex<EpochBatcher>>,
    buffer: Arc<Mutex<BatchBuffer>>,
    config: Config,
    collector_id: String,
    config_version: u64,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        if !manager.should_save() {
            continue;
        }

        // Build checkpoint
        let checkpoint = {
            let batcher_lock = batcher.lock().unwrap();
            let buffer_lock = buffer.lock().unwrap();

            // Build source checkpoints (simplified - in full implementation would track reader state)
            let mut sources = HashMap::new();
            for (source_id, source_config) in &config.sources {
                sources.insert(
                    source_id.clone(),
                    SourceCheckpoint {
                        path: source_config.path.clone(),
                        offset: 0, // TODO: track actual offset from source readers
                        inode: 0,  // TODO: track actual inode from source readers
                        last_timestamp: None,
                    },
                );
            }

            let buffer_stats = buffer_lock.stats();

            CollectorCheckpoint {
                version: 1,
                timestamp: Utc::now(),
                config_version,
                collector_id: collector_id.clone(),
                sources,
                sequencer: SequencerCheckpoint {
                    watermarks: HashMap::new(), // TODO: track sequencer watermarks
                },
                epoch_batcher: EpochBatcherCheckpoint {
                    sequence_counter: batcher_lock.sequence_counter(),
                    rewind_generation: batcher_lock.rewind_generation(),
                    current_epoch: None, // We don't restore partial epochs
                },
                batch_buffer: BatchBufferCheckpoint {
                    oldest_sequence: buffer_stats.oldest_sequence,
                    newest_sequence: buffer_stats.newest_sequence,
                    unacknowledged_count: buffer_stats.current_epochs,
                },
            }
        };

        // Save checkpoint
        if let Err(e) = manager.save_collector(&checkpoint).await {
            error!(error = %e, "Failed to save collector checkpoint");
        } else {
            info!(
                sequence = checkpoint.epoch_batcher.sequence_counter,
                generation = checkpoint.epoch_batcher.rewind_generation,
                "Saved collector checkpoint"
            );
        }
    }
}
