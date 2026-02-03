use crate::config::parse::{load_config, load_config_with_yaml};
use crate::config::reconcile::{reconcile_config_on_startup, ReconcileResult};
use crate::config::types::OperationMode;
#[allow(deprecated)]
use crate::config::version::compute_config_version;
use crate::fiber::FiberProcessor;
use crate::pipeline::{create_channel, run_processor, run_writer, FiberUpdate};
use crate::reprocessing::ReprocessState;
use crate::sequencer::merge::{run_sequencer, SequencerRunConfig};
use crate::source::reader::{LogRecord, SourceReader};
use crate::storage::checkpoint::{Checkpoint, CheckpointManager, SequencerCheckpoint, SharedSourceState, SharedFiberProcessorState, SourceCheckpoint};
use crate::storage::duckdb::DuckDbStorage;
use crate::storage::traits::Storage;
use crate::web::run_server;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::signal;
use tokio::sync::{mpsc, watch, oneshot, RwLock};
use tracing::{error, info, warn};

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

    // Load config to check mode
    let temp_config = load_config(config_path)?;

    // Dispatch based on operation mode
    match temp_config.mode {
        crate::config::types::OperationMode::Standalone => {
            run_standalone_mode(config_path).await
        }
        crate::config::types::OperationMode::Collector => {
            run_collector_mode(config_path).await
        }
        crate::config::types::OperationMode::Parent => {
            run_parent_mode(config_path).await
        }
    }
}

async fn run_collector_mode(config_path: &PathBuf) -> Result<(), RunError> {
    info!("Starting in collector mode");

    // Load config to get storage path
    let temp_config = load_config(config_path)?;

    // Create storage for checkpoint persistence
    let storage_path = &temp_config.storage.path;
    let storage = Arc::new(
        DuckDbStorage::new(storage_path)?
    ) as Arc<dyn Storage>;

    // Initialize storage schema and reconcile config
    storage.init_schema().await?;
    reconcile_and_log(config_path, storage.as_ref()).await?;

    // Load config (may have been updated by reconciliation)
    let (config, _config_yaml) = load_config_with_yaml(config_path)?;

    // Compute config version
    #[allow(deprecated)]
    let config_version = compute_config_version(config_path)
        .map_err(|e| crate::config::parse::ConfigError::Io(e))?;

    info!(config_version = config_version, "Computed config version");

    // Create and run collector
    let runner = crate::collector::runner::CollectorRunner::new(config, config_version)
        .map_err(|e| RunError::Config(crate::config::parse::ConfigError::Validation(e.to_string())))?;

    runner.run(storage).await
        .map_err(|e| RunError::Config(crate::config::parse::ConfigError::Validation(e.to_string())))?;

    Ok(())
}

