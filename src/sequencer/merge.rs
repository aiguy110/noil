use crate::config::types::SequencerConfig;
use crate::sequencer::local::Sequencer;
use crate::source::reader::{LogRecord, SourceReader};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Error)]
pub enum SequencerError {
    #[error("source reader error: {0}")]
    SourceReader(String),

    #[error("channel send error")]
    ChannelSend,

    #[error("join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// Configuration for the async sequencer runner
pub struct SequencerRunConfig {
    pub safety_margin: Duration,
    pub emit_interval: Duration,
}

impl Default for SequencerRunConfig {
    fn default() -> Self {
        Self {
            safety_margin: Duration::from_secs(1),
            emit_interval: Duration::from_millis(100),
        }
    }
}

impl From<&SequencerConfig> for SequencerRunConfig {
    fn from(config: &SequencerConfig) -> Self {
        Self {
            safety_margin: config
                .watermark_safety_margin
                .unwrap_or(Duration::from_secs(1)),
            emit_interval: Duration::from_millis(100),
        }
    }
}

/// Run the sequencer with multiple source readers.
///
/// Spawns a task for each source reader, collects records via channels,
/// and emits them in timestamp order to the output channel.
///
/// # Arguments
/// * `sources` - List of source readers to merge
/// * `output` - Channel to send sequenced records to
/// * `config` - Sequencer configuration
///
/// # Returns
/// A handle that can be awaited to wait for sequencer completion
pub async fn run_sequencer(
    sources: Vec<SourceReader>,
    output: mpsc::Sender<LogRecord>,
    config: SequencerRunConfig,
) -> Result<SequencerHandle, SequencerError> {
    if sources.is_empty() {
        return Err(SequencerError::SourceReader(
            "no sources provided".to_string(),
        ));
    }

    let source_ids: Vec<String> = sources.iter().map(|s| s.source_id().to_string()).collect();

    // Create channels for each source
    let (source_txs, mut source_rxs): (Vec<_>, Vec<_>) = source_ids
        .iter()
        .map(|_| mpsc::channel::<SourceEvent>(1000))
        .unzip();

    // Spawn a task for each source reader
    let mut source_handles = Vec::new();
    for (mut reader, tx) in sources.into_iter().zip(source_txs.into_iter()) {
        let handle = tokio::spawn(async move {
            loop {
                match reader.next_record().await {
                    Ok(Some(record)) => {
                        let watermark = reader.watermark();
                        if tx
                            .send(SourceEvent::Record {
                                record,
                                watermark: watermark.unwrap(),
                            })
                            .await
                            .is_err()
                        {
                            break; // Receiver dropped
                        }
                    }
                    Ok(None) => {
                        // EOF reached
                        let _ = tx.send(SourceEvent::Done).await;
                        break;
                    }
                    Err(e) => {
                        let _ = tx.send(SourceEvent::Error(format!("{}", e))).await;
                        break;
                    }
                }
            }
        });
        source_handles.push(handle);
    }

    // Spawn the main sequencer task
    let sequencer_handle = tokio::spawn(async move {
        let mut sequencer = Sequencer::new(source_ids.clone(), config.safety_margin);
        let mut emit_interval = tokio::time::interval(config.emit_interval);
        let mut active_sources = source_ids.len();

        loop {
            tokio::select! {
                // Check each source channel
                Some((source_idx, event)) = receive_any(&mut source_rxs) => {
                    let source_id = &source_ids[source_idx];

                    match event {
                        SourceEvent::Record { record, watermark } => {
                            sequencer.push(record);
                            sequencer.update_watermark(source_id, watermark);
                        }
                        SourceEvent::Done => {
                            sequencer.mark_source_done(source_id);
                            active_sources -= 1;

                            if active_sources == 0 {
                                // All sources done, flush and exit
                                let remaining = sequencer.flush_all();
                                for record in remaining {
                                    if output.send(record).await.is_err() {
                                        return Err(SequencerError::ChannelSend);
                                    }
                                }
                                return Ok(());
                            }
                        }
                        SourceEvent::Error(e) => {
                            return Err(SequencerError::SourceReader(e));
                        }
                    }

                    // Try to emit ready records
                    let ready = sequencer.emit_ready();
                    for record in ready {
                        if output.send(record).await.is_err() {
                            return Err(SequencerError::ChannelSend);
                        }
                    }
                }

                // Periodic emit check
                _ = emit_interval.tick() => {
                    let ready = sequencer.emit_ready();
                    for record in ready {
                        if output.send(record).await.is_err() {
                            return Err(SequencerError::ChannelSend);
                        }
                    }
                }
            }
        }
    });

    Ok(SequencerHandle {
        sequencer_task: sequencer_handle,
        source_tasks: source_handles,
    })
}

/// Events sent from source reader tasks to the main sequencer
#[derive(Debug)]
enum SourceEvent {
    Record {
        record: LogRecord,
        watermark: chrono::DateTime<chrono::Utc>,
    },
    Done,
    Error(String),
}

/// Handle to the running sequencer
pub struct SequencerHandle {
    sequencer_task: JoinHandle<Result<(), SequencerError>>,
    source_tasks: Vec<JoinHandle<()>>,
}

impl SequencerHandle {
    /// Wait for the sequencer to complete
    pub async fn wait(self) -> Result<(), SequencerError> {
        // Wait for main sequencer task
        let result = self.sequencer_task.await??;

        // Wait for all source tasks to complete
        for task in self.source_tasks {
            task.await?;
        }

        Ok(result)
    }

