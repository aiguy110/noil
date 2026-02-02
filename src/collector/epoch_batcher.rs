use crate::collector::batch::{EpochInfo, LogBatch};
use crate::source::reader::LogRecord;
use chrono::{DateTime, Utc};
use std::time::Duration;
use uuid::Uuid;

pub struct EpochBatcher {
    collector_id: String,
    epoch_duration: Duration,
    current_epoch: Option<EpochBuilder>,
    sequence_counter: u64,
    config_version: u64,
    rewind_generation: u64,
}

struct EpochBuilder {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    logs: Vec<LogRecord>,
}

impl EpochBatcher {
    pub fn new(
        collector_id: String,
        epoch_duration: Duration,
        config_version: u64,
    ) -> Self {
        Self {
            collector_id,
            epoch_duration,
            current_epoch: None,
            sequence_counter: 0,
            config_version,
            rewind_generation: 0,
        }
    }

    /// Add a log to the batcher
    /// Returns a completed batch if epoch boundary crossed
    pub fn push(&mut self, log: LogRecord) -> Option<LogBatch> {
        // If no current epoch, start one
        if self.current_epoch.is_none() {
            self.start_new_epoch(log.timestamp);
        }

        let current = self.current_epoch.as_mut().unwrap();

        // Check if log belongs to current epoch
        if log.timestamp < current.end {
            current.logs.push(log);
            None
        } else {
            // Log crosses epoch boundary - complete current epoch
            let completed = self.complete_current_epoch();

            // Start new epoch with this log
            self.start_new_epoch(log.timestamp);
            self.current_epoch.as_mut().unwrap().logs.push(log);

            completed
        }
    }

    /// Force completion of current epoch (called on shutdown or watermark update)
    pub fn flush_current(&mut self, watermark: DateTime<Utc>) -> Option<LogBatch> {
        self.current_epoch.take().map(|epoch_builder| {
            self.create_batch(epoch_builder, watermark)
        })
    }

    /// Rewind to a specific sequence number
    pub fn rewind(&mut self, target_sequence: u64) {
        self.sequence_counter = target_sequence;
        self.current_epoch = None;
        self.rewind_generation += 1;
    }

    /// Restore from checkpoint (doesn't increment generation)
    pub fn restore_from_checkpoint(&mut self, sequence_counter: u64, rewind_generation: u64) {
        self.sequence_counter = sequence_counter;
        self.rewind_generation = rewind_generation;
        self.current_epoch = None;
    }

    /// Get current sequence counter
    pub fn sequence_counter(&self) -> u64 {
        self.sequence_counter
    }

    /// Get current rewind generation
    pub fn rewind_generation(&self) -> u64 {
        self.rewind_generation
    }

    fn start_new_epoch(&mut self, first_timestamp: DateTime<Utc>) {
        let start = self.epoch_start_for_timestamp(first_timestamp);
        let end = start + chrono::Duration::from_std(self.epoch_duration).unwrap();

        self.current_epoch = Some(EpochBuilder {
            start,
            end,
            logs: Vec::new(),
        });
    }

    fn complete_current_epoch(&mut self) -> Option<LogBatch> {
        let epoch_builder = self.current_epoch.take()?;
        let watermark = epoch_builder.end;
        Some(self.create_batch(epoch_builder, watermark))
    }

    fn create_batch(
        &mut self,
        epoch_builder: EpochBuilder,
        watermark: DateTime<Utc>,
    ) -> LogBatch {
        let batch = LogBatch {
            batch_id: Uuid::new_v4(),
            collector_id: self.collector_id.clone(),
            epoch: EpochInfo {
                start: epoch_builder.start,
                end: epoch_builder.end,
                watermark,
                generation: self.rewind_generation,
            },
            logs: epoch_builder.logs,
            config_version: self.config_version,
            sequence_num: self.sequence_counter,
        };

        self.sequence_counter += 1;
        batch
    }

