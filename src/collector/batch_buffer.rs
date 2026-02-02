use crate::collector::batch::LogBatch;
use crate::config::types::BufferStrategy;
use std::collections::{HashSet, VecDeque};
use std::time::Instant;
use thiserror::Error;

pub struct BatchBuffer {
    max_epochs: usize,
    strategy: BufferStrategy,
    buffer: VecDeque<BufferedBatch>,
    acknowledged: HashSet<u64>,
}

struct BufferedBatch {
    batch: LogBatch,
    #[allow(dead_code)]
    created_at: Instant,
}

impl BatchBuffer {
    pub fn new(max_epochs: usize, strategy: BufferStrategy) -> Self {
        Self {
            max_epochs,
            strategy,
            buffer: VecDeque::new(),
            acknowledged: HashSet::new(),
        }
    }

    /// Add a batch to the buffer
    /// Returns Err if buffer is full and strategy is Block
    pub fn push(&mut self, batch: LogBatch) -> Result<(), BufferError> {
        // Check if buffer is full
        if self.buffer.len() >= self.max_epochs {
            match self.strategy {
                BufferStrategy::Block => {
                    return Err(BufferError::BufferFull);
                }
                BufferStrategy::DropOldest => {
                    // Remove oldest unacknowledged batch
                    if let Some(removed) = self.buffer.pop_front() {
                        tracing::warn!(
                            sequence_num = removed.batch.sequence_num,
                            "Dropping oldest batch due to buffer full"
                        );
                    }
                }
                BufferStrategy::WaitForever => {
                    // No limit, just grow
                }
            }
        }

        self.buffer.push_back(BufferedBatch {
            batch,
            created_at: Instant::now(),
        });

        Ok(())
    }

    /// Get batches after the specified sequence number, up to limit
    /// If after_seq is None, returns batches from the beginning (sequence_num >= 0)
    /// If after_seq is Some(n), returns batches with sequence_num > n
    pub fn get_batches(&self, after_seq: Option<u64>, limit: usize) -> Vec<LogBatch> {
        self.buffer
            .iter()
            .filter(|b| match after_seq {
                None => true, // From beginning - include all batches
                Some(seq) => b.batch.sequence_num > seq,
            })
            .take(limit)
            .map(|b| b.batch.clone())
            .collect()
    }

    /// Mark batches as acknowledged
    pub fn acknowledge(&mut self, sequence_nums: Vec<u64>) -> usize {
        let count = sequence_nums.len();
        self.acknowledged.extend(sequence_nums);
        count
    }

    /// Remove acknowledged batches (called periodically, e.g., every 10s)
    pub fn compact(&mut self) -> usize {
        let before = self.buffer.len();

        self.buffer.retain(|b| {
            !self.acknowledged.contains(&b.batch.sequence_num)
        });

        let removed = before - self.buffer.len();

        // Clear acknowledged set after compaction
        self.acknowledged.clear();

        removed
    }

    /// Get buffer statistics
    pub fn stats(&self) -> BufferStats {
        let oldest_sequence = self.buffer.front().map(|b| b.batch.sequence_num).unwrap_or(0);
        let newest_sequence = self.buffer.back().map(|b| b.batch.sequence_num).unwrap_or(0);

        BufferStats {
            current_epochs: self.buffer.len(),
            max_epochs: self.max_epochs,
            oldest_sequence,
            newest_sequence,
            acknowledged_count: self.acknowledged.len(),
        }
    }

    /// Clear all batches from buffer
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.acknowledged.clear();
    }
}

#[derive(Debug, Clone)]
pub struct BufferStats {
    pub current_epochs: usize,
    pub max_epochs: usize,
    pub oldest_sequence: u64,
    pub newest_sequence: u64,
    pub acknowledged_count: usize,
}

#[derive(Debug, Error)]
pub enum BufferError {
    #[error("Buffer is full")]
    BufferFull,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::batch::EpochInfo;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn make_batch(sequence_num: u64) -> LogBatch {
        let timestamp = Utc.with_ymd_and_hms(2026, 1, 28, 10, 0, 0).unwrap();
        LogBatch {
            batch_id: Uuid::new_v4(),
            collector_id: "test".to_string(),
            epoch: EpochInfo {
                start: timestamp,
                end: timestamp + chrono::Duration::seconds(10),
                watermark: timestamp + chrono::Duration::seconds(10),
                generation: 0,
            },
            logs: Vec::new(),
            config_version: 1,
            sequence_num,
        }
    }

    #[test]
    fn test_buffer_respects_max_epochs() {
        let mut buffer = BatchBuffer::new(3, BufferStrategy::Block);

        // Fill buffer to max
        assert!(buffer.push(make_batch(0)).is_ok());
        assert!(buffer.push(make_batch(1)).is_ok());
        assert!(buffer.push(make_batch(2)).is_ok());

        // Next push should fail with Block strategy
        assert!(matches!(buffer.push(make_batch(3)), Err(BufferError::BufferFull)));

        let stats = buffer.stats();
        assert_eq!(stats.current_epochs, 3);
        assert_eq!(stats.max_epochs, 3);
    }

