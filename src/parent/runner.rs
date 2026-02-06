use crate::config::types::{CollectorEndpoint, Config, RemoteCollectorsConfig};
use crate::fiber::FiberProcessor;
use crate::parent::collector_client::CollectorClient;
use crate::parent::collector_stream::CollectorStream;
use crate::pipeline::{run_processor, run_writer, FiberUpdate};
use crate::source::reader::LogRecord;
use crate::storage::checkpoint::{
    CheckpointManager, CollectorSequencerCheckpoint, ParentCheckpoint, SequencerCheckpoint,
};
use crate::storage::traits::Storage;
use chrono::Utc;
use std::collections::HashMap;
use std::future::pending;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{mpsc, watch, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

#[derive(Debug, Error)]
pub enum ParentError {
    #[error("config error: {0}")]
    Config(String),

    #[error("collector client error: {0}")]
    CollectorClient(#[from] crate::parent::collector_client::CollectorClientError),

    #[error("collector stream error: {0}")]
    CollectorStream(#[from] crate::parent::collector_stream::CollectorStreamError),

    #[error("sequencer error: {0}")]
    Sequencer(String),

    #[error("pipeline error: {0}")]
    Pipeline(#[from] crate::pipeline::runner::PipelineError),

    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("channel send error")]
    ChannelSend,

    #[error("checkpoint error: {0}")]
    Checkpoint(#[from] crate::storage::checkpoint::CheckpointError),
}

/// Shared state for tracking acknowledgments
struct AcknowledgmentState {
    /// Pending acknowledgments per collector (collector_id -> Vec<sequence_nums>)
    pending_acks: HashMap<String, Vec<u64>>,

    /// Collector checkpoint state (last sequence, last ack, watermark)
    collector_checkpoints: HashMap<String, CollectorSequencerCheckpoint>,
}

pub struct ParentRunner {
    config: Config,
    remote_collectors_config: RemoteCollectorsConfig,
    config_version: u64,
}

impl ParentRunner {
    pub fn new(config: Config, config_version: u64) -> Result<Self, ParentError> {
        let remote_collectors_config = config
            .remote_collectors
            .as_ref()
            .ok_or_else(|| ParentError::Config("remote_collectors config section missing".to_string()))?
            .clone();

        Ok(Self {
            config,
            remote_collectors_config,
            config_version,
        })
    }

    pub async fn run(
        self,
        storage: Arc<dyn Storage>,
        fiber_processor: Option<Arc<RwLock<FiberProcessor>>>,
        shutdown_rx: Option<watch::Receiver<bool>>,
    ) -> Result<(), ParentError> {
        info!("Starting parent mode");

        // Initialize storage schema
        storage
            .init_schema()
            .await
            .map_err(|e| ParentError::Config(format!("Failed to initialize storage: {}", e)))?;

        // Create checkpoint manager
        let checkpoint_interval = Duration::from_secs(30); // TODO: make configurable
        let checkpoint_manager = CheckpointManager::new(storage.clone(), checkpoint_interval);

        // Try to load checkpoint
        let checkpoint_opt = checkpoint_manager
            .load_parent()
            .await
            .map_err(|e| ParentError::Config(format!("Failed to load checkpoint: {}", e)))?;

        // Track last acknowledged sequence per collector
        let mut collector_checkpoints: HashMap<String, CollectorSequencerCheckpoint> = checkpoint_opt
            .as_ref()
            .map(|cp| cp.collectors.clone())
            .unwrap_or_default();

        if let Some(checkpoint) = &checkpoint_opt {
            info!(
                "Loaded parent checkpoint with {} collectors",
                checkpoint.collectors.len()
            );
            for (collector_id, cp) in &checkpoint.collectors {
                info!(
                    collector_id = %collector_id,
                    last_sequence = cp.last_sequence,
                    last_ack = cp.last_acknowledged_sequence,
                    "Restored collector checkpoint"
                );
            }
        }

        // Create collector clients and streams
        let mut collector_streams = Vec::new();

        for endpoint in &self.remote_collectors_config.endpoints {
            info!(
                collector_id = %endpoint.id,
                url = %endpoint.url,
                "Creating collector client"
            );

            // Get last acknowledged sequence from checkpoint (if available)
            let last_ack_seq = collector_checkpoints
                .get(&endpoint.id)
                .map(|cp| cp.last_acknowledged_sequence)
                .unwrap_or(0);
            let has_checkpoint_for_collector = collector_checkpoints.contains_key(&endpoint.id);

            // Initialize collector checkpoint if not in map
            collector_checkpoints
                .entry(endpoint.id.clone())
                .or_insert_with(|| CollectorSequencerCheckpoint {
                    collector_id: endpoint.id.clone(),
                    last_sequence: last_ack_seq,
                    last_acknowledged_sequence: last_ack_seq,
                    watermark: None,
                });

            let client = CollectorClient::new(endpoint)?;
            let mut stream = CollectorStream::new(client);

            // Resume from last acknowledged sequence when we have a checkpoint for this collector.
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

        if collector_streams.is_empty() {
            return Err(ParentError::Config("no collectors configured".to_string()));
        }

        // Create shared state for tracking acknowledgments
        let ack_state = Arc::new(RwLock::new(AcknowledgmentState {
            pending_acks: HashMap::new(),
            collector_checkpoints: collector_checkpoints.clone(),
        }));

        // Create channels for pipeline
        let (sequencer_tx, sequencer_rx) = mpsc::channel::<LogRecord>(1000);
        let (processor_tx, processor_rx) = mpsc::channel::<FiberUpdate>(1000);

        // Use provided fiber processor or create a new one
        let fiber_processor = if let Some(processor) = fiber_processor {
            processor
        } else {
            Arc::new(RwLock::new(
                FiberProcessor::from_config(&self.config, self.config_version)
                    .map_err(|e| ParentError::Config(format!("Failed to create fiber processor: {}", e)))?,
            ))
        };

        // Restore fiber processor checkpoints (parent mode resume)
        if let Some(checkpoint) = &checkpoint_opt {
            if !checkpoint.fiber_processors.is_empty() {
                let mut processor_guard = fiber_processor.write().await;
                for (fiber_type, fiber_checkpoint) in &checkpoint.fiber_processors {
                    if let Some(typed_processor) = processor_guard.get_processor_mut(fiber_type) {
                        typed_processor.restore_from_checkpoint(fiber_checkpoint);
                        info!(
                            fiber_type = %fiber_type,
                            open_fibers = fiber_checkpoint.open_fibers.len(),
                            "Restored fiber processor checkpoint"
                        );
                    } else {
                        warn!(
                            fiber_type = %fiber_type,
                            "Checkpoint contains unknown fiber type; skipping restore"
                        );
                    }
                }
            }
        }

        // Config version for hot-reload (Phase 5)
        let config_version = Arc::new(RwLock::new(self.config_version));

        // Spawn collector polling tasks that feed the sequencer channel
        let collector_tasks = self.spawn_collector_tasks(
            collector_streams,
            sequencer_tx.clone(),
            Arc::clone(&ack_state),
        );

        // Spawn processor task (processes logs and generates fiber updates)
        let mut processor_handle = tokio::spawn({
            let config = self.config.clone();
            let processor = fiber_processor.clone();
            let config_ver = config_version.clone();
            let storage = storage.clone();
            async move {
                run_processor(
                    sequencer_rx,
                    processor_tx,
                    processor,
                    config_ver,
                    storage,
                    &config,
                    None, // No shared state yet (Phase 4: checkpoints)
                )
                .await
            }
        });

        // Spawn storage writer task (writes fiber updates to storage)
        let mut writer_handle = tokio::spawn({
            let storage = storage.clone();
            let storage_config = self.config.storage.clone();
            async move { run_writer(processor_rx, storage, &storage_config).await }
        });

        // Spawn acknowledgment task
        let mut ack_handle = self.spawn_acknowledgment_task(
            storage.clone(),
            Arc::clone(&ack_state),
        );

        // Spawn checkpoint saving task
        let mut checkpoint_handle = tokio::spawn({
            let ack_state = Arc::clone(&ack_state);
            let config_version = self.config_version;
            let fiber_processor = fiber_processor.clone();
            async move {
                run_checkpoint_task(
                    checkpoint_manager,
                    ack_state,
                    config_version,
                    fiber_processor,
                )
                .await;
            }
        });

        // Wait for tasks to complete or handle shutdown
        let mut shutdown_requested = false;
        let shutdown_watch = async {
            if let Some(mut rx) = shutdown_rx {
                loop {
                    if *rx.borrow() {
                        break;
                    }
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            } else {
                pending::<()>().await;
            }
        };

        tokio::select! {
            result = &mut processor_handle => {
                info!("Processor task completed");
                result??;
            }
            result = &mut writer_handle => {
                info!("Writer task completed");
                result??;
            }
            result = &mut ack_handle => {
                info!("Acknowledgment task completed");
                result?;
            }
            result = &mut checkpoint_handle => {
                info!("Checkpoint task completed");
                result?;
            }
            _ = shutdown_watch => {
                info!("Parent runner received shutdown signal");
                shutdown_requested = true;
            }
        }

        if shutdown_requested {
            info!("Draining in-flight logs before shutdown");

            // Stop collector polling and close the sequencer channel.
            for handle in &collector_tasks {
                handle.abort();
            }
            drop(sequencer_tx);

            // Stop periodic tasks while we drain.
            ack_handle.abort();
            checkpoint_handle.abort();

            // Drain processor + writer to flush in-flight logs/memberships.
            if let Err(e) = processor_handle.await {
                error!(error = %e, "Processor task join error during shutdown drain");
            }
            if let Err(e) = writer_handle.await {
                error!(error = %e, "Writer task join error during shutdown drain");
            }

            // Flush acknowledgments after drain to advance checkpoints.
            flush_pending_acks(&self.remote_collectors_config.endpoints, Arc::clone(&ack_state)).await;

            // Save final checkpoint after drain.
            if let Err(e) = save_parent_checkpoint_once(
                storage.clone(),
                Arc::clone(&ack_state),
                self.config_version,
                fiber_processor.clone(),
            )
            .await
            {
                error!(error = %e, "Failed to save final parent checkpoint on shutdown");
            }

            info!("Parent mode shutdown complete");
            return Ok(());
        }

        // Wait for collector tasks to finish
        for handle in collector_tasks {
            handle.await?;
        }

        flush_pending_acks(&self.remote_collectors_config.endpoints, Arc::clone(&ack_state)).await;

        if let Err(e) = save_parent_checkpoint_once(
            storage.clone(),
            Arc::clone(&ack_state),
            self.config_version,
            fiber_processor.clone(),
        )
        .await
        {
            error!(error = %e, "Failed to save final parent checkpoint");
        }

        info!("Parent mode shutdown complete");
        Ok(())
    }

    /// Spawn per-collector polling tasks that feed logs into the sequencer channel
    fn spawn_collector_tasks(
        &self,
        mut streams: Vec<CollectorStream>,
        sequencer_tx: mpsc::Sender<LogRecord>,
        ack_state: Arc<RwLock<AcknowledgmentState>>,
    ) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();

        for mut stream in streams.drain(..) {
            let tx = sequencer_tx.clone();
            let poll_interval = self.remote_collectors_config.poll_interval;
            let ack_state_clone = Arc::clone(&ack_state);

            let handle = tokio::spawn(async move {
                let collector_id = stream.collector_id().to_string();
                info!(collector_id = %collector_id, "Collector polling task started");

                let mut poll_timer = tokio::time::interval(poll_interval);
                let mut last_sequence: Option<u64> = None;

                loop {
                    poll_timer.tick().await;

                    // Fetch at most one batch per interval, then drain queued records quickly.
                    let fetched = match stream.fetch_new_logs().await {
                        Ok(count) => count,
                        Err(e) => {
                            error!(
                                collector_id = %collector_id,
                                error = %e,
                                "Error fetching from collector"
                            );
                            // Continue polling even on error (retry logic in client)
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    };

                    if fetched == 0 {
                        debug!(collector_id = %collector_id, "No records available");
                        continue;
                    }

                    // Track sequence number for acknowledgment once per fetch.
                    let current_sequence = stream.last_sequence();
                    if last_sequence.is_none() || Some(current_sequence) != last_sequence {
                        let mut state = ack_state_clone.write().await;
                        state
                            .pending_acks
                            .entry(collector_id.clone())
                            .or_insert_with(Vec::new)
                            .push(current_sequence);
                        debug!(
                            collector_id = %collector_id,
                            current_sequence = current_sequence,
                            pending_acks = state
                                .pending_acks
                                .get(&collector_id)
                                .map(|v| v.len())
                                .unwrap_or(0),
                            "Queued pending acknowledgment"
                        );

                        if let Some(cp) = state.collector_checkpoints.get_mut(&collector_id) {
                            cp.last_sequence = current_sequence;
                            if let Some((_gen, ts)) = stream.watermark_with_generation() {
                                cp.watermark = Some(ts);
                            }
                        }

                        last_sequence = Some(current_sequence);
                    }

                    while let Some(record) = stream.pop_queued_record() {
                        debug!(
                            collector_id = %collector_id,
                            log_id = %record.id,
                            timestamp = %record.timestamp,
                            "Received log from collector"
                        );

                        if tx.send(record).await.is_err() {
                            warn!(collector_id = %collector_id, "Sequencer channel closed, stopping");
                            break;
                        }
                    }
                }

            });

            handles.push(handle);
        }

        handles
    }

    /// Spawn acknowledgment task
    ///
    /// This task periodically sends acknowledgments to collectors after logs are
    /// successfully written to storage. This is CRITICAL for correctness - we must
    /// only acknowledge after durable storage write completes.
    ///
    /// NOTE: For Phase 4, we implement a simplified version that periodically acknowledges
    /// all seen batches. In production, this should be tied to storage write completion callbacks.
    fn spawn_acknowledgment_task(
        &self,
        _storage: Arc<dyn Storage>,
        ack_state: Arc<RwLock<AcknowledgmentState>>,
    ) -> JoinHandle<()> {
        let collectors = self.remote_collectors_config.endpoints.clone();

        tokio::spawn(async move {
            info!("Acknowledgment task started");

            let mut interval = tokio::time::interval(Duration::from_secs(5));

            loop {
                interval.tick().await;
                flush_pending_acks(&collectors, Arc::clone(&ack_state)).await;
            }
        })
    }
}

/// Periodic checkpoint saving task for parent mode
async fn run_checkpoint_task(
    mut manager: CheckpointManager,
    ack_state: Arc<RwLock<AcknowledgmentState>>,
    config_version: u64,
    fiber_processor: Arc<RwLock<FiberProcessor>>,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        if !manager.should_save() {
            continue;
        }

        // Build checkpoint
        let checkpoint = {
            let state = ack_state.read().await;
            let processor = fiber_processor.read().await;

            ParentCheckpoint {
                version: 1,
                timestamp: Utc::now(),
                config_version,
                collectors: state.collector_checkpoints.clone(),
                sequencer: SequencerCheckpoint {
                    watermarks: HashMap::new(), // TODO: track hierarchical sequencer watermarks
                },
                fiber_processors: processor.create_checkpoint(), // Get fiber processor checkpoint
            }
        };

        // Save checkpoint
        if let Err(e) = manager.save_parent(&checkpoint).await {
            error!(error = %e, "Failed to save parent checkpoint");
        } else {
            info!(
                collectors = checkpoint.collectors.len(),
                "Saved parent checkpoint"
            );
            for (collector_id, cp) in &checkpoint.collectors {
                debug!(
                    collector_id = %collector_id,
                    last_sequence = cp.last_sequence,
                    last_ack = cp.last_acknowledged_sequence,
                    "Checkpoint collector state"
                );
            }
        }
    }
}

async fn flush_pending_acks(
    collectors: &[CollectorEndpoint],
    ack_state: Arc<RwLock<AcknowledgmentState>>,
) {
    // Process pending acknowledgments
    let acks_to_send = {
        let mut state = ack_state.write().await;
        std::mem::take(&mut state.pending_acks)
    };

    if acks_to_send.is_empty() {
        debug!("No pending acknowledgments to flush");
        return;
    }

    // Send acknowledgments to each collector
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

        // Find collector endpoint
        let endpoint = collectors.iter().find(|e| e.id == collector_id);
        if endpoint.is_none() {
            warn!(collector_id = %collector_id, "Collector endpoint not found for acknowledgment");
            continue;
        }
        let endpoint = endpoint.unwrap();

        // Create client and send acknowledgment
        match CollectorClient::new(endpoint) {
            Ok(client) => {
                info!(
                    collector_id = %collector_id,
                    count = seq_nums.len(),
                    "Sending acknowledgments"
                );

                match client.acknowledge(seq_nums.clone()).await {
                    Ok(_) => {
                        // Update last acknowledged sequence
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

async fn save_parent_checkpoint_once(
    storage: Arc<dyn Storage>,
    ack_state: Arc<RwLock<AcknowledgmentState>>,
    config_version: u64,
    fiber_processor: Arc<RwLock<FiberProcessor>>,
) -> Result<(), ParentError> {
    let checkpoint = {
        let state = ack_state.read().await;
        let processor = fiber_processor.read().await;

        ParentCheckpoint {
            version: 1,
            timestamp: Utc::now(),
            config_version,
            collectors: state.collector_checkpoints.clone(),
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::new(),
            },
            fiber_processors: processor.create_checkpoint(),
        }
    };

    let mut manager = CheckpointManager::new(storage, Duration::from_secs(0));
    manager
        .save_parent(&checkpoint)
        .await
        .map_err(ParentError::Checkpoint)?;
    info!(
        collectors = checkpoint.collectors.len(),
        "Saved final parent checkpoint"
    );
    Ok(())
}
