use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

const CURRENT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Invalid checkpoint version: {0}")]
    InvalidVersion(u32),
}

pub type Result<T> = std::result::Result<T, CheckpointError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub config_version: u64,
    pub sources: HashMap<String, SourceCheckpoint>,
    pub sequencer: SequencerCheckpoint,
    pub fiber_processors: HashMap<String, FiberProcessorCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCheckpoint {
    pub path: PathBuf,
    pub offset: u64,
    pub inode: u64,
    pub last_timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencerCheckpoint {
    pub watermarks: HashMap<String, DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiberProcessorCheckpoint {
    pub open_fibers: Vec<OpenFiberCheckpoint>,
    pub logical_clock: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenFiberCheckpoint {
    pub fiber_id: Uuid,
    pub keys: HashMap<String, String>,
    pub attributes: HashMap<String, serde_json::Value>,
    pub first_activity: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub log_ids: Vec<Uuid>,
}

pub struct CheckpointManager {
    path: PathBuf,
    interval: Duration,
    last_save: Instant,
}

impl CheckpointManager {
    pub fn new(path: impl AsRef<Path>, interval: Duration) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            interval,
            last_save: Instant::now(),
        }
    }

    pub fn load(&self) -> Result<Option<Checkpoint>> {
        if !self.path.exists() {
            tracing::info!("No checkpoint file found at {:?}", self.path);
            return Ok(None);
        }

        tracing::info!("Loading checkpoint from {:?}", self.path);
        let json = std::fs::read_to_string(&self.path)?;
        let checkpoint: Checkpoint = serde_json::from_str(&json)?;

        if checkpoint.version != CURRENT_VERSION {
            tracing::warn!(
                "Checkpoint version mismatch: {} vs {}, ignoring checkpoint",
                checkpoint.version,
                CURRENT_VERSION
            );
            return Ok(None);
        }

        tracing::info!(
            "Loaded checkpoint from {} with config version {}",
            checkpoint.timestamp,
            checkpoint.config_version
        );
        Ok(Some(checkpoint))
    }

    pub fn save(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        let json = serde_json::to_string_pretty(checkpoint)?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write to temp file first for atomic write
        let temp_path = self.path.with_extension("tmp");
        std::fs::write(&temp_path, &json)?;
        std::fs::rename(&temp_path, &self.path)?;

        self.last_save = Instant::now();
        tracing::debug!("Checkpoint saved to {:?}", self.path);
        Ok(())
    }

    pub fn should_save(&self) -> bool {
        self.last_save.elapsed() >= self.interval
    }

    pub fn reset_timer(&mut self) {
        self.last_save = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_checkpoint_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let checkpoint_path = temp_dir.path().join("checkpoint.json");

        let mut manager = CheckpointManager::new(&checkpoint_path, Duration::from_secs(30));

        let checkpoint = Checkpoint {
            version: CURRENT_VERSION,
            timestamp: Utc::now(),
            config_version: 1,
            sources: HashMap::new(),
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::new(),
            },
            fiber_processors: HashMap::new(),
        };

        manager.save(&checkpoint).unwrap();

        let loaded = manager.load().unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.version, CURRENT_VERSION);
        assert_eq!(loaded.config_version, 1);
    }

    #[test]
    fn test_checkpoint_no_file() {
        let temp_dir = TempDir::new().unwrap();
        let checkpoint_path = temp_dir.path().join("nonexistent.json");

        let manager = CheckpointManager::new(&checkpoint_path, Duration::from_secs(30));
        let loaded = manager.load().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_checkpoint_version_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let checkpoint_path = temp_dir.path().join("checkpoint.json");

        let checkpoint = Checkpoint {
            version: 999,
            timestamp: Utc::now(),
            config_version: 1,
            sources: HashMap::new(),
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::new(),
            },
            fiber_processors: HashMap::new(),
        };

        let json = serde_json::to_string_pretty(&checkpoint).unwrap();
        std::fs::write(&checkpoint_path, json).unwrap();

        let manager = CheckpointManager::new(&checkpoint_path, Duration::from_secs(30));
        let loaded = manager.load().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_checkpoint_should_save() {
        let temp_dir = TempDir::new().unwrap();
        let checkpoint_path = temp_dir.path().join("checkpoint.json");

        let mut manager = CheckpointManager::new(&checkpoint_path, Duration::from_millis(100));

        assert!(!manager.should_save());

        std::thread::sleep(Duration::from_millis(150));
        assert!(manager.should_save());

        let checkpoint = Checkpoint {
            version: CURRENT_VERSION,
            timestamp: Utc::now(),
            config_version: 1,
            sources: HashMap::new(),
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::new(),
            },
            fiber_processors: HashMap::new(),
        };

        manager.save(&checkpoint).unwrap();
        assert!(!manager.should_save());
    }

    #[test]
    fn test_checkpoint_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let checkpoint_path = temp_dir.path().join("checkpoint.json");

        let mut manager = CheckpointManager::new(&checkpoint_path, Duration::from_secs(30));

        let checkpoint = Checkpoint {
            version: CURRENT_VERSION,
            timestamp: Utc::now(),
            config_version: 1,
            sources: HashMap::new(),
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::new(),
            },
            fiber_processors: HashMap::new(),
        };

        manager.save(&checkpoint).unwrap();

        // Verify temp file is cleaned up
        let temp_path = checkpoint_path.with_extension("tmp");
        assert!(!temp_path.exists());

        // Verify checkpoint file exists
        assert!(checkpoint_path.exists());
    }
}