    /// Calculate the epoch start time for a given timestamp
    /// Rounds down to the nearest epoch boundary
    pub fn epoch_start_for_timestamp(&self, timestamp: DateTime<Utc>) -> DateTime<Utc> {
        // Round down to epoch boundary
        let epoch_duration_secs = self.epoch_duration.as_secs() as i64;
        let timestamp_secs = timestamp.timestamp();
        let epoch_start_secs = (timestamp_secs / epoch_duration_secs) * epoch_duration_secs;
        DateTime::from_timestamp(epoch_start_secs, 0).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_log(timestamp: DateTime<Utc>, text: &str) -> LogRecord {
        LogRecord {
            id: Uuid::new_v4(),
            timestamp,
            source_id: "test_source".to_string(),
            raw_text: text.to_string(),
            file_offset: 0,
        }
    }

    #[test]
    fn test_epoch_batching_within_epoch() {
        let mut batcher = EpochBatcher::new(
            "collector1".to_string(),
            Duration::from_secs(10),
            1,
        );

        let base = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 0).unwrap();

        // All logs within same 10s epoch
        let batch1 = batcher.push(make_log(base, "log1"));
        assert!(batch1.is_none());

        let batch2 = batcher.push(make_log(base + chrono::Duration::seconds(5), "log2"));
        assert!(batch2.is_none());

        let batch3 = batcher.push(make_log(base + chrono::Duration::seconds(9), "log3"));
        assert!(batch3.is_none());

        // Flush should return a batch with all 3 logs
        let batch = batcher.flush_current(base + chrono::Duration::seconds(10)).unwrap();
        assert_eq!(batch.logs.len(), 3);
        assert_eq!(batch.sequence_num, 0);
        assert_eq!(batch.epoch.generation, 0);
    }

    #[test]
    fn test_epoch_boundary_crossing() {
        let mut batcher = EpochBatcher::new(
            "collector1".to_string(),
            Duration::from_secs(10),
            1,
        );

        let base = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 0).unwrap();

        // First log in epoch 1
        let batch1 = batcher.push(make_log(base, "log1"));
        assert!(batch1.is_none());

        // Second log crosses into epoch 2, should complete epoch 1
        let batch2 = batcher.push(make_log(base + chrono::Duration::seconds(15), "log2"));
        assert!(batch2.is_some());

        let completed = batch2.unwrap();
        assert_eq!(completed.logs.len(), 1);
        assert_eq!(completed.logs[0].raw_text, "log1");
        assert_eq!(completed.sequence_num, 0);

        // Flush should return epoch 2 with log2
        let batch3 = batcher.flush_current(base + chrono::Duration::seconds(20)).unwrap();
        assert_eq!(batch3.logs.len(), 1);
        assert_eq!(batch3.logs[0].raw_text, "log2");
        assert_eq!(batch3.sequence_num, 1);
    }

    #[test]
    fn test_sequence_increment() {
        let mut batcher = EpochBatcher::new(
            "collector1".to_string(),
            Duration::from_secs(10),
            1,
        );

        let base = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 0).unwrap();

        // Create multiple epochs
        batcher.push(make_log(base, "log1"));
        let batch1 = batcher.push(make_log(base + chrono::Duration::seconds(10), "log2"));
        let batch2 = batcher.push(make_log(base + chrono::Duration::seconds(20), "log3"));
        let batch3 = batcher.flush_current(base + chrono::Duration::seconds(30));

        assert_eq!(batch1.unwrap().sequence_num, 0);
        assert_eq!(batch2.unwrap().sequence_num, 1);
        assert_eq!(batch3.unwrap().sequence_num, 2);
    }

    #[test]
    fn test_rewind_increments_generation() {
        let mut batcher = EpochBatcher::new(
            "collector1".to_string(),
            Duration::from_secs(10),
            1,
        );

        let base = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 0).unwrap();

        batcher.push(make_log(base, "log1"));
        let batch1 = batcher.flush_current(base + chrono::Duration::seconds(10)).unwrap();
        assert_eq!(batch1.epoch.generation, 0);
        assert_eq!(batch1.sequence_num, 0);

        // Rewind to sequence 0
        batcher.rewind(0);

        batcher.push(make_log(base, "log2"));
        let batch2 = batcher.flush_current(base + chrono::Duration::seconds(10)).unwrap();
        assert_eq!(batch2.epoch.generation, 1);
        assert_eq!(batch2.sequence_num, 0);
    }

    #[test]
    fn test_epoch_boundary_calculation() {
        let batcher = EpochBatcher::new(
            "collector1".to_string(),
            Duration::from_secs(10),
            1,
        );

        // Test various timestamps round down correctly
        let t1 = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 5).unwrap();
        let start1 = batcher.epoch_start_for_timestamp(t1);
        assert_eq!(start1, Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 0).unwrap());

        let t2 = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 15).unwrap();
        let start2 = batcher.epoch_start_for_timestamp(t2);
        assert_eq!(start2, Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 10).unwrap());
    }
}
