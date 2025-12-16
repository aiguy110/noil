use crate::config::parse::load_config;
use crate::config::compute_config_version;
use crate::fiber::FiberProcessor;
use crate::pipeline::{create_channel, run_processor, run_writer, FiberUpdate};
use crate::sequencer::merge::{run_sequencer, SequencerRunConfig};
use crate::source::reader::{LogRecord, SourceReader};
use crate::storage::duckdb::DuckDbStorage;
use crate::storage::traits::Storage;
use crate::web::run_server;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::signal;
use tokio::sync::watch;
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

    // Load and validate config
    let config = load_config(config_path)?;

    // Compute config version from content hash
    let config_version = compute_config_version(config_path)
        .map_err(|e| crate::config::parse::ConfigError::Io(e))?;

    info!(config_version = config_version, "Computed config version");

    // Load checkpoint if enabled
    let checkpoint = if config.pipeline.checkpoint.enabled {
        use crate::storage::checkpoint::CheckpointManager;
        let checkpoint_mgr = CheckpointManager::new(
            &config.pipeline.checkpoint.path,
            std::time::Duration::from_secs(config.pipeline.checkpoint.interval_seconds),
        );
        match checkpoint_mgr.load() {
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

    // Initialize storage
    info!(path = %config.storage.path.display(), "Initializing storage");
    let storage = Arc::new(DuckDbStorage::new(&config.storage.path)?);
    storage.init_schema().await?;

    // Create source readers (with checkpoint restoration if available)
    let mut readers = Vec::new();
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
    }
    info!(
        fiber_types = config.fiber_types.len(),
        open_fibers = fiber_processor.total_open_fibers(),
        "Fiber processor initialized"
    );

    // Create channels
    let buffer_size = config.pipeline.backpressure.buffer_limit;
    let (seq_tx, seq_rx) = create_channel::<LogRecord>(buffer_size);
    let (fiber_tx, fiber_rx) = create_channel::<FiberUpdate>(buffer_size);

    // Create shutdown signal
    let (shutdown_tx, _shutdown_rx) = watch::channel(false);

    // Note: For periodic checkpoint saving, we would need to capture source state here
    // This is deferred to a future iteration when we refactor to use shared state

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
    let processor_handle = tokio::spawn(async move {
        run_processor(seq_rx, fiber_tx, fiber_processor, processor_storage, &processor_config, config_version).await
    });

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
    let web_handle = tokio::spawn(async move {
        run_server(web_storage, web_config)
            .await
            .map_err(|e| RunError::WebServer(e.to_string()))
    });

    info!("Pipeline started, press Ctrl+C to shutdown");

    // Wait for shutdown signal or task completion
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received");
            let _ = shutdown_tx.send(true);
        }
        result = async {
            if let Some(handle) = sequencer_handle {
                handle.wait().await
            } else {
                Ok(())
            }
        } => {
            match result {
                Ok(()) => info!("Sequencer completed"),
                Err(e) => error!(error = %e, "Sequencer error"),
            }
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)) => {
            // This won't complete normally, just keeps select! alive
        }
    }

    // Wait for tasks to complete
    info!("Waiting for pipeline tasks to complete");

    // Wait for processor
    match processor_handle.await {
        Ok(Ok(())) => info!("Processor task completed successfully"),
        Ok(Err(e)) => error!(error = %e, "Processor task error"),
        Err(e) => error!(error = %e, "Processor task join error"),
    }

    // Wait for writer
    match writer_handle.await {
        Ok(Ok(())) => info!("Writer task completed successfully"),
        Ok(Err(e)) => error!(error = %e, "Writer task error"),
        Err(e) => error!(error = %e, "Writer task join error"),
    }

    // Note: web server doesn't gracefully shutdown yet, so we just abort it
    web_handle.abort();
    info!("Web server stopped");

    info!("Pipeline shutdown complete");

    Ok(())
}
