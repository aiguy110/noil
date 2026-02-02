use crate::source::reader::LogRecord;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogBatch {
    /// Unique batch ID (for deduplication)
    pub batch_id: Uuid,

    /// ID of collector that created this batch
    pub collector_id: String,

    /// Epoch information (time window, watermark)
    pub epoch: EpochInfo,

    /// Log records in this batch (sorted by timestamp)
    pub logs: Vec<LogRecord>,

    /// Config version used when reading these logs
    pub config_version: u64,

    /// Monotonic sequence number for this collector
    /// Starts at 0, increments by 1 per batch
    pub sequence_num: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochInfo {
    /// Epoch start timestamp (inclusive)
    pub start: DateTime<Utc>,

    /// Epoch end timestamp (exclusive)
    pub end: DateTime<Utc>,

    /// Watermark: all logs in this batch have timestamp < watermark
    /// AND no future logs from this collector will have timestamp < watermark
    /// Used by parent for hierarchical sequencing
    pub watermark: DateTime<Utc>,

    /// Rewind generation (for watermark consistency after rewind)
    pub generation: u64,
}