    /// Abort the sequencer and all source tasks
    pub fn abort(&self) {
        self.sequencer_task.abort();
        for task in &self.source_tasks {
            task.abort();
        }
    }
}

/// Helper function to receive from any of the source channels
async fn receive_any(
    receivers: &mut [mpsc::Receiver<SourceEvent>],
) -> Option<(usize, SourceEvent)> {
    use tokio::sync::mpsc::error::TryRecvError;

    loop {
        // Try non-blocking receive from all channels
        for (idx, rx) in receivers.iter_mut().enumerate() {
            match rx.try_recv() {
                Ok(event) => return Some((idx, event)),
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => continue,
            }
        }

        // If all channels are empty, wait for any to receive
        for (idx, rx) in receivers.iter_mut().enumerate() {
            tokio::select! {
                biased;
                result = rx.recv() => {
                    if let Some(event) = result {
                        return Some((idx, event));
                    }
                }
                else => break,
            }
        }

        // If all channels are closed, return None
        if receivers.iter_mut().all(|rx| rx.is_closed()) {
            return None;
        }

        // Small delay to avoid busy-waiting
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{
        ParseErrorStrategy, ReadConfig, ReadStart, SourceConfig, SourceType, TimestampConfig,
    };
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    fn create_test_config(path: PathBuf, pattern: &str, format: &str) -> SourceConfig {
        SourceConfig {
            source_type: SourceType::File,
            path,
            timestamp: TimestampConfig {
                pattern: pattern.to_string(),
                format: format.to_string(),
            },
            read: ReadConfig {
                start: ReadStart::Beginning,
                follow: false,
            },
        }
    }

    #[tokio::test]
    async fn test_single_source() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z Line 1").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Line 2").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:02Z Line 3").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        let (output_tx, mut output_rx) = mpsc::channel(100);

        let run_config = SequencerRunConfig {
            safety_margin: Duration::from_secs(0),
            emit_interval: Duration::from_millis(10),
        };

        let handle = run_sequencer(vec![reader], output_tx, run_config)
            .await
            .unwrap();

        // Collect all output records
        let mut records = Vec::new();
        while let Some(record) = output_rx.recv().await {
            records.push(record);
        }

        handle.wait().await.unwrap();

        assert_eq!(records.len(), 3);
        assert_eq!(records[0].raw_text, "2025-12-04T10:00:00Z Line 1");
        assert_eq!(records[1].raw_text, "2025-12-04T10:00:01Z Line 2");
        assert_eq!(records[2].raw_text, "2025-12-04T10:00:02Z Line 3");
    }

