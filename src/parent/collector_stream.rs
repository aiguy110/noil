use crate::parent::collector_client::{CollectorClient, CollectorClientError};
use crate::source::reader::LogRecord;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CollectorStreamError {
    #[error("Collector client error: {0}")]
    ClientError(#[from] CollectorClientError),

    #[error("Stream closed")]
    StreamClosed,
}

pub type Result<T> = std::result::Result<T, CollectorStreamError>;

/// Adapts a collector as a source-like stream for hierarchical sequencing
///
/// This implements the same interface as SourceReader, allowing collectors
/// to be treated as regular sources by the sequencer.
#[derive(Debug)]
pub struct CollectorStream {
    collector_id: String,
    client: CollectorClient,
    last_sequence: u64,

    /// Watermark with generation for lexicographic comparison
    /// (generation, timestamp) - generation takes precedence
    watermark: Option<(u64, DateTime<Utc>)>,

    /// Queue of logs from fetched batches
    batch_queue: VecDeque<LogRecord>,

    /// Batch size limit for fetches
    fetch_limit: usize,

    /// Whether the stream has been closed
    closed: bool,

    /// Whether we've ever fetched batches (used to distinguish first fetch)
    has_fetched: bool,
}

impl CollectorStream {
    pub fn new(client: CollectorClient) -> Self {
        let collector_id = client.collector_id().to_string();

        Self {
            collector_id,
            client,
            last_sequence: 0,
            watermark: None,
            batch_queue: VecDeque::new(),
            fetch_limit: 10,
            closed: false,
            has_fetched: false,
        }
    }

    /// Get the collector ID
    pub fn collector_id(&self) -> &str {
        &self.collector_id
    }

    /// Fetch new batches from the collector and enqueue their logs.
    ///
    /// Returns the number of newly enqueued log records (may be 0).
    pub async fn fetch_new_logs(&mut self) -> Result<usize> {
        if self.closed {
            return Ok(0);
        }

        self.fetch_batch().await
    }

    /// Pop a queued log record without fetching new batches.
    ///
    /// Returns None if the in-memory queue is empty.
    pub fn pop_queued_record(&mut self) -> Option<LogRecord> {
        self.batch_queue.pop_front()
    }

    /// Get the current watermark timestamp
    ///
    /// Returns None if no batches have been fetched yet
    pub fn watermark(&self) -> Option<DateTime<Utc>> {
        self.watermark.map(|(_, ts)| ts)
    }

    /// Get the current watermark with generation
    ///
    /// Used for lexicographic comparison: generation first, then timestamp
    pub fn watermark_with_generation(&self) -> Option<(u64, DateTime<Utc>)> {
        self.watermark
    }

    /// Get the last processed sequence number
    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Close the stream
    pub fn close(&mut self) {
        self.closed = true;
        self.batch_queue.clear();
    }

    /// Fetch the next batch from the collector and queue its logs
    async fn fetch_batch(&mut self) -> Result<usize> {
        // On first fetch, use None to get batches from the beginning
        // On subsequent fetches, use Some(last_sequence) to get batches after what we've seen
        let after = if self.has_fetched {
            Some(self.last_sequence)
        } else {
            None
        };

        let response = self
            .client
            .get_batches(after, self.fetch_limit)
            .await?;

        if response.batches.is_empty() {
            tracing::trace!(
                collector_id = %self.collector_id,
                last_sequence = self.last_sequence,
                has_fetched = self.has_fetched,
                "No new batches available"
            );
            return Ok(0);
        }

        // Mark that we've fetched at least once
        self.has_fetched = true;

        tracing::debug!(
            collector_id = %self.collector_id,
            batch_count = response.batches.len(),
            has_more = response.has_more,
            "Fetched batches from collector"
        );

        let mut newly_enqueued = 0usize;

        for batch in response.batches {
            // Update watermark with generation
            // Lexicographic comparison: generation first, then timestamp
            let new_watermark = (batch.epoch.generation, batch.epoch.watermark);

            match self.watermark {
                Some(current) => {
                    // Verify watermark monotonicity within same generation
                    if new_watermark < current {
                        tracing::warn!(
                            collector_id = %self.collector_id,
                            old_generation = current.0,
                            old_watermark = %current.1,
                            new_generation = new_watermark.0,
                            new_watermark = %new_watermark.1,
                            "Watermark moved backward (expected after rewind)"
                        );
                    }
                    self.watermark = Some(new_watermark);
                }
                None => {
                    self.watermark = Some(new_watermark);
                }
            }

            // Queue all logs from this batch
            let log_count = batch.logs.len();
            self.batch_queue.extend(batch.logs);
            newly_enqueued += log_count;

            // Update last sequence
            self.last_sequence = batch.sequence_num;

            tracing::trace!(
                collector_id = %self.collector_id,
                sequence_num = batch.sequence_num,
                log_count = log_count,
                generation = batch.epoch.generation,
                watermark = %batch.epoch.watermark,
                "Queued logs from batch"
            );
        }

        Ok(newly_enqueued)
    }

    /// Reset the stream to start from a specific sequence
    ///
    /// Used for recovery after parent restart
    pub fn reset_to_sequence(&mut self, sequence: u64) {
        self.last_sequence = sequence;
        self.batch_queue.clear();
        self.watermark = None;
        // Mark as fetched since we're explicitly resuming from a known sequence
        self.has_fetched = true;
        tracing::info!(
            collector_id = %self.collector_id,
            sequence = sequence,
            "Reset stream to sequence"
        );
    }

    /// Get statistics about the stream
    pub fn stats(&self) -> StreamStats {
        StreamStats {
            collector_id: self.collector_id.clone(),
            last_sequence: self.last_sequence,
            queued_logs: self.batch_queue.len(),
            watermark: self.watermark,
            closed: self.closed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamStats {
    pub collector_id: String,
    pub last_sequence: u64,
    pub queued_logs: usize,
    pub watermark: Option<(u64, DateTime<Utc>)>,
    pub closed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::CollectorEndpoint;
    use chrono::Utc;
    use std::time::Duration;

    fn make_test_client() -> CollectorClient {
        let config = CollectorEndpoint {
            id: "test-collector".to_string(),
            url: "http://localhost:7105".to_string(),
            retry_interval: Duration::from_secs(5),
            timeout: Duration::from_secs(30),
        };
        CollectorClient::new(&config).unwrap()
    }

    #[test]
    fn test_stream_creation() {
        let client = make_test_client();
        let stream = CollectorStream::new(client);

        assert_eq!(stream.collector_id(), "test-collector");
        assert_eq!(stream.last_sequence(), 0);
        assert!(stream.watermark().is_none());
    }

    #[test]
    fn test_watermark_lexicographic_comparison() {
        // Generation takes precedence over timestamp
        let gen0_time1 = (0, Utc::now());
        let gen1_time0 = (1, Utc::now() - chrono::Duration::hours(1));

        // Generation 1 is greater even though timestamp is earlier
        assert!(gen1_time0 > gen0_time1);
    }

    #[tokio::test]
    async fn test_fetch_new_logs_when_closed_is_noop() {
        let client = make_test_client();
        let mut stream = CollectorStream::new(client);

        stream.close();

        let count = stream.fetch_new_logs().await.unwrap();
        assert_eq!(count, 0);
        assert!(stream.pop_queued_record().is_none());
    }

    #[test]
    fn test_stats_reflect_closed_stream() {
        let client = make_test_client();
        let mut stream = CollectorStream::new(client);

        stream.close();

        let stats = stream.stats();
        assert!(stats.closed);
        assert_eq!(stats.queued_logs, 0);
    }
}
