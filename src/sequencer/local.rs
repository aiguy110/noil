use crate::source::reader::LogRecord;
use chrono::{DateTime, Utc};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::time::Duration;

/// Local sequencer that merges multiple source streams into global timestamp order.
///
/// Uses watermarks to determine when it's safe to emit records without risking
/// out-of-order delivery.
pub struct Sequencer {
    sources: HashMap<String, SourceState>,
    heap: BinaryHeap<Reverse<HeapEntry>>,
    safety_margin: Duration,
}

#[derive(Debug, Clone)]
struct SourceState {
    watermark: Option<DateTime<Utc>>,
    active: bool,
}

#[derive(Debug, Clone)]
struct HeapEntry {
    timestamp: DateTime<Utc>,
    record: LogRecord,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
    }
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Order by timestamp (min-heap via Reverse wrapper)
        self.timestamp.cmp(&other.timestamp)
    }
}

impl Sequencer {
    /// Create a new sequencer for the given source IDs.
    ///
    /// # Arguments
    /// * `source_ids` - List of source IDs to track
    /// * `safety_margin` - Safety margin to subtract from watermark threshold
    pub fn new(source_ids: Vec<String>, safety_margin: Duration) -> Self {
        let sources = source_ids
            .into_iter()
            .map(|id| {
                (
                    id,
                    SourceState {
                        watermark: None,
                        active: true,
                    },
                )
            })
            .collect();

        Self {
            sources,
            heap: BinaryHeap::new(),
            safety_margin,
        }
    }

    /// Add a record to the heap.
    pub fn push(&mut self, record: LogRecord) {
        self.heap.push(Reverse(HeapEntry {
            timestamp: record.timestamp,
            record,
        }));
    }

    /// Update the watermark for a source.
    ///
    /// The watermark indicates that this source will not emit any records
    /// with timestamps earlier than the given watermark.
    pub fn update_watermark(&mut self, source_id: &str, watermark: DateTime<Utc>) {
        if let Some(state) = self.sources.get_mut(source_id) {
            state.watermark = Some(watermark);
        }
    }

    /// Mark a source as done (no more records will arrive from this source).
    ///
    /// This is typically called when a source reaches EOF and is not following.
    pub fn mark_source_done(&mut self, source_id: &str) {
        if let Some(state) = self.sources.get_mut(source_id) {
            state.active = false;
        }
    }

    /// Emit all records that are safe to emit based on current watermarks.
    ///
    /// A record is safe to emit if its timestamp is strictly less than
    /// the minimum watermark minus the safety margin.
    ///
    /// Watermark semantics: a watermark of T means no records with timestamp < T
    /// will arrive from that source.
    pub fn emit_ready(&mut self) -> Vec<LogRecord> {
        let min_watermark = self.compute_min_watermark();
        let Some(threshold) = min_watermark else {
            return vec![]; // No watermarks yet, can't emit
        };

        // Subtract safety margin to account for clock skew
        let threshold = threshold - chrono::Duration::from_std(self.safety_margin).unwrap();
        let mut result = vec![];

        while let Some(Reverse(entry)) = self.heap.peek() {
            if entry.timestamp < threshold {
                result.push(self.heap.pop().unwrap().0.record);
            } else {
                break;
            }
        }
        result
    }

    /// Flush all remaining records (called at shutdown or when all sources are done).
    pub fn flush_all(&mut self) -> Vec<LogRecord> {
        let mut result = vec![];
        while let Some(Reverse(entry)) = self.heap.pop() {
            result.push(entry.record);
        }
        result
    }

    /// Check if all sources are done (inactive).
    pub fn all_sources_done(&self) -> bool {
        self.sources.values().all(|s| !s.active)
    }

    /// Get the number of records currently buffered in the heap.
    pub fn buffered_count(&self) -> usize {
        self.heap.len()
    }