    #[tokio::test]
    async fn test_two_sources_merged() {
        // Create two temp files with interleaved timestamps
        let mut temp_file1 = NamedTempFile::new().unwrap();
        writeln!(temp_file1, "2025-12-04T10:00:00Z Source1 Line1").unwrap();
        writeln!(temp_file1, "2025-12-04T10:00:02Z Source1 Line2").unwrap();
        writeln!(temp_file1, "2025-12-04T10:00:04Z Source1 Line3").unwrap();
        temp_file1.flush().unwrap();

        let mut temp_file2 = NamedTempFile::new().unwrap();
        writeln!(temp_file2, "2025-12-04T10:00:01Z Source2 Line1").unwrap();
        writeln!(temp_file2, "2025-12-04T10:00:03Z Source2 Line2").unwrap();
        writeln!(temp_file2, "2025-12-04T10:00:05Z Source2 Line3").unwrap();
        temp_file2.flush().unwrap();

        let config1 = create_test_config(
            temp_file1.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let config2 = create_test_config(
            temp_file2.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let reader1 =
            SourceReader::new("source1".to_string(), &config1, ParseErrorStrategy::Panic).unwrap();
        let reader2 =
            SourceReader::new("source2".to_string(), &config2, ParseErrorStrategy::Panic).unwrap();

        let (output_tx, mut output_rx) = mpsc::channel(100);

        let run_config = SequencerRunConfig {
            safety_margin: Duration::from_secs(0),
            emit_interval: Duration::from_millis(10),
        };

        let handle = run_sequencer(vec![reader1, reader2], output_tx, run_config)
            .await
            .unwrap();

        // Collect all output records
        let mut records = Vec::new();
        while let Some(record) = output_rx.recv().await {
            records.push(record);
        }

        handle.wait().await.unwrap();

        // Should be merged in timestamp order
        assert_eq!(records.len(), 6);
        assert_eq!(records[0].raw_text, "2025-12-04T10:00:00Z Source1 Line1");
        assert_eq!(records[1].raw_text, "2025-12-04T10:00:01Z Source2 Line1");
        assert_eq!(records[2].raw_text, "2025-12-04T10:00:02Z Source1 Line2");
        assert_eq!(records[3].raw_text, "2025-12-04T10:00:03Z Source2 Line2");
        assert_eq!(records[4].raw_text, "2025-12-04T10:00:04Z Source1 Line3");
        assert_eq!(records[5].raw_text, "2025-12-04T10:00:05Z Source2 Line3");
    }

    #[tokio::test]
    async fn test_empty_source() {
        let temp_file = NamedTempFile::new().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        let (output_tx, mut output_rx) = mpsc::channel(100);

        let run_config = SequencerRunConfig {
            safety_margin: Duration::from_secs(0),
            emit_interval: Duration::from_millis(10),
        };

        let handle = run_sequencer(vec![reader], output_tx, run_config)
            .await
            .unwrap();

        // Collect all output records
        let mut records = Vec::new();
        while let Some(record) = output_rx.recv().await {
            records.push(record);
        }

        handle.wait().await.unwrap();

        assert_eq!(records.len(), 0);
    }

    #[tokio::test]
    async fn test_three_sources() {
        // Create three temp files
        let mut temp_file1 = NamedTempFile::new().unwrap();
        writeln!(temp_file1, "2025-12-04T10:00:00Z S1-1").unwrap();
        writeln!(temp_file1, "2025-12-04T10:00:03Z S1-2").unwrap();
        temp_file1.flush().unwrap();

        let mut temp_file2 = NamedTempFile::new().unwrap();
        writeln!(temp_file2, "2025-12-04T10:00:01Z S2-1").unwrap();
        writeln!(temp_file2, "2025-12-04T10:00:04Z S2-2").unwrap();
        temp_file2.flush().unwrap();

        let mut temp_file3 = NamedTempFile::new().unwrap();
        writeln!(temp_file3, "2025-12-04T10:00:02Z S3-1").unwrap();
        writeln!(temp_file3, "2025-12-04T10:00:05Z S3-2").unwrap();
        temp_file3.flush().unwrap();

        let config1 = create_test_config(
            temp_file1.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );
        let config2 = create_test_config(
            temp_file2.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );
        let config3 = create_test_config(
            temp_file3.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let reader1 =
            SourceReader::new("source1".to_string(), &config1, ParseErrorStrategy::Panic).unwrap();
        let reader2 =
            SourceReader::new("source2".to_string(), &config2, ParseErrorStrategy::Panic).unwrap();
        let reader3 =
            SourceReader::new("source3".to_string(), &config3, ParseErrorStrategy::Panic).unwrap();

        let (output_tx, mut output_rx) = mpsc::channel(100);

        let run_config = SequencerRunConfig {
            safety_margin: Duration::from_secs(0),
            emit_interval: Duration::from_millis(10),
        };

        let handle = run_sequencer(vec![reader1, reader2, reader3], output_tx, run_config)
            .await
            .unwrap();

        // Collect all output records
        let mut records = Vec::new();
        while let Some(record) = output_rx.recv().await {
            records.push(record);
        }

        handle.wait().await.unwrap();

        assert_eq!(records.len(), 6);
        assert_eq!(records[0].raw_text, "2025-12-04T10:00:00Z S1-1");
        assert_eq!(records[1].raw_text, "2025-12-04T10:00:01Z S2-1");
        assert_eq!(records[2].raw_text, "2025-12-04T10:00:02Z S3-1");
        assert_eq!(records[3].raw_text, "2025-12-04T10:00:03Z S1-2");
        assert_eq!(records[4].raw_text, "2025-12-04T10:00:04Z S2-2");
        assert_eq!(records[5].raw_text, "2025-12-04T10:00:05Z S3-2");
    }
}