async fn run_parent_mode(config_path: &PathBuf) -> Result<(), RunError> {
    info!("Starting in parent mode");

    // Load config to get storage path
    let temp_config = load_config(config_path)?;

    // Initialize storage and reconcile config
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

    // In parent mode, add auto-generated source fiber types for sources from collectors
    // Query database for all source IDs that have sent logs
    if config.auto_source_fibers || config.mode == OperationMode::Parent {
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

    // Create fiber processor - shared between ParentRunner and web server
    let fiber_processor = FiberProcessor::from_config(&config, config_version)?;
    let shared_processor = Arc::new(RwLock::new(fiber_processor));

    // Create shared state for web server and hot-reload support
    let shared_config = Arc::new(RwLock::new(config.clone()));
    let shared_config_yaml = Arc::new(RwLock::new(config_yaml));
    let shared_version = Arc::new(RwLock::new(config_version));
    let shared_reprocess_state: Arc<RwLock<Option<ReprocessState>>> = Arc::new(RwLock::new(None));

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start web server task
    info!("Starting web server on {}", config.web.listen);
    let web_storage = storage.clone();
    let web_config = config.web.clone();
    let web_shutdown_rx = shutdown_rx.clone();
    let web_processor = Arc::clone(&shared_processor);
    let web_shared_config = Arc::clone(&shared_config);
    let web_shared_config_yaml = Arc::clone(&shared_config_yaml);
    let web_version = Arc::clone(&shared_version);
    let web_reprocess_state = Arc::clone(&shared_reprocess_state);
    let web_config_path = config_path.clone();
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
        )
        .await
        .map_err(|e| RunError::WebServer(e.to_string()))
    });

    // Create and run parent in a separate task
    let runner = crate::parent::runner::ParentRunner::new(config.clone(), config_version)
        .map_err(|e| RunError::Config(crate::config::parse::ConfigError::Validation(e.to_string())))?;

    let parent_storage = storage.clone();
    let parent_processor = Arc::clone(&shared_processor);
    let parent_shutdown_rx = shutdown_rx.clone();
    let mut parent_handle = tokio::spawn(async move {
        runner
            .run(parent_storage, Some(parent_processor), Some(parent_shutdown_rx))
            .await
            .map_err(|e| RunError::Config(crate::config::parse::ConfigError::Validation(e.to_string())))
    });

    let web_url = format_web_url(&config.web.listen);
    info!("Parent mode started, press Ctrl+C to shutdown");

    // Wait for shutdown signal or parent completion
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received");
            let _ = shutdown_tx.send(true);
        }
        result = &mut parent_handle => {
            match result {
                Ok(Ok(())) => {
                    info!("Parent runner completed successfully. Web server continues running at {}. Press Ctrl+C to shutdown.", web_url);
                    // Wait for Ctrl+C
                    match signal::ctrl_c().await {
                        Ok(()) => {
                            info!("Shutdown signal received");
                            let _ = shutdown_tx.send(true);
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to listen for shutdown signal");
                            let _ = shutdown_tx.send(true);
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!(error = %e, "Parent runner error, shutting down");
                    let _ = shutdown_tx.send(true);
                }
                Err(e) => {
                    error!(error = %e, "Parent runner join error, shutting down");
                    let _ = shutdown_tx.send(true);
                }
            }
        }
    }

    // Wait for parent runner to finish drain/shutdown
    match tokio::time::timeout(std::time::Duration::from_secs(10), &mut parent_handle).await {
        Ok(Ok(Ok(()))) => info!("Parent runner stopped gracefully"),
        Ok(Ok(Err(e))) => error!(error = %e, "Parent runner error during shutdown"),
        Ok(Err(e)) => error!(error = %e, "Parent runner join error during shutdown"),
        Err(_) => warn!("Parent runner shutdown timed out after 10 seconds"),
    }

    // Wait for web server with graceful shutdown timeout
    match tokio::time::timeout(std::time::Duration::from_secs(5), web_handle).await {
        Ok(Ok(Ok(()))) => info!("Web server stopped gracefully"),
        Ok(Ok(Err(e))) => error!(error = %e, "Web server error"),
        Ok(Err(e)) => error!(error = %e, "Web server join error"),
        Err(_) => {
            warn!("Web server shutdown timed out after 5 seconds");
        }
    }

    info!("Parent mode shutdown complete");

    Ok(())
}

