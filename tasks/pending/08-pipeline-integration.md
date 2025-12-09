# 08: Pipeline Integration

Wire all components together into a working pipeline.

## Location

`src/pipeline/mod.rs`, `src/pipeline/channel.rs`, `src/cli/run.rs`

## Pipeline Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Source    │────▶│  Sequencer  │────▶│    Fiber    │────▶│   Storage   │
│   Readers   │     │             │     │  Processor  │     │   Writer    │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
                                               │
                                               ▼
                                        ┌─────────────┐
                                        │  Raw Log    │
                                        │   Writer    │
                                        └─────────────┘
```

## Channel Setup

```rust
// Sequencer output -> splits to raw writer and fiber processor
let (seq_tx, seq_rx) = mpsc::channel::<LogRecord>(1000);

// Fiber processor output -> storage writer
let (fiber_tx, fiber_rx) = mpsc::channel::<FiberUpdate>(1000);
```

Use bounded channels for backpressure.

## Pipeline Runner (`cli/run.rs`)

```rust
pub async fn run(config: Config) -> Result<()> {
    // Initialize storage
    let storage = DuckDbStorage::new(&config.storage)?;
    storage.init_schema().await?;

    // Create source readers
    let readers: Vec<SourceReader> = config.sources
        .iter()
        .map(|(id, cfg)| SourceReader::new(id.clone(), cfg))
        .collect::<Result<_>>()?;

    // Create fiber processor
    let fiber_processor = FiberProcessor::from_config(&config.fiber_types)?;

    // Create channels
    let (seq_tx, seq_rx) = mpsc::channel(config.pipeline.backpressure.buffer_limit);
    let (fiber_tx, fiber_rx) = mpsc::channel(config.pipeline.backpressure.buffer_limit);

    // Spawn tasks
    let sequencer_handle = tokio::spawn(run_sequencer(readers, seq_tx, config.sequencer));
    let processor_handle = tokio::spawn(run_processor(seq_rx, fiber_tx, fiber_processor, storage.clone()));
    let writer_handle = tokio::spawn(run_writer(fiber_rx, storage.clone(), config.storage));

    // Wait for completion or error
    tokio::select! {
        r = sequencer_handle => r??,
        r = processor_handle => r??,
        r = writer_handle => r??,
    }

    Ok(())
}
```

## Sequencer Task

```rust
async fn run_sequencer(
    readers: Vec<SourceReader>,
    output: mpsc::Sender<LogRecord>,
    config: SequencerConfig,
) -> Result<()> {
    let mut sequencer = Sequencer::new(
        readers.iter().map(|r| r.source_id().to_string()).collect(),
        config.watermark_safety_margin,
    );

    // Channel per reader
    let (txs, mut rxs): (Vec<_>, Vec<_>) = readers
        .iter()
        .map(|_| mpsc::channel::<LogRecord>(100))
        .unzip();

    // Spawn reader tasks
    for (reader, tx) in readers.into_iter().zip(txs) {
        tokio::spawn(async move {
            let mut reader = reader;
            reader.open()?;
            while let Some(record) = reader.next_record().await? {
                if tx.send(record).await.is_err() {
                    break;
                }
            }
            Ok::<_, Error>(())
        });
    }

    // Merge loop
    loop {
        // Select from all reader channels
        // On receive: push to sequencer, update watermark
        // Emit ready records to output
        // Handle channel closures (mark source done)
    }
}
```

## Processor Task

```rust
async fn run_processor(
    mut input: mpsc::Receiver<LogRecord>,
    output: mpsc::Sender<FiberUpdate>,
    mut processor: FiberProcessor,
    storage: Arc<dyn Storage>,
) -> Result<()> {
    let mut log_batch = Vec::with_capacity(BATCH_SIZE);
    let mut flush_interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            Some(log) = input.recv() => {
                // Write raw log
                log_batch.push(log.clone());
                if log_batch.len() >= BATCH_SIZE {
                    storage.write_logs(&log_batch).await?;
                    log_batch.clear();
                }

                // Process for fibers
                let results = processor.process_log(&log);
                for result in results {
                    output.send(FiberUpdate::from(result)).await?;
                }
            }
            _ = flush_interval.tick() => {
                if !log_batch.is_empty() {
                    storage.write_logs(&log_batch).await?;
                    log_batch.clear();
                }
            }
            else => break,
        }
    }

    // Final flush
    if !log_batch.is_empty() {
        storage.write_logs(&log_batch).await?;
    }

    Ok(())
}
```

## Storage Writer Task

```rust
async fn run_writer(
    mut input: mpsc::Receiver<FiberUpdate>,
    storage: Arc<dyn Storage>,
    config: StorageConfig,
) -> Result<()> {
    let mut membership_batch = Vec::new();
    let mut flush_interval = tokio::time::interval(
        Duration::from_secs(config.flush_interval_seconds)
    );

    loop {
        tokio::select! {
            Some(update) = input.recv() => {
                // Write new/updated fibers immediately (or batch)
                for fiber in update.new_fibers {
                    storage.write_fiber(&fiber).await?;
                }
                for fiber in update.updated_fibers {
                    storage.update_fiber(&fiber).await?;
                }

                // Batch memberships
                membership_batch.extend(update.memberships);
                if membership_batch.len() >= config.batch_size {
                    storage.write_memberships(&membership_batch).await?;
                    membership_batch.clear();
                }
            }
            _ = flush_interval.tick() => {
                if !membership_batch.is_empty() {
                    storage.write_memberships(&membership_batch).await?;
                    membership_batch.clear();
                }
            }
            else => break,
        }
    }

    // Final flush
    if !membership_batch.is_empty() {
        storage.write_memberships(&membership_batch).await?;
    }

    Ok(())
}
```

## Graceful Shutdown

Handle SIGINT/SIGTERM:
1. Stop reading new logs
2. Drain sequencer
3. Process remaining logs
4. Flush all batches
5. Close storage

```rust
let shutdown = tokio::signal::ctrl_c();
tokio::select! {
    _ = shutdown => {
        tracing::info!("Shutdown signal received");
        // Trigger graceful shutdown
    }
    // ... other branches
}
```

## Error Handling

Based on config:
- `on_parse_error: drop`: log warning, continue
- `on_parse_error: panic`: propagate error, stop pipeline

Storage errors: retry with exponential backoff, eventually fail.

## Acceptance Criteria

- `noil run` starts pipeline with valid config
- Logs flow through all stages
- Batching reduces storage write frequency
- Backpressure propagates correctly (slow storage slows readers)
- Graceful shutdown flushes all pending data
- Errors logged with context
