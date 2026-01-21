use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sources: HashMap<String, SourceConfig>,
    pub fiber_types: HashMap<String, FiberTypeConfig>,
    #[serde(default = "default_auto_source_fibers")]
    pub auto_source_fibers: bool,
    pub pipeline: PipelineConfig,
    pub sequencer: SequencerConfig,
    pub storage: StorageConfig,
    pub web: WebConfig,
}

fn default_auto_source_fibers() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    #[serde(rename = "type")]
    pub source_type: SourceType,
    pub path: PathBuf,
    pub timestamp: TimestampConfig,
    pub read: ReadConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampConfig {
    pub pattern: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadConfig {
    pub start: ReadStart,
    pub follow: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadStart {
    Beginning,
    End,
    #[serde(rename = "stored_offset")]
    StoredOffset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiberTypeConfig {
    pub description: Option<String>,
    pub temporal: TemporalConfig,
    pub attributes: Vec<AttributeConfig>,
    pub sources: HashMap<String, FiberSourceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalConfig {
    #[serde(with = "duration_format")]
    pub max_gap: Option<Duration>,
    #[serde(default = "default_gap_mode")]
    pub gap_mode: GapMode,
}

fn default_gap_mode() -> GapMode {
    GapMode::Session
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GapMode {
    Session,
    #[serde(rename = "from_start")]
    FromStart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub attr_type: AttributeType,
    #[serde(default)]
    pub key: bool,
    pub derived: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttributeType {
    String,
    Ip,
    Mac,
    Int,
    Float,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiberSourceConfig {
    pub patterns: Vec<PatternConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternConfig {
    pub regex: String,
    #[serde(default)]
    pub release_matching_peer_keys: Vec<String>,
    #[serde(default)]
    pub release_self_keys: Vec<String>,
    #[serde(default)]
    pub close: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub backpressure: BackpressureConfig,
    pub errors: ErrorConfig,
    pub checkpoint: CheckpointConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackpressureConfig {
    pub strategy: BackpressureStrategy,
    #[serde(default = "default_buffer_limit")]
    pub buffer_limit: usize,
}

fn default_buffer_limit() -> usize {
    10000
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackpressureStrategy {
    Block,
    Drop,
    #[serde(rename = "buffer_in_memory")]
    BufferInMemory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorConfig {
    pub on_parse_error: ParseErrorStrategy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParseErrorStrategy {
    Drop,
    Panic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencerConfig {
    #[serde(with = "duration_format")]
    pub batch_epoch_duration: Option<Duration>,
    #[serde(with = "duration_format")]
    pub watermark_safety_margin: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub path: PathBuf,
    pub batch_size: usize,
    pub flush_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    pub listen: String,
}

// Custom serde module for duration parsing
mod duration_format {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => {
                let s = format_duration(*d);
                serializer.serialize_str(&s)
            }
            None => serializer.serialize_str("infinite"),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "infinite" {
            Ok(None)
        } else {
            parse_duration(&s)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }

    fn parse_duration(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty duration string".to_string());
        }

        let (value_str, unit) = if s.ends_with("ms") {
            (&s[..s.len() - 2], "ms")
        } else if s.ends_with('s') {
            (&s[..s.len() - 1], "s")
        } else if s.ends_with('m') {
            (&s[..s.len() - 1], "m")
        } else if s.ends_with('h') {
            (&s[..s.len() - 1], "h")
        } else {
            return Err(format!("invalid duration format: {}", s));
        };

        let value: u64 = value_str
            .parse()
            .map_err(|_| format!("invalid numeric value: {}", value_str))?;

        let duration = match unit {
            "ms" => Duration::from_millis(value),
            "s" => Duration::from_secs(value),
            "m" => Duration::from_secs(value * 60),
            "h" => Duration::from_secs(value * 3600),
            _ => return Err(format!("unknown unit: {}", unit)),
        };

        Ok(duration)
    }

    fn format_duration(d: Duration) -> String {
        let secs = d.as_secs();
        if secs % 3600 == 0 && secs > 0 {
            format!("{}h", secs / 3600)
        } else if secs % 60 == 0 && secs > 0 {
            format!("{}m", secs / 60)
        } else if secs > 0 {
            format!("{}s", secs)
        } else {
            format!("{}ms", d.as_millis())
        }
    }
}