async fn run_standalone_mode(config_path: &PathBuf) -> Result<(), RunError> {
    info!("Starting in standalone mode");

    // Initialize storage and reconcile config
    let temp_config = load_config(config_path)?;
    let storage_path = &temp_config.storage.path;
    info!(path = %storage_path.display(), "Initializing storage");
    let storage = Arc::new(DuckDbStorage::new(storage_path)?);
    storage.init_schema().await?;
    reconcile_and_log(config_path, storage.as_ref()).await?;

    // Load config (may have been updated by reconciliation)
    let (config, config_yaml) = load_config_with_yaml(config_path)?;

    // Compute config version from content hash
    #[allow(deprecated)]
    let config_version = compute_config_version(config_path)
        .map_err(|e| crate::config::parse::ConfigError::Io(e))?;

    info!(config_version = config_version, "Computed config version");

    // Storage already initialized for reconciliation above

    // Load checkpoint if enabled
    let checkpoint = if config.pipeline.checkpoint.enabled {
        use crate::storage::checkpoint::CheckpointManager;
        let checkpoint_mgr = CheckpointManager::new(
            storage.clone(),
            std::time::Duration::from_secs(config.pipeline.checkpoint.interval_seconds),
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

    // Create source readers (with checkpoint restoration if available)
    let mut readers = Vec::new();
    let mut shared_states: HashMap<String, SharedSourceState> = HashMap::new();

    for (source_id, source_config) in &config.sources {
        info!(source_id = %source_id, path = %source_config.path.display(), "Creating source reader");

        let reader = if let Some(ref checkpoint) = checkpoint {
            if let Some(source_checkpoint) = checkpoint.sources.get(source_id) {
                // Check if file inode matches (detect file rotation)
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

        // Attach shared state for checkpointing
        let (reader, state) = reader.with_shared_state();
        shared_states.insert(source_id.clone(), state);
        readers.push(reader);
    }

    if readers.is_empty() {
        warn!("No sources configured, pipeline will not process any logs");
    }

    // Create fiber processor (with checkpoint restoration if available)
    info!("Creating fiber processor");
    let mut fiber_processor = FiberProcessor::from_config(&config, config_version)?;
    if let Some(ref checkpoint) = checkpoint {
        info!("Restoring fiber processor state from checkpoint");
        fiber_processor.restore_from_checkpoint(&checkpoint.fiber_processors);

        // Close orphaned fibers in storage that aren't in the checkpoint
        // This prevents duplicates when fibers were written to storage but didn't make it into
        // the checkpoint before a crash
        let checkpointed_fiber_ids: std::collections::HashSet<uuid::Uuid> = checkpoint
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
                warn!(
                    closed_count = count,
                    "Closed orphaned fibers that were in storage but not in checkpoint"
                );
            }
            Ok(_) => {
                info!("No orphaned fibers found");
            }
            Err(e) => {
                error!(error = %e, "Failed to close orphaned fibers");
            }
        }
    }
    info!(
        fiber_types = config.fiber_types.len(),
        open_fibers = fiber_processor.total_open_fibers(),
        "Fiber processor initialized"
    );

    // Create shared state for fiber processor checkpointing (if enabled)
    let shared_fiber_state: Option<SharedFiberProcessorState> = if config.pipeline.checkpoint.enabled {
        let initial_state = fiber_processor.create_checkpoint();
        Some(Arc::new(std::sync::Mutex::new(initial_state)))
    } else {
        None
    };

    // Wrap fiber processor and config in shared state for hot-reload support
    let shared_processor = Arc::new(RwLock::new(fiber_processor));
    let shared_config = Arc::new(RwLock::new(config.clone()));
    let shared_config_yaml = Arc::new(RwLock::new(config_yaml));
    let shared_version = Arc::new(RwLock::new(config_version));
    let shared_reprocess_state: Arc<RwLock<Option<ReprocessState>>> = Arc::new(RwLock::new(None));

    // Create channels
    let buffer_size = config.pipeline.backpressure.buffer_limit;
    let (seq_tx, seq_rx) = create_channel::<LogRecord>(buffer_size);
    let (fiber_tx, fiber_rx) = create_channel::<FiberUpdate>(buffer_size);

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Create channel for triggering immediate checkpoint saves
    // This allows critical events (shutdown, sequencer completion) to trigger
    // checkpoint saves immediately rather than waiting for the next timer tick.
    // This prevents state loss and duplicate fibers on restart.
    let (checkpoint_save_tx, mut checkpoint_save_rx) = mpsc::channel::<()>(10);

    // Start periodic checkpoint saving task if enabled
    let checkpoint_handle = if config.pipeline.checkpoint.enabled {
        let checkpoint_storage = storage.clone();
        let interval = config.pipeline.checkpoint.interval_seconds;
        let states = shared_states.clone();
        let source_configs = config.sources.clone();
        let fiber_state = shared_fiber_state.clone();
        let mut shutdown_watch = shutdown_rx.clone();

        info!(
            interval_seconds = interval,
            "Starting checkpoint task"
        );

        Some(tokio::spawn(async move {
            let mut checkpoint_mgr = CheckpointManager::new(
                checkpoint_storage,
                std::time::Duration::from_secs(interval),
            );
            let mut interval_ticker = tokio::time::interval(std::time::Duration::from_secs(interval));

            // Macro to build and save checkpoint
            macro_rules! save_checkpoint {
                () => {{
                    // Build checkpoint from current state
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

                    // Capture fiber processor state
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
                        tracing::debug!("Checkpoint saved successfully");
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

    // Start sequencer
    info!("Starting sequencer");
    let sequencer_config = SequencerRunConfig::from(&config.sequencer);
    let sequencer_handle = if !readers.is_empty() {
        Some(run_sequencer(readers, seq_tx, sequencer_config).await?)
    } else {
        drop(seq_tx); // No sources, close the channel immediately
        None
    };

    // Start fiber processor task
    info!("Starting fiber processor task");
    let processor_storage = storage.clone();
    let processor_config = config.clone();
    let processor_shared_state = shared_fiber_state.clone();
    let processor_lock = Arc::clone(&shared_processor);
    let version_lock = Arc::clone(&shared_version);
    let mut processor_handle = Some(tokio::spawn(async move {
        run_processor(seq_rx, fiber_tx, processor_lock, version_lock, processor_storage, &processor_config, processor_shared_state).await
    }));

    // Start storage writer task
    info!("Starting storage writer task");
    let writer_storage = storage.clone();
    let writer_config = config.storage.clone();
    let writer_handle = tokio::spawn(async move {
        run_writer(fiber_rx, writer_storage, &writer_config).await
    });

    // Start web server task
    info!("Starting web server on {}", config.web.listen);
    let web_storage = storage.clone();
    let web_config = config.web.clone();
    let web_shutdown_rx = shutdown_rx.clone();
    let web_processor = Arc::clone(&shared_processor);
    let web_shared_config = Arc::clone(&shared_config);
    let web_shared_config_yaml = Arc::clone(&shared_config_yaml);
    let web_version = Arc::clone(&shared_version);
    let web_reprocess_state = Arc::clone(&shared_reprocess_state);
    let web_config_path = config_path.clone();
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
        )
        .await
        .map_err(|e| RunError::WebServer(e.to_string()))
    });

    let web_url = format_web_url(&config.web.listen);
    info!("Pipeline started, press Ctrl+C to shutdown");

    // Create channel to signal abort to sequencer wait task
    let (abort_tx, abort_rx) = oneshot::channel::<()>();

    // Spawn task to wait for sequencer completion or abort signal
    let sequencer_wait_task = if let Some(mut handle) = sequencer_handle {
        Some(tokio::spawn(async move {
            tokio::select! {
                result = handle.wait() => {
                    // Sequencer completed naturally
                    Ok(result)
                }
                _ = abort_rx => {
                    // Abort signal received
                    info!("Aborting sequencer and source readers");
                    handle.abort();
                    Err(()) // Aborted
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

            // Trigger immediate checkpoint save before shutdown
            if config.pipeline.checkpoint.enabled {
                info!("Triggering checkpoint save before shutdown");
                let _ = checkpoint_save_tx.send(()).await;
                // Give checkpoint task time to save
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            let _ = shutdown_tx.send(true);

            // Signal the sequencer wait task to abort
            let _ = abort_tx.send(());
        }
        result = async {
            if let Some(task) = sequencer_wait_task {
                task.await
            } else {
                // No sequencer to wait for, sleep forever
                std::future::pending::<Result<Result<Result<(), crate::sequencer::merge::SequencerError>, ()>, tokio::task::JoinError>>().await
            }
        } => {
            match result {
                Ok(Ok(Ok(()))) => {
                    info!("Log ingestion complete. Waiting for fiber processor to finish.");

                    // Wait for fiber processor to complete before saving checkpoint.
                    // This ensures the checkpoint reflects the final state after all logs are processed.
                    if let Some(handle) = processor_handle.take() {
                        match handle.await {
                            Ok(Ok(())) => info!("Fiber processor completed successfully"),
                            Ok(Err(e)) => error!(error = %e, "Fiber processor error"),
                            Err(e) => error!(error = %e, "Fiber processor join error"),
                        }
                    }

                    // Now trigger checkpoint save - fiber processor has updated shared state
                    if config.pipeline.checkpoint.enabled {
                        info!("Triggering checkpoint save after fiber processor completion");
                        let _ = checkpoint_save_tx.send(()).await;
                        // Give checkpoint task time to save
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }

                    info!("Web server continues running at {}. Press Ctrl+C to shutdown.", web_url);
                    // Don't send shutdown signal - let web server continue running
                    // Wait for Ctrl+C
                    match signal::ctrl_c().await {
                        Ok(()) => {
                            info!("Shutdown signal received");

                            // Trigger checkpoint save before final shutdown
                            if config.pipeline.checkpoint.enabled {
                                info!("Triggering final checkpoint save");
                                let _ = checkpoint_save_tx.send(()).await;
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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

    // Wait for tasks to complete
    info!("Waiting for pipeline tasks to complete");

    // Wait for checkpoint task
    if let Some(handle) = checkpoint_handle {
        match handle.await {
            Ok(()) => info!("Checkpoint task completed successfully"),
            Err(e) => error!(error = %e, "Checkpoint task join error"),
        }
    }

    // Wait for processor (if not already awaited during sequencer completion)
    if let Some(handle) = processor_handle {
        match handle.await {
            Ok(Ok(())) => info!("Processor task completed successfully"),
            Ok(Err(e)) => error!(error = %e, "Processor task error"),
            Err(e) => error!(error = %e, "Processor task join error"),
        }
    }

    // Wait for writer
    match writer_handle.await {
        Ok(Ok(())) => info!("Writer task completed successfully"),
        Ok(Err(e)) => error!(error = %e, "Writer task error"),
        Err(e) => error!(error = %e, "Writer task join error"),
    }

    // Wait for web server with graceful shutdown timeout
    match tokio::time::timeout(std::time::Duration::from_secs(5), web_handle).await {
        Ok(Ok(Ok(()))) => info!("Web server stopped gracefully"),
        Ok(Ok(Err(e))) => error!(error = %e, "Web server error"),
        Ok(Err(e)) => error!(error = %e, "Web server join error"),
        Err(_) => {
            warn!("Web server shutdown timed out after 5 seconds");
            // Handle was consumed by timeout, task will be dropped
        }
    }

    info!("Pipeline shutdown complete");

    Ok(())
}