    /// Create a checkpoint of the sequencer state.
    pub fn create_checkpoint(&self) -> crate::storage::checkpoint::SequencerCheckpoint {
        crate::storage::checkpoint::SequencerCheckpoint {
            watermarks: self
                .sources
                .iter()
                .filter_map(|(id, state)| state.watermark.map(|w| (id.clone(), w)))
                .collect(),
        }
    }

    /// Restore sequencer state from a checkpoint.
    pub fn restore_from_checkpoint(
        &mut self,
        checkpoint: &crate::storage::checkpoint::SequencerCheckpoint,
    ) {
        for (source_id, watermark) in &checkpoint.watermarks {
            self.update_watermark(source_id, *watermark);
        }
    }

    /// Compute the minimum watermark across all active sources.
    ///
    /// Returns None if any active source has no watermark yet.
    fn compute_min_watermark(&self) -> Option<DateTime<Utc>> {
        let active_sources: Vec<_> = self.sources.values().filter(|s| s.active).collect();

        if active_sources.is_empty() {
            return None;
        }

        // If any active source has no watermark, we can't compute min
        if active_sources.iter().any(|s| s.watermark.is_none()) {
            return None;
        }

        // All active sources have watermarks, return the minimum
        active_sources.iter().filter_map(|s| s.watermark).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_record(source: &str, timestamp: &str, text: &str) -> LogRecord {
        LogRecord {
            id: Uuid::new_v4(),
            timestamp: timestamp.parse().unwrap(),
            source_id: source.to_string(),
            raw_text: text.to_string(),
            file_offset: 0,
        }
    }

    #[test]
    fn test_sequencer_basic_ordering() {
        let mut seq = Sequencer::new(
            vec!["source1".to_string(), "source2".to_string()],
            Duration::from_secs(0),
        );

        // Push records out of order
        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:02Z",
            "source1 line 2",
        ));
        seq.push(make_record(
            "source2",
            "2025-12-04T10:00:01Z",
            "source2 line 1",
        ));
        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:03Z",
            "source1 line 3",
        ));

        // Update watermarks
        // Watermark semantics: T means "I've emitted T, won't emit anything < T"
        // So to make records emittable, set watermark > their timestamp
        seq.update_watermark("source1", "2025-12-04T10:00:04Z".parse().unwrap());
        seq.update_watermark("source2", "2025-12-04T10:00:02Z".parse().unwrap());

        // Emit ready records
        let ready = seq.emit_ready();

        // Min watermark is 10:00:02, so should emit records < 10:00:02
        // That's the record at 10:00:01
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].raw_text, "source2 line 1");

        // Update source2 watermark
        seq.update_watermark("source2", "2025-12-04T10:00:05Z".parse().unwrap());

        // Now should be able to emit the remaining records
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].raw_text, "source1 line 2");
        assert_eq!(ready[1].raw_text, "source1 line 3");
    }

    #[test]
    fn test_no_watermarks_no_emit() {
        let mut seq = Sequencer::new(vec!["source1".to_string()], Duration::from_secs(0));

        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:01Z",
            "source1 line 1",
        ));

        // Without watermark, nothing should be emitted
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 0);
    }

    #[test]
    fn test_safety_margin() {
        let mut seq = Sequencer::new(vec!["source1".to_string()], Duration::from_secs(5));

        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:01Z",
            "source1 line 1",
        ));
        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:03Z",
            "source1 line 2",
        ));

        seq.update_watermark("source1", "2025-12-04T10:00:10Z".parse().unwrap());

        // With safety margin of 5s, threshold is 10:00:10 - 5s = 10:00:05
        // Should emit both records
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn test_mark_source_done() {
        let mut seq = Sequencer::new(
            vec!["source1".to_string(), "source2".to_string()],
            Duration::from_secs(0),
        );

        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:01Z",
            "source1 line 1",
        ));

        // Update source1 watermark but not source2
        seq.update_watermark("source1", "2025-12-04T10:00:10Z".parse().unwrap());

        // Can't emit yet because source2 has no watermark
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 0);

        // Mark source2 as done
        seq.mark_source_done("source2");

        // Now source2 is inactive, so only source1's watermark matters
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 1);
    }

    #[test]
    fn test_all_sources_done() {
        let mut seq = Sequencer::new(
            vec!["source1".to_string(), "source2".to_string()],
            Duration::from_secs(0),
        );

        assert!(!seq.all_sources_done());

        seq.mark_source_done("source1");
        assert!(!seq.all_sources_done());

        seq.mark_source_done("source2");
        assert!(seq.all_sources_done());
    }

    #[test]
    fn test_flush_all() {
        let mut seq = Sequencer::new(vec!["source1".to_string()], Duration::from_secs(0));

        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:03Z",
            "source1 line 3",
        ));
        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:01Z",
            "source1 line 1",
        ));
        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:02Z",
            "source1 line 2",
        ));

        // Flush all should emit in timestamp order
        let all = seq.flush_all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].raw_text, "source1 line 1");
        assert_eq!(all[1].raw_text, "source1 line 2");
        assert_eq!(all[2].raw_text, "source1 line 3");

        // Heap should be empty
        assert_eq!(seq.buffered_count(), 0);
    }

    #[test]
    fn test_multiple_sources_interleaved() {
        let mut seq = Sequencer::new(
            vec![
                "source1".to_string(),
                "source2".to_string(),
                "source3".to_string(),
            ],
            Duration::from_secs(0),
        );

        // Add records from different sources
        seq.push(make_record("source1", "2025-12-04T10:00:01Z", "s1-1"));
        seq.push(make_record("source2", "2025-12-04T10:00:02Z", "s2-1"));
        seq.push(make_record("source3", "2025-12-04T10:00:03Z", "s3-1"));
        seq.push(make_record("source1", "2025-12-04T10:00:04Z", "s1-2"));
        seq.push(make_record("source2", "2025-12-04T10:00:05Z", "s2-2"));

        // Update all watermarks
        seq.update_watermark("source1", "2025-12-04T10:00:04Z".parse().unwrap());
        seq.update_watermark("source2", "2025-12-04T10:00:05Z".parse().unwrap());
        seq.update_watermark("source3", "2025-12-04T10:00:04Z".parse().unwrap());

        // Min watermark is 10:00:04, so should emit < 10:00:04
        // That's records at 10:00:01, 10:00:02, 10:00:03
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 3);
        assert_eq!(ready[0].raw_text, "s1-1");
        assert_eq!(ready[1].raw_text, "s2-1");
        assert_eq!(ready[2].raw_text, "s3-1");

        // Update all watermarks to allow remaining records to be emitted
        seq.update_watermark("source1", "2025-12-04T10:00:05Z".parse().unwrap());
        seq.update_watermark("source2", "2025-12-04T10:00:06Z".parse().unwrap());
        seq.update_watermark("source3", "2025-12-04T10:00:10Z".parse().unwrap());

        // Min watermark is now 10:00:05, so should emit < 10:00:05
        // That's s1-2 at 10:00:04
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].raw_text, "s1-2");

        // Update source1 to allow last record
        seq.update_watermark("source1", "2025-12-04T10:00:10Z".parse().unwrap());

        // Min watermark is now 10:00:06, so should emit < 10:00:06
        // That's s2-2 at 10:00:05
        let ready = seq.emit_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].raw_text, "s2-2");
    }

    #[test]
    fn test_buffered_count() {
        let mut seq = Sequencer::new(vec!["source1".to_string()], Duration::from_secs(0));

        assert_eq!(seq.buffered_count(), 0);

        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:01Z",
            "source1 line 1",
        ));
        assert_eq!(seq.buffered_count(), 1);

        seq.push(make_record(
            "source1",
            "2025-12-04T10:00:02Z",
            "source1 line 2",
        ));
        assert_eq!(seq.buffered_count(), 2);

        seq.update_watermark("source1", "2025-12-04T10:00:10Z".parse().unwrap());
        let _ready = seq.emit_ready();
        assert_eq!(seq.buffered_count(), 0);
    }
}