    #[test]
    fn test_drop_oldest_strategy() {
        let mut buffer = BatchBuffer::new(3, BufferStrategy::DropOldest);

        // Fill buffer to max
        assert!(buffer.push(make_batch(0)).is_ok());
        assert!(buffer.push(make_batch(1)).is_ok());
        assert!(buffer.push(make_batch(2)).is_ok());

        // Next push should succeed, dropping oldest (0)
        assert!(buffer.push(make_batch(3)).is_ok());

        let stats = buffer.stats();
        assert_eq!(stats.current_epochs, 3);
        assert_eq!(stats.oldest_sequence, 1);
        assert_eq!(stats.newest_sequence, 3);
    }

    #[test]
    fn test_wait_forever_strategy() {
        let mut buffer = BatchBuffer::new(3, BufferStrategy::WaitForever);

        // Fill buffer beyond max
        assert!(buffer.push(make_batch(0)).is_ok());
        assert!(buffer.push(make_batch(1)).is_ok());
        assert!(buffer.push(make_batch(2)).is_ok());
        assert!(buffer.push(make_batch(3)).is_ok());
        assert!(buffer.push(make_batch(4)).is_ok());

        let stats = buffer.stats();
        assert_eq!(stats.current_epochs, 5);
        assert_eq!(stats.max_epochs, 3); // max_epochs is just a hint with WaitForever
    }

    #[test]
    fn test_get_batches_filters_correctly() {
        let mut buffer = BatchBuffer::new(10, BufferStrategy::Block);

        buffer.push(make_batch(5)).unwrap();
        buffer.push(make_batch(10)).unwrap();
        buffer.push(make_batch(15)).unwrap();
        buffer.push(make_batch(20)).unwrap();

        // Get batches after sequence 10
        let batches = buffer.get_batches(Some(10), 10);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].sequence_num, 15);
        assert_eq!(batches[1].sequence_num, 20);
    }

    #[test]
    fn test_get_batches_respects_limit() {
        let mut buffer = BatchBuffer::new(10, BufferStrategy::Block);

        buffer.push(make_batch(0)).unwrap();
        buffer.push(make_batch(1)).unwrap();
        buffer.push(make_batch(2)).unwrap();
        buffer.push(make_batch(3)).unwrap();

        // Get only 2 batches
        let batches = buffer.get_batches(Some(0), 2);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].sequence_num, 1);
        assert_eq!(batches[1].sequence_num, 2);
    }

    #[test]
    fn test_acknowledge_and_compact() {
        let mut buffer = BatchBuffer::new(10, BufferStrategy::Block);

        buffer.push(make_batch(0)).unwrap();
        buffer.push(make_batch(1)).unwrap();
        buffer.push(make_batch(2)).unwrap();
        buffer.push(make_batch(3)).unwrap();

        // Acknowledge some batches
        let ack_count = buffer.acknowledge(vec![0, 2]);
        assert_eq!(ack_count, 2);

        let stats = buffer.stats();
        assert_eq!(stats.acknowledged_count, 2);

        // Compact should remove acknowledged batches
        let removed = buffer.compact();
        assert_eq!(removed, 2);

        let stats = buffer.stats();
        assert_eq!(stats.current_epochs, 2);
        assert_eq!(stats.oldest_sequence, 1);
        assert_eq!(stats.newest_sequence, 3);
        assert_eq!(stats.acknowledged_count, 0); // Cleared after compaction
    }

    #[test]
    fn test_stats_accuracy() {
        let mut buffer = BatchBuffer::new(10, BufferStrategy::Block);

        // Empty buffer
        let stats = buffer.stats();
        assert_eq!(stats.current_epochs, 0);
        assert_eq!(stats.oldest_sequence, 0);
        assert_eq!(stats.newest_sequence, 0);

        // Add batches
        buffer.push(make_batch(100)).unwrap();
        buffer.push(make_batch(101)).unwrap();
        buffer.push(make_batch(102)).unwrap();

        let stats = buffer.stats();
        assert_eq!(stats.current_epochs, 3);
        assert_eq!(stats.oldest_sequence, 100);
        assert_eq!(stats.newest_sequence, 102);

        // Acknowledge one
        buffer.acknowledge(vec![101]);
        let stats = buffer.stats();
        assert_eq!(stats.acknowledged_count, 1);
    }

    #[test]
    fn test_clear() {
        let mut buffer = BatchBuffer::new(10, BufferStrategy::Block);

        buffer.push(make_batch(0)).unwrap();
        buffer.push(make_batch(1)).unwrap();
        buffer.acknowledge(vec![0]);

        assert_eq!(buffer.stats().current_epochs, 2);
        assert_eq!(buffer.stats().acknowledged_count, 1);

        buffer.clear();

        assert_eq!(buffer.stats().current_epochs, 0);
        assert_eq!(buffer.stats().acknowledged_count, 0);
    }
}
