use crate::collector::api::{CollectorState, RewindResult, SourceInfo};
use crate::collector::batch::LogBatch;
use crate::collector::batch_buffer::{BatchBuffer, BufferStats};
use crate::collector::epoch_batcher::EpochBatcher;
use crate::config::parse::{load_config, load_config_with_yaml};
use crate::config::reconcile::{reconcile_config_on_startup, ReconcileResult};
#[allow(deprecated)]
use crate::config::version::compute_config_version;
use crate::fiber::FiberProcessor;
use crate::parent::collector_client::CollectorClient;
use crate::parent::collector_stream::CollectorStream;
use crate::pipeline::{create_channel, run_processor, run_writer, FiberUpdate};
use crate::reprocessing::ReprocessState;
use crate::sequencer::merge::{run_sequencer, SequencerRunConfig};
use crate::source::reader::{LogRecord, SourceReader};
use crate::storage::checkpoint::{
    Checkpoint, CheckpointManager, CollectorSequencerCheckpoint, ParentCheckpoint,
    SequencerCheckpoint, SharedFiberProcessorState, SharedSourceState, SourceCheckpoint,
};
use crate::storage::duckdb::DuckDbStorage;
use crate::storage::traits::Storage;
use crate::web::run_server;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::signal;
use tokio::sync::{mpsc, oneshot, watch, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

#[derive(Debug, Error)]
pub enum RunError {
    #[error("config error: {0}")]
    Config(#[from] crate::config::parse::ConfigError),

    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::traits::StorageError),

    #[error("fiber processor error: {0}")]
    FiberProcessor(#[from] crate::fiber::RuleError),

    #[error("source reader error: {0}")]
    SourceReader(#[from] crate::source::reader::ReaderError),

    #[error("sequencer error: {0}")]
    Sequencer(#[from] crate::sequencer::merge::SequencerError),

    #[error("pipeline error: {0}")]
    Pipeline(#[from] crate::pipeline::PipelineError),

    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("web server error: {0}")]
    WebServer(String),
}

/// Reconcile config file with database and log the result.
/// Returns an error if there's an unresolved conflict.
async fn reconcile_and_log(
    config_path: &PathBuf,
    storage: &dyn Storage,
) -> Result<(), RunError> {
    info!("Reconciling config with database");
    let result = reconcile_config_on_startup(config_path, storage).await?;

    match &result {
        ReconcileResult::Initialized { hash } => {
            info!(hash = %hash, "Config initialized in database");
        }
        ReconcileResult::NoChange => {
            info!("Config unchanged");
        }
        ReconcileResult::FastForwardedFile { from_hash, to_hash } => {
            info!(from = %from_hash, to = %to_hash, "Config file fast-forwarded from database");
        }
        ReconcileResult::FastForwardedDB { from_hash, to_hash } => {
            info!(from = %from_hash, to = %to_hash, "Database fast-forwarded from config file");
        }
        ReconcileResult::Merged { merged_hash, .. } => {
            info!(hash = %merged_hash, "Config merged successfully");
        }
        ReconcileResult::UnresolvedConflict { conflict_file } => {
            error!(conflict_file = %conflict_file, "Unresolved config conflict");
            return Err(crate::config::parse::ConfigError::Validation(
                format!("Unresolved conflict in {}", conflict_file)
            ).into());
        }
    }

    Ok(())
}

fn format_web_url(listen: &str) -> String {
    let trimmed = listen.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    let (host, port) = trimmed
        .rsplit_once(':')
        .map(|(host, port)| (host, port))
        .unwrap_or((trimmed, ""));

    let host = match host {
        "0.0.0.0" => "127.0.0.1",
        "[::]" => "[::1]",
        _ => host,
    };

    if port.is_empty() {
        format!("http://{}", host)
    } else {
        format!("http://{}:{}", host, port)
    }
}

pub async fn run(config_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = match config_path {
        Some(path) => path,
        None => {
            eprintln!("Error: config not found");
            eprintln!("Searched locations:");
            eprintln!("  ~/.config/noil/config.yml");
            eprintln!("  /etc/noil/config.yml");
            eprintln!("\nUse --config <path> to specify a config file, or run 'noil config init' to generate one.");
            std::process::exit(1);
        }
    };

    run_pipeline(&config_path).await.map_err(|e| e.into())
}

async fn run_pipeline(config_path: &PathBuf) -> Result<(), RunError> {
    info!(config_path = %config_path.display(), "Loading configuration");

    // === Phase 1: Initialize storage + reconcile config (always) ===
    let temp_config = load_config(config_path)?;
    let storage_path = &temp_config.storage.path;
    info!(path = %storage_path.display(), "Initializing storage");
    let storage = Arc::new(DuckDbStorage::new(storage_path)?);
    storage.init_schema().await?;
    reconcile_and_log(config_path, storage.as_ref()).await?;

    // Load config (may have been updated by reconciliation)
    let (mut config, config_yaml) = load_config_with_yaml(config_path)?;

    // Compute config version
    #[allow(deprecated)]
    let config_version = compute_config_version(config_path)
        .map_err(|e| crate::config::parse::ConfigError::Io(e))?;
    info!(config_version = config_version, "Computed config version");

    // === Phase 2: Determine capabilities from config ===
    let has_local = config.has_local_sources();
    let has_remote = config.has_remote_sources();
    let stores = config.stores_logs();
    let serves_collector = config.has_collector_serving();

    info!(
        has_local_sources = has_local,
        has_remote_sources = has_remote,
        stores_logs = stores,
        serves_collector = serves_collector,
        "Pipeline capabilities"
    );

    // === Phase 3: Auto source fibers for remote sources ===
    if config.auto_source_fibers || has_remote {
        info!("Adding auto-generated source fiber types from database");
        let source_ids = storage.get_all_source_ids().await
            .map_err(|e| RunError::Storage(e))?;
        if !source_ids.is_empty() {
            info!("Found {} source(s) in database: {:?}", source_ids.len(), source_ids);
            crate::config::parse::add_auto_source_fibers_from_list(&mut config, &source_ids);
        } else {
            info!("No sources found in database yet");
        }
    }

    // === Phase 4: Load checkpoint if enabled ===
    let checkpoint = if config.pipeline.checkpoint.enabled {
        let checkpoint_mgr = CheckpointManager::new(
            storage.clone(),
            Duration::from_secs(config.pipeline.checkpoint.interval_seconds),
        );
        match checkpoint_mgr.load().await {
            Ok(Some(checkpoint)) => {
                if checkpoint.config_version != config_version {
                    warn!(
                        "Checkpoint config version {} does not match current version {}, starting fresh",
                        checkpoint.config_version, config_version
                    );
                    None
                } else {
                    info!("Loaded checkpoint from {}", checkpoint.timestamp);
                    Some(checkpoint)
                }
            }
            Ok(None) => None,
            Err(e) => {
                warn!("Failed to load checkpoint: {}, starting fresh", e);
                None
            }
        }
    } else {
        None
    };

    // Load parent checkpoint for remote sources
    let parent_checkpoint = if has_remote {
        let checkpoint_mgr = CheckpointManager::new(
            storage.clone(),
            Duration::from_secs(30),
        );
        match checkpoint_mgr.load_parent().await {
            Ok(cp) => cp,
            Err(e) => {
                warn!("Failed to load parent checkpoint: {}, starting fresh", e);
                None
            }
        }
    } else {
        None
    };

    // Create fiber processor (if storing logs)
    let shared_processor = if stores {
        info!("Creating fiber processor");
        let mut fiber_processor = FiberProcessor::from_config(&config, config_version)?;

        // Restore from standalone checkpoint
        if let Some(ref cp) = checkpoint {
            info!("Restoring fiber processor state from checkpoint");
            fiber_processor.restore_from_checkpoint(&cp.fiber_processors);

            // Close orphaned fibers
            let checkpointed_fiber_ids: std::collections::HashSet<uuid::Uuid> = cp
                .fiber_processors
                .values()
                .flat_map(|fp| fp.open_fibers.iter().map(|f| f.fiber_id))
                .collect();
            info!(
                checkpointed_fibers = checkpointed_fiber_ids.len(),
                "Closing orphaned fibers not in checkpoint"
            );
            match storage.close_orphaned_fibers(&checkpointed_fiber_ids).await {
                Ok(count) if count > 0 => {
                    warn!(closed_count = count, "Closed orphaned fibers");
                }
                Ok(_) => info!("No orphaned fibers found"),
                Err(e) => error!(error = %e, "Failed to close orphaned fibers"),
            }
        }

        // Restore from parent checkpoint
        if let Some(ref cp) = parent_checkpoint {
            if !cp.fiber_processors.is_empty() {
                for (fiber_type, fiber_checkpoint) in &cp.fiber_processors {
                    if let Some(typed_processor) = fiber_processor.get_processor_mut(fiber_type) {
                        typed_processor.restore_from_checkpoint(fiber_checkpoint);
                        info!(
                            fiber_type = %fiber_type,
                            open_fibers = fiber_checkpoint.open_fibers.len(),
                            "Restored fiber processor checkpoint from parent"
                        );
                    } else {
                        warn!(
                            fiber_type = %fiber_type,
                            "Parent checkpoint contains unknown fiber type; skipping"
                        );
                    }
                }
            }
        }

        info!(
            fiber_types = config.fiber_types_or_empty().len(),
            open_fibers = fiber_processor.total_open_fibers(),
            "Fiber processor initialized"
        );
        Some(Arc::new(RwLock::new(fiber_processor)))
    } else {
        None
    };

    // Create shared state for fiber processor checkpointing
    let shared_fiber_state: Option<SharedFiberProcessorState> = if stores && config.pipeline.checkpoint.enabled {
        let initial_state = shared_processor.as_ref().unwrap().read().await.create_checkpoint();
        Some(Arc::new(std::sync::Mutex::new(initial_state)))
    } else {
        None
    };

    // Shared config state for web server and hot-reload
    let shared_config = Arc::new(RwLock::new(config.clone()));
    let shared_config_yaml = Arc::new(RwLock::new(config_yaml));
    let shared_version = Arc::new(RwLock::new(config_version));
    let shared_reprocess_state: Arc<RwLock<Option<ReprocessState>>> = Arc::new(RwLock::new(None));

    // Create channels for the pipeline
    let buffer_size = config.pipeline.backpressure.buffer_limit;
    let (seq_tx, seq_rx) = mpsc::channel::<LogRecord>(buffer_size);

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Channel for triggering immediate checkpoint saves
    let (checkpoint_save_tx, mut checkpoint_save_rx) = mpsc::channel::<()>(10);

    // === Phase 5: Local sources (if has_local_sources) ===
    let mut source_readers = Vec::new();
    let mut shared_source_states: HashMap<String, SharedSourceState> = HashMap::new();
    let mut sequencer_handle = None;

    if has_local {
        for (source_id, source_config) in &config.sources {
            info!(source_id = %source_id, path = %source_config.path.display(), "Creating source reader");

            let reader = if let Some(ref cp) = checkpoint {
                if let Some(source_checkpoint) = cp.sources.get(source_id) {
                    if let Ok(metadata) = std::fs::metadata(&source_config.path) {
                        #[cfg(unix)]
                        let current_inode = {
                            use std::os::unix::fs::MetadataExt;
                            metadata.ino()
                        };
                        #[cfg(not(unix))]
                        let current_inode = {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            metadata.len().hash(&mut hasher);
                            if let Ok(modified) = metadata.modified() {
                                modified.hash(&mut hasher);
                            }
                            hasher.finish()
                        };

                        if current_inode == source_checkpoint.inode {
                            info!(
                                source_id = %source_id,
                                offset = source_checkpoint.offset,
                                "Restoring source reader from checkpoint"
                            );
                            SourceReader::new_with_offset(
                                source_id.clone(),
                                source_config,
                                config.pipeline.errors.on_parse_error,
                                source_checkpoint.offset,
                            )?
                        } else {
                            warn!(
                                source_id = %source_id,
                                "File inode changed (rotation detected), starting from beginning"
                            );
                            SourceReader::new(
                                source_id.clone(),
                                source_config,
                                config.pipeline.errors.on_parse_error,
                            )?
                        }
                    } else {
                        warn!(source_id = %source_id, "Cannot read file metadata, starting from beginning");
                        SourceReader::new(
                            source_id.clone(),
                            source_config,
                            config.pipeline.errors.on_parse_error,
                        )?
                    }
                } else {
                    SourceReader::new(
                        source_id.clone(),
                        source_config,
                        config.pipeline.errors.on_parse_error,
                    )?
                }
            } else {
                SourceReader::new(
                    source_id.clone(),
                    source_config,
                    config.pipeline.errors.on_parse_error,
                )?
            };

            let (reader, state) = reader.with_shared_state();
            shared_source_states.insert(source_id.clone(), state);
            source_readers.push(reader);
        }

        if !source_readers.is_empty() {
            info!("Starting sequencer for local sources");
            let sequencer_config = SequencerRunConfig::from(&config.sequencer);
            sequencer_handle = Some(run_sequencer(source_readers, seq_tx.clone(), sequencer_config).await?);
        }
    }

    // === Phase 6: Remote sources (if has_remote_sources) ===
    let mut collector_tasks: Vec<JoinHandle<()>> = Vec::new();
    let ack_state = if has_remote {
        let remote_config = config.remote_collectors.as_ref().unwrap();

        // Track acknowledgment state
        let mut collector_checkpoints: HashMap<String, CollectorSequencerCheckpoint> = parent_checkpoint
            .as_ref()
            .map(|cp| cp.collectors.clone())
            .unwrap_or_default();

        // Create collector streams
        let mut collector_streams = Vec::new();
        for endpoint in &remote_config.endpoints {
            info!(
                collector_id = %endpoint.id,
                url = %endpoint.url,
                "Creating collector client"
            );

            let last_ack_seq = collector_checkpoints
                .get(&endpoint.id)
                .map(|cp| cp.last_acknowledged_sequence)
                .unwrap_or(0);
            let has_checkpoint_for_collector = collector_checkpoints.contains_key(&endpoint.id);

            collector_checkpoints
                .entry(endpoint.id.clone())
                .or_insert_with(|| CollectorSequencerCheckpoint {
                    collector_id: endpoint.id.clone(),
                    last_sequence: last_ack_seq,
                    last_acknowledged_sequence: last_ack_seq,
                    watermark: None,
                });

            let client = CollectorClient::new(endpoint)
                .map_err(|e| RunError::Config(crate::config::parse::ConfigError::Validation(e.to_string())))?;
            let mut stream = CollectorStream::new(client);

            if has_checkpoint_for_collector {
                info!(
                    collector_id = %endpoint.id,
                    last_ack = last_ack_seq,
                    "Resuming collector from last acknowledged sequence"
                );
                stream.reset_to_sequence(last_ack_seq);
            }

            collector_streams.push(stream);
        }

        // Shared acknowledgment state
        let ack_state = Arc::new(RwLock::new(AcknowledgmentState {
            pending_acks: HashMap::new(),
            collector_checkpoints,
        }));

        // Spawn per-collector polling tasks
        for mut stream in collector_streams {
            let tx = seq_tx.clone();
            let poll_interval = remote_config.poll_interval;
            let ack_state_clone = Arc::clone(&ack_state);

            let handle = tokio::spawn(async move {
                let collector_id = stream.collector_id().to_string();
                info!(collector_id = %collector_id, "Collector polling task started");

                let mut poll_timer = tokio::time::interval(poll_interval);
                let mut last_sequence: Option<u64> = None;

                loop {
                    poll_timer.tick().await;

                    let fetched = match stream.fetch_new_logs().await {
                        Ok(count) => count,
                        Err(e) => {
                            error!(
                                collector_id = %collector_id,
                                error = %e,
                                "Error fetching from collector"
                            );
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    };

                    if fetched == 0 {
                        debug!(collector_id = %collector_id, "No records available");
                        continue;
                    }

                    let current_sequence = stream.last_sequence();
                    if last_sequence.is_none() || Some(current_sequence) != last_sequence {
                        let mut state = ack_state_clone.write().await;
                        state
                            .pending_acks
                            .entry(collector_id.clone())
                            .or_insert_with(Vec::new)
                            .push(current_sequence);

                        if let Some(cp) = state.collector_checkpoints.get_mut(&collector_id) {
                            cp.last_sequence = current_sequence;
                            if let Some((_gen, ts)) = stream.watermark_with_generation() {
                                cp.watermark = Some(ts);
                            }
                        }

                        last_sequence = Some(current_sequence);
                    }

                    while let Some(record) = stream.pop_queued_record() {
                        if tx.send(record).await.is_err() {
                            warn!(collector_id = %collector_id, "Sequencer channel closed, stopping");
                            return;
                        }
                    }
                }
            });

            collector_tasks.push(handle);
        }

        Some(ack_state)
    } else {
        None
    };

    // === Phase 7: Drop seq_tx — all producers have cloned it ===
    drop(seq_tx);

    // === Phase 8: Stream fan-out + fiber processing + collector serving ===

    // Set up collector serving state if needed
    let collector_state = if serves_collector {
        let collector_config = config.collector.as_ref().unwrap();

        let collector_id = hostname::get()
            .ok()
            .and_then(|h| h.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "collector".to_string());

        info!(collector_id = %collector_id, "Setting up collector serving");

        let batcher = Arc::new(Mutex::new(EpochBatcher::new(
            collector_id.clone(),
            collector_config.epoch_duration,
            config_version,
        )));

        let buffer = Arc::new(Mutex::new(BatchBuffer::new(
            collector_config.buffer.max_epochs,
            collector_config.buffer.strategy,
        )));

        // Source states for collector status API
        let source_states: Arc<RwLock<HashMap<String, Arc<Mutex<CollectorSourceState>>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        for source_id in config.sources.keys() {
            source_states.write().await.insert(
                source_id.clone(),
                Arc::new(Mutex::new(CollectorSourceState {
                    watermark: None,
                    active: true,
                })),
            );
        }

        // Global watermark for collector
        let global_watermark: Arc<RwLock<Option<DateTime<Utc>>>> = Arc::new(RwLock::new(None));

        // Build collector API state
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
                max_epochs: collector_config.buffer.max_epochs,
                oldest_sequence: 0,
                newest_sequence: 0,
                acknowledged_count: 0,
            })),
            watermark: Arc::clone(&global_watermark),
            source_watermarks: Arc::new(RwLock::new(Vec::new())),
            batches_fn: Arc::new(move |after: Option<u64>, limit| {
                buffer_for_get.lock().unwrap().get_batches(after, limit)
            }),
            acknowledge_fn: Arc::new(move |seq_nums| {
                buffer_for_ack.lock().unwrap().acknowledge(seq_nums)
            }),
            rewind_fn: Arc::new(move |target_seq, preserve_buffer| {
                let mut b = batcher_for_rewind.lock().unwrap();
                let old_seq = 0;
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

        // Start stats updater
        let stats_state = Arc::clone(&api_state);
        let stats_buffer = Arc::clone(&buffer);
        let stats_sources = Arc::clone(&source_states);
        tokio::spawn(async move {
            run_stats_updater(stats_state, stats_buffer, stats_sources).await;
        });

        // Start compaction task
        let buffer_compaction = Arc::clone(&buffer);
        tokio::spawn(async move {
            run_compaction_task(buffer_compaction).await;
        });

        Some(CollectorServingState {
            api_state,
            batcher,
            buffer,
            global_watermark,
            source_states,
        })
    } else {
        None
    };

    // Determine what consumes seq_rx
    let (fiber_tx, fiber_rx) = if stores {
        let (tx, rx) = create_channel::<FiberUpdate>(buffer_size);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    // If both stores_logs and serves_collector, we need a tee
    let mut processor_handle = None;
    let mut writer_handle = None;
    let mut batcher_handle = None;
    let mut batch_receiver_handle = None;

    if stores && serves_collector {
        // Tee: clone records to both fiber processing and epoch batcher
        let (processor_tx, processor_rx) = mpsc::channel::<LogRecord>(buffer_size);
        let (batcher_input_tx, batcher_input_rx) = mpsc::channel::<LogRecord>(buffer_size);

        // Spawn tee task
        tokio::spawn(async move {
            let mut seq_rx = seq_rx;
            while let Some(record) = seq_rx.recv().await {
                let record_clone = record.clone();
                // Send to processor; if it fails, log but continue for batcher
                if processor_tx.send(record).await.is_err() {
                    warn!("Processor channel closed in tee");
                }
                if batcher_input_tx.send(record_clone).await.is_err() {
                    warn!("Batcher channel closed in tee");
                    break;
                }
            }
        });

        // Start fiber processing
        let (ph, wh) = start_fiber_processing(
            processor_rx,
            fiber_tx.unwrap(),
            fiber_rx.unwrap(),
            Arc::clone(shared_processor.as_ref().unwrap()),
            Arc::clone(&shared_version),
            storage.clone(),
            &config,
            shared_fiber_state.clone(),
        );
        processor_handle = Some(ph);
        writer_handle = Some(wh);

        // Start epoch batcher
        let cs = collector_state.as_ref().unwrap();
        let (bh, brh) = start_epoch_batcher(
            batcher_input_rx,
            Arc::clone(&cs.batcher),
            Arc::clone(&cs.buffer),
            Arc::clone(&cs.global_watermark),
            Arc::clone(&cs.source_states),
        );
        batcher_handle = Some(bh);
        batch_receiver_handle = Some(brh);
    } else if stores {
        // Only fiber processing
        let (ph, wh) = start_fiber_processing(
            seq_rx,
            fiber_tx.unwrap(),
            fiber_rx.unwrap(),
            Arc::clone(shared_processor.as_ref().unwrap()),
            Arc::clone(&shared_version),
            storage.clone(),
            &config,
            shared_fiber_state.clone(),
        );
        processor_handle = Some(ph);
        writer_handle = Some(wh);
    } else if serves_collector {
        // Only collector serving
        let cs = collector_state.as_ref().unwrap();
        let (bh, brh) = start_epoch_batcher(
            seq_rx,
            Arc::clone(&cs.batcher),
            Arc::clone(&cs.buffer),
            Arc::clone(&cs.global_watermark),
            Arc::clone(&cs.source_states),
        );
        batcher_handle = Some(bh);
        batch_receiver_handle = Some(brh);
    } else {
        // No fiber processing, no collector serving — drain seq_rx
        tokio::spawn(async move {
            let mut rx = seq_rx;
            while let Some(_record) = rx.recv().await {
                // Discard
            }
            info!("Drained sequencer output (no consumers configured)");
        });
    }

    // === Phase 9: Start periodic checkpoint task (if enabled) ===
    let checkpoint_handle = if config.pipeline.checkpoint.enabled {
        let checkpoint_storage = storage.clone();
        let interval = config.pipeline.checkpoint.interval_seconds;
        let states = shared_source_states.clone();
        let source_configs = config.sources.clone();
        let fiber_state = shared_fiber_state.clone();
        let mut shutdown_watch = shutdown_rx.clone();

        info!(interval_seconds = interval, "Starting checkpoint task");

        Some(tokio::spawn(async move {
            let mut checkpoint_mgr = CheckpointManager::new(
                checkpoint_storage,
                Duration::from_secs(interval),
            );
            let mut interval_ticker = tokio::time::interval(Duration::from_secs(interval));

            macro_rules! save_checkpoint {
                () => {{
                    let mut sources = HashMap::new();
                    for (source_id, state) in &states {
                        if let Ok(guard) = state.lock() {
                            sources.insert(
                                source_id.clone(),
                                SourceCheckpoint {
                                    path: source_configs[source_id].path.clone(),
                                    offset: guard.offset,
                                    inode: guard.inode,
                                    last_timestamp: guard.last_timestamp,
                                },
                            );
                        }
                    }

                    let fiber_processors = if let Some(ref fiber_state) = fiber_state {
                        if let Ok(guard) = fiber_state.lock() {
                            guard.clone()
                        } else {
                            HashMap::new()
                        }
                    } else {
                        HashMap::new()
                    };

                    let checkpoint = Checkpoint {
                        version: 1,
                        timestamp: chrono::Utc::now(),
                        config_version,
                        sources,
                        sequencer: SequencerCheckpoint {
                            watermarks: HashMap::new(),
                        },
                        fiber_processors,
                    };

                    if let Err(e) = checkpoint_mgr.save(&checkpoint).await {
                        error!(error = %e, "Failed to save checkpoint");
                    } else {
                        debug!("Checkpoint saved successfully");
                    }
                }};
            }

            loop {
                tokio::select! {
                    _ = interval_ticker.tick() => {
                        save_checkpoint!();
                    }
                    Some(()) = checkpoint_save_rx.recv() => {
                        info!("Immediate checkpoint save requested");
                        save_checkpoint!();
                    }
                    _ = shutdown_watch.changed() => {
                        info!("Checkpoint task shutting down, saving final checkpoint");
                        save_checkpoint!();
                        break;
                    }
                }
            }
        }))
    } else {
        None
    };

    // === Phase 10: Acknowledgment + parent checkpoint tasks (if remote) ===
    let mut ack_handle = None;
    let mut parent_checkpoint_handle = None;
    if has_remote {
        let remote_config = config.remote_collectors.as_ref().unwrap();
        let ack_state_ref = ack_state.as_ref().unwrap();

        // Acknowledgment task
        let collectors = remote_config.endpoints.clone();
        let ack_st = Arc::clone(ack_state_ref);
        ack_handle = Some(tokio::spawn(async move {
            info!("Acknowledgment task started");
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                flush_pending_acks(&collectors, Arc::clone(&ack_st)).await;
            }
        }));

        // Parent checkpoint task
        let ack_st = Arc::clone(ack_state_ref);
        let fp = shared_processor.as_ref().map(|p| Arc::clone(p));
        let cv = config_version;
        let st = storage.clone();
        parent_checkpoint_handle = Some(tokio::spawn(async move {
            run_parent_checkpoint_task(st, ack_st, cv, fp).await;
        }));
    }

    // Collector serving checkpoint task
    if serves_collector && config.collector.as_ref().unwrap().checkpoint.enabled {
        let cs = collector_state.as_ref().unwrap();
        let checkpoint_batcher = Arc::clone(&cs.batcher);
        let checkpoint_buffer = Arc::clone(&cs.buffer);
        let collector_id = hostname::get()
            .ok()
            .and_then(|h| h.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "collector".to_string());
        let checkpoint_config_version = config_version;

        // Build source checkpoint states from shared_source_states
        let source_checkpoint_states: Arc<RwLock<HashMap<String, SharedSourceState>>> =
            Arc::new(RwLock::new(shared_source_states.clone()));

        let checkpoint_manager = CheckpointManager::new(storage.clone(), Duration::from_secs(10));
        tokio::spawn(async move {
            run_collector_checkpoint_task(
                checkpoint_manager,
                checkpoint_batcher,
                checkpoint_buffer,
                source_checkpoint_states,
                collector_id,
                checkpoint_config_version,
            )
            .await;
        });
    }

    // === Phase 11: Web server (always) ===
    info!("Starting web server on {}", config.web.listen);
    let web_storage = storage.clone();
    let web_config = config.web.clone();
    let web_shutdown_rx = shutdown_rx.clone();
    // Use shared_processor for web server, or create a dummy one if not storing logs
    let web_processor = if let Some(ref p) = shared_processor {
        Arc::clone(p)
    } else {
        // Create an empty processor for the web server when not storing logs
        Arc::new(RwLock::new(FiberProcessor::from_config(&config, config_version)?))
    };
    let web_shared_config = Arc::clone(&shared_config);
    let web_shared_config_yaml = Arc::clone(&shared_config_yaml);
    let web_version = Arc::clone(&shared_version);
    let web_reprocess_state = Arc::clone(&shared_reprocess_state);
    let web_config_path = config_path.clone();
    // Extract collector API state for the web server (if serving collector protocol)
    let web_collector_state = collector_state.as_ref().map(|cs| Arc::clone(&cs.api_state));

    let web_handle = tokio::spawn(async move {
        run_server(
            web_storage,
            web_processor,
            web_shared_config,
            web_version,
            web_reprocess_state,
            web_config_path,
            web_shared_config_yaml,
            web_config,
            web_shutdown_rx,
            web_collector_state,
        )
        .await
        .map_err(|e| RunError::WebServer(e.to_string()))
    });

    let web_url = format_web_url(&config.web.listen);
    info!("Pipeline started, press Ctrl+C to shutdown");

    // === Phase 12: Shutdown logic ===
    // Create channel to signal abort to sequencer wait task
    let (abort_tx, abort_rx) = oneshot::channel::<()>();

    let sequencer_wait_task = if let Some(mut handle) = sequencer_handle {
        Some(tokio::spawn(async move {
            tokio::select! {
                result = handle.wait() => Ok(result),
                _ = abort_rx => {
                    info!("Aborting sequencer and source readers");
                    handle.abort();
                    Err(())
                }
            }
        }))
    } else {
        None
    };

    // Wait for shutdown signal or sequencer completion
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received");

            if config.pipeline.checkpoint.enabled {
                info!("Triggering checkpoint save before shutdown");
                let _ = checkpoint_save_tx.send(()).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            let _ = shutdown_tx.send(true);
            let _ = abort_tx.send(());

            // Abort collector polling tasks
            for handle in &collector_tasks {
                handle.abort();
            }
        }
        result = async {
            if let Some(task) = sequencer_wait_task {
                task.await
            } else if !collector_tasks.is_empty() {
                // Wait for all collector tasks (they run forever, so wait for any error/completion)
                // In practice, collector tasks run until aborted
                std::future::pending::<Result<Result<Result<(), crate::sequencer::merge::SequencerError>, ()>, tokio::task::JoinError>>().await
            } else {
                std::future::pending::<Result<Result<Result<(), crate::sequencer::merge::SequencerError>, ()>, tokio::task::JoinError>>().await
            }
        } => {
            match result {
                Ok(Ok(Ok(()))) => {
                    info!("Log ingestion complete. Waiting for pipeline to finish.");

                    // Wait for fiber processor to complete
                    if let Some(handle) = processor_handle.take() {
                        match handle.await {
                            Ok(Ok(())) => info!("Fiber processor completed successfully"),
                            Ok(Err(e)) => error!(error = %e, "Fiber processor error"),
                            Err(e) => error!(error = %e, "Fiber processor join error"),
                        }
                    }

                    // Trigger checkpoint save after processor completion
                    if config.pipeline.checkpoint.enabled {
                        info!("Triggering checkpoint save after fiber processor completion");
                        let _ = checkpoint_save_tx.send(()).await;
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }

                    info!("Web server continues running at {}. Press Ctrl+C to shutdown.", web_url);
                    match signal::ctrl_c().await {
                        Ok(()) => {
                            info!("Shutdown signal received");
                            if config.pipeline.checkpoint.enabled {
                                info!("Triggering final checkpoint save");
                                let _ = checkpoint_save_tx.send(()).await;
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                            let _ = shutdown_tx.send(true);
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to listen for shutdown signal");
                            let _ = shutdown_tx.send(true);
                        }
                    }
                }
                Ok(Ok(Err(e))) => {
                    error!(error = %e, "Sequencer error, shutting down");
                    let _ = shutdown_tx.send(true);
                }
                Ok(Err(())) => {
                    // Sequencer was aborted
                }
                Err(e) => {
                    error!(error = %e, "Sequencer task join error, shutting down");
                    let _ = shutdown_tx.send(true);
                }
            }
        }
    }

    // === Shutdown: drain and cleanup ===
    info!("Waiting for pipeline tasks to complete");

    // Flush remote acknowledgments
    if has_remote {
        if let Some(ref ack_st) = ack_state {
            let remote_config = config.remote_collectors.as_ref().unwrap();
            flush_pending_acks(&remote_config.endpoints, Arc::clone(ack_st)).await;
        }

        // Abort periodic tasks
        if let Some(h) = ack_handle {
            h.abort();
        }
        if let Some(h) = parent_checkpoint_handle {
            h.abort();
        }

        // Save final parent checkpoint
        if let Some(ref ack_st) = ack_state {
            let fp = shared_processor.as_ref().map(|p| Arc::clone(p));
            if let Err(e) = save_parent_checkpoint_once(storage.clone(), Arc::clone(ack_st), config_version, fp).await {
                error!(error = %e, "Failed to save final parent checkpoint");
            }
        }
    }

    // Wait for checkpoint task
    if let Some(handle) = checkpoint_handle {
        match handle.await {
            Ok(()) => info!("Checkpoint task completed successfully"),
            Err(e) => error!(error = %e, "Checkpoint task join error"),
        }
    }

    // Wait for processor (if not already awaited)
    if let Some(handle) = processor_handle {
        match handle.await {
            Ok(Ok(())) => info!("Processor task completed successfully"),
            Ok(Err(e)) => error!(error = %e, "Processor task error"),
            Err(e) => error!(error = %e, "Processor task join error"),
        }
    }

    // Wait for writer
    if let Some(handle) = writer_handle {
        match handle.await {
            Ok(Ok(())) => info!("Writer task completed successfully"),
            Ok(Err(e)) => error!(error = %e, "Writer task error"),
            Err(e) => error!(error = %e, "Writer task join error"),
        }
    }

    // Wait for batcher
    if let Some(handle) = batcher_handle {
        match handle.await {
            Ok(()) => info!("Batcher completed successfully"),
            Err(e) => error!(error = %e, "Batcher task join error"),
        }
    }

    if let Some(handle) = batch_receiver_handle {
        match handle.await {
            Ok(()) => info!("Batch receiver completed successfully"),
            Err(e) => error!(error = %e, "Batch receiver task join error"),
        }
    }

    // Wait for web server with graceful shutdown timeout
    match tokio::time::timeout(Duration::from_secs(5), web_handle).await {
        Ok(Ok(Ok(()))) => info!("Web server stopped gracefully"),
        Ok(Ok(Err(e))) => error!(error = %e, "Web server error"),
        Ok(Err(e)) => error!(error = %e, "Web server join error"),
        Err(_) => {
            warn!("Web server shutdown timed out after 5 seconds");
        }
    }

    info!("Pipeline shutdown complete");
    Ok(())
}

// === Helper functions ===

/// Start fiber processing (processor + writer tasks)
fn start_fiber_processing(
    input: mpsc::Receiver<LogRecord>,
    fiber_tx: mpsc::Sender<FiberUpdate>,
    fiber_rx: mpsc::Receiver<FiberUpdate>,
    processor: Arc<RwLock<FiberProcessor>>,
    config_version: Arc<RwLock<u64>>,
    storage: Arc<dyn Storage>,
    config: &crate::config::types::Config,
    shared_fiber_state: Option<SharedFiberProcessorState>,
) -> (JoinHandle<Result<(), crate::pipeline::PipelineError>>, JoinHandle<Result<(), crate::pipeline::PipelineError>>) {
    info!("Starting fiber processor task");
    let processor_storage = storage.clone();
    let processor_config = config.clone();
    let processor_handle = tokio::spawn(async move {
        run_processor(input, fiber_tx, processor, config_version, processor_storage, &processor_config, shared_fiber_state).await
    });

    info!("Starting storage writer task");
    let writer_storage = storage;
    let writer_config = config.storage.clone();
    let writer_handle = tokio::spawn(async move {
        run_writer(fiber_rx, writer_storage, &writer_config).await
    });

    (processor_handle, writer_handle)
}

/// Start epoch batcher for collector serving
fn start_epoch_batcher(
    input: mpsc::Receiver<LogRecord>,
    batcher: Arc<Mutex<EpochBatcher>>,
    buffer: Arc<Mutex<BatchBuffer>>,
    global_watermark: Arc<RwLock<Option<DateTime<Utc>>>>,
    source_states: Arc<RwLock<HashMap<String, Arc<Mutex<CollectorSourceState>>>>>,
) -> (JoinHandle<()>, JoinHandle<()>) {
    let (batch_tx, batch_rx) = mpsc::channel::<LogBatch>(100);

    let batcher_handle = tokio::spawn(async move {
        run_epoch_batcher_task(input, batch_tx, batcher, global_watermark, source_states).await;
    });

    let batch_buffer = Arc::clone(&buffer);
    let batch_receiver_handle = tokio::spawn(async move {
        run_batch_receiver(batch_rx, batch_buffer).await;
    });

    (batcher_handle, batch_receiver_handle)
}

/// Shared state for tracking acknowledgments (from parent/runner.rs)
struct AcknowledgmentState {
    pending_acks: HashMap<String, Vec<u64>>,
    collector_checkpoints: HashMap<String, CollectorSequencerCheckpoint>,
}

/// Source state for collector status (from collector/runner.rs)
struct CollectorSourceState {
    watermark: Option<DateTime<Utc>>,
    active: bool,
}

/// Intermediate state for collector serving setup
struct CollectorServingState {
    api_state: Arc<CollectorState>,
    batcher: Arc<Mutex<EpochBatcher>>,
    buffer: Arc<Mutex<BatchBuffer>>,
    global_watermark: Arc<RwLock<Option<DateTime<Utc>>>>,
    source_states: Arc<RwLock<HashMap<String, Arc<Mutex<CollectorSourceState>>>>>,
}

/// Epoch batcher task (adapted from collector/runner.rs)
async fn run_epoch_batcher_task(
    mut seq_rx: mpsc::Receiver<LogRecord>,
    batch_tx: mpsc::Sender<LogBatch>,
    batcher: Arc<Mutex<EpochBatcher>>,
    global_watermark: Arc<RwLock<Option<DateTime<Utc>>>>,
    source_states: Arc<RwLock<HashMap<String, Arc<Mutex<CollectorSourceState>>>>>,
) {
    while let Some(record) = seq_rx.recv().await {
        // Update source watermark
        if let Some(source_state) = source_states.read().await.get(&record.source_id) {
            if let Ok(mut state) = source_state.lock() {
                state.watermark = Some(record.timestamp);
            }
        }

        // Compute minimum watermark
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

        if let Some(batch) = completed_batch {
            if batch_tx.send(batch).await.is_err() {
                error!("Batch receiver channel closed");
                break;
            }
        }
    }

    // Flush current epoch
    info!("Sequencer complete, flushing current epoch");
    let watermark = global_watermark.read().await.unwrap_or_else(Utc::now);
    let maybe_batch = batcher.lock().unwrap().flush_current(watermark);
    if let Some(batch) = maybe_batch {
        let _ = batch_tx.send(batch).await;
    }
}

/// Batch receiver task (from collector/runner.rs)
async fn run_batch_receiver(
    mut batch_rx: mpsc::Receiver<LogBatch>,
    buffer: Arc<Mutex<BatchBuffer>>,
) {
    use crate::collector::batch_buffer::BufferError;

    while let Some(batch) = batch_rx.recv().await {
        loop {
            let result = buffer.lock().unwrap().push(batch.clone());
            match result {
                Ok(()) => break,
                Err(BufferError::BufferFull) => {
                    warn!("Buffer full, applying backpressure");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

/// Compaction task (from collector/runner.rs)
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

/// Stats updater task (from collector/runner.rs)
async fn run_stats_updater(
    state: Arc<CollectorState>,
    buffer: Arc<Mutex<BatchBuffer>>,
    source_states: Arc<RwLock<HashMap<String, Arc<Mutex<CollectorSourceState>>>>>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;

        let stats = buffer.lock().unwrap().stats();
        *state.buffer_stats.write().await = stats;

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

/// Flush pending acknowledgments to remote collectors
async fn flush_pending_acks(
    collectors: &[crate::config::types::CollectorEndpoint],
    ack_state: Arc<RwLock<AcknowledgmentState>>,
) {
    let acks_to_send = {
        let mut state = ack_state.write().await;
        std::mem::take(&mut state.pending_acks)
    };

    if acks_to_send.is_empty() {
        debug!("No pending acknowledgments to flush");
        return;
    }

    for (collector_id, seq_nums) in acks_to_send {
        if seq_nums.is_empty() {
            continue;
        }
        debug!(
            collector_id = %collector_id,
            count = seq_nums.len(),
            max_sequence = seq_nums.iter().max().copied().unwrap_or(0),
            "Flushing pending acknowledgments"
        );

        let endpoint = collectors.iter().find(|e| e.id == collector_id);
        if endpoint.is_none() {
            warn!(collector_id = %collector_id, "Collector endpoint not found for acknowledgment");
            continue;
        }
        let endpoint = endpoint.unwrap();

        match CollectorClient::new(endpoint) {
            Ok(client) => {
                info!(
                    collector_id = %collector_id,
                    count = seq_nums.len(),
                    "Sending acknowledgments"
                );

                match client.acknowledge(seq_nums.clone()).await {
                    Ok(_) => {
                        if let Some(max_seq) = seq_nums.iter().max() {
                            let mut state = ack_state.write().await;
                            if let Some(cp) = state.collector_checkpoints.get_mut(&collector_id) {
                                cp.last_acknowledged_sequence = *max_seq;
                            }
                        }
                        debug!(
                            collector_id = %collector_id,
                            last_ack = seq_nums.iter().max().copied().unwrap_or(0),
                            "Acknowledgment flush succeeded"
                        );
                    }
                    Err(e) => {
                        error!(
                            collector_id = %collector_id,
                            error = %e,
                            "Failed to send acknowledgments"
                        );
                        let mut state = ack_state.write().await;
                        state
                            .pending_acks
                            .entry(collector_id.clone())
                            .or_insert_with(Vec::new)
                            .extend(seq_nums);
                    }
                }
            }
            Err(e) => {
                error!(
                    collector_id = %collector_id,
                    error = %e,
                    "Failed to create collector client for acknowledgment"
                );
                let mut state = ack_state.write().await;
                state
                    .pending_acks
                    .entry(collector_id.clone())
                    .or_insert_with(Vec::new)
                    .extend(seq_nums);
            }
        }
    }
}

/// Parent checkpoint task
async fn run_parent_checkpoint_task(
    storage: Arc<dyn Storage>,
    ack_state: Arc<RwLock<AcknowledgmentState>>,
    config_version: u64,
    fiber_processor: Option<Arc<RwLock<FiberProcessor>>>,
) {
    let mut manager = CheckpointManager::new(storage, Duration::from_secs(30));
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        if !manager.should_save() {
            continue;
        }

        let checkpoint = {
            let state = ack_state.read().await;
            let fiber_processors = if let Some(ref fp) = fiber_processor {
                fp.read().await.create_checkpoint()
            } else {
                HashMap::new()
            };

            ParentCheckpoint {
                version: 1,
                timestamp: Utc::now(),
                config_version,
                collectors: state.collector_checkpoints.clone(),
                sequencer: SequencerCheckpoint {
                    watermarks: HashMap::new(),
                },
                fiber_processors,
            }
        };

        if let Err(e) = manager.save_parent(&checkpoint).await {
            error!(error = %e, "Failed to save parent checkpoint");
        } else {
            info!(
                collectors = checkpoint.collectors.len(),
                "Saved parent checkpoint"
            );
        }
    }
}

/// Save a parent checkpoint once (for shutdown)
async fn save_parent_checkpoint_once(
    storage: Arc<dyn Storage>,
    ack_state: Arc<RwLock<AcknowledgmentState>>,
    config_version: u64,
    fiber_processor: Option<Arc<RwLock<FiberProcessor>>>,
) -> Result<(), RunError> {
    let checkpoint = {
        let state = ack_state.read().await;
        let fiber_processors = if let Some(ref fp) = fiber_processor {
            fp.read().await.create_checkpoint()
        } else {
            HashMap::new()
        };

        ParentCheckpoint {
            version: 1,
            timestamp: Utc::now(),
            config_version,
            collectors: state.collector_checkpoints.clone(),
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::new(),
            },
            fiber_processors,
        }
    };

    let mut manager = CheckpointManager::new(storage, Duration::from_secs(0));
    manager
        .save_parent(&checkpoint)
        .await
        .map_err(|e| RunError::Storage(crate::storage::traits::StorageError::Database(e.to_string())))?;
    info!(
        collectors = checkpoint.collectors.len(),
        "Saved final parent checkpoint"
    );
    Ok(())
}

/// Collector checkpoint task (adapted from collector/runner.rs)
async fn run_collector_checkpoint_task(
    mut manager: CheckpointManager,
    batcher: Arc<Mutex<EpochBatcher>>,
    buffer: Arc<Mutex<BatchBuffer>>,
    source_checkpoint_states: Arc<RwLock<HashMap<String, SharedSourceState>>>,
    collector_id: String,
    config_version: u64,
) {
    use crate::storage::checkpoint::{
        BatchBufferCheckpoint, CollectorCheckpoint, EpochBatcherCheckpoint,
    };

    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        if !manager.should_save() {
            continue;
        }

        let sources = {
            let states = source_checkpoint_states.read().await;
            let mut sources = HashMap::new();
            for (source_id, shared_state) in states.iter() {
                if let Ok(state) = shared_state.lock() {
                    sources.insert(
                        source_id.clone(),
                        SourceCheckpoint {
                            path: PathBuf::new(),
                            offset: state.offset,
                            inode: state.inode,
                            last_timestamp: state.last_timestamp,
                        },
                    );
                }
            }
            sources
        };

        let checkpoint = {
            let batcher_lock = batcher.lock().unwrap();
            let buffer_lock = buffer.lock().unwrap();
            let buffer_stats = buffer_lock.stats();

            CollectorCheckpoint {
                version: 1,
                timestamp: Utc::now(),
                config_version,
                collector_id: collector_id.clone(),
                sources,
                sequencer: SequencerCheckpoint {
                    watermarks: HashMap::new(),
                },
                epoch_batcher: EpochBatcherCheckpoint {
                    sequence_counter: batcher_lock.sequence_counter(),
                    rewind_generation: batcher_lock.rewind_generation(),
                    current_epoch: None,
                },
                batch_buffer: BatchBufferCheckpoint {
                    oldest_sequence: buffer_stats.oldest_sequence,
                    newest_sequence: buffer_stats.newest_sequence,
                    unacknowledged_count: buffer_stats.current_epochs,
                },
            }
        };

        if let Err(e) = manager.save_collector(&checkpoint).await {
            error!(error = %e, "Failed to save collector checkpoint");
        } else {
            info!(
                sequence = checkpoint.epoch_batcher.sequence_counter,
                "Saved collector checkpoint"
            );
        }
    }
}
