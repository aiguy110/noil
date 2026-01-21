use super::traits::{Storage, StorageError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const CURRENT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Invalid checkpoint version: {0}")]
    InvalidVersion(u32),
}

pub type Result<T> = std::result::Result<T, CheckpointError>;

/// Runtime state for a source reader that can be safely shared across threads
/// for checkpoint collection without blocking the reader
#[derive(Debug, Default)]
pub struct SourceCheckpointState {
    pub offset: u64,
    pub inode: u64,
    pub last_timestamp: Option<DateTime<Utc>>,
}

/// Shared reference to source checkpoint state
pub type SharedSourceState = Arc<Mutex<SourceCheckpointState>>;

/// Shared reference to fiber processor checkpoint state
pub type SharedFiberProcessorState = Arc<Mutex<HashMap<String, FiberProcessorCheckpoint>>>;

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
    storage: Arc<dyn Storage>,
    interval: Duration,
    last_save: Instant,
}

impl CheckpointManager {
    pub fn new(storage: Arc<dyn Storage>, interval: Duration) -> Self {
        Self {
            storage,
            interval,
            last_save: Instant::now(),
        }
    }

    pub async fn load(&self) -> Result<Option<Checkpoint>> {
        tracing::info!("Loading checkpoint from storage");

        let checkpoint_opt = self.storage.load_checkpoint().await?;

        if let Some(checkpoint) = checkpoint_opt {
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
        } else {
            tracing::info!("No checkpoint found in storage");
            Ok(None)
        }
    }

    pub async fn save(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        self.storage.save_checkpoint(checkpoint).await?;
        self.last_save = Instant::now();
        tracing::debug!("Checkpoint saved to storage");
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
    use crate::storage::duckdb::DuckDbStorage;

    async fn setup_storage() -> Arc<dyn Storage> {
        let storage = DuckDbStorage::in_memory().unwrap();
        storage.init_schema().await.unwrap();
        Arc::new(storage) as Arc<dyn Storage>
    }

    #[tokio::test]
    async fn test_checkpoint_save_load() {
        let storage = setup_storage().await;
        let mut manager = CheckpointManager::new(storage.clone(), Duration::from_secs(30));

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

        manager.save(&checkpoint).await.unwrap();

        let loaded = manager.load().await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.version, CURRENT_VERSION);
        assert_eq!(loaded.config_version, 1);
    }

    #[tokio::test]
    async fn test_checkpoint_no_checkpoint() {
        let storage = setup_storage().await;
        let manager = CheckpointManager::new(storage, Duration::from_secs(30));
        let loaded = manager.load().await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_version_mismatch() {
        let storage = setup_storage().await;

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

        // Save directly to storage to bypass CheckpointManager version check
        storage.save_checkpoint(&checkpoint).await.unwrap();

        let manager = CheckpointManager::new(storage, Duration::from_secs(30));
        let loaded = manager.load().await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_should_save() {
        let storage = setup_storage().await;
        let mut manager = CheckpointManager::new(storage, Duration::from_millis(100));

        assert!(!manager.should_save());

        tokio::time::sleep(Duration::from_millis(150)).await;
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

        manager.save(&checkpoint).await.unwrap();
        assert!(!manager.should_save());
    }

    #[tokio::test]
    async fn test_checkpoint_round_trip() {
        let storage = setup_storage().await;
        let mut manager = CheckpointManager::new(storage.clone(), Duration::from_secs(30));

        // Create a more complex checkpoint with data
        let mut sources = HashMap::new();
        sources.insert(
            "source1".to_string(),
            SourceCheckpoint {
                path: PathBuf::from("/var/log/test.log"),
                offset: 12345,
                inode: 67890,
                last_timestamp: Some(Utc::now()),
            },
        );

        let checkpoint = Checkpoint {
            version: CURRENT_VERSION,
            timestamp: Utc::now(),
            config_version: 42,
            sources,
            sequencer: SequencerCheckpoint {
                watermarks: HashMap::from([("source1".to_string(), Utc::now())]),
            },
            fiber_processors: HashMap::new(),
        };

        manager.save(&checkpoint).await.unwrap();

        let loaded = manager.load().await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.version, CURRENT_VERSION);
        assert_eq!(loaded.config_version, 42);
        assert_eq!(loaded.sources.len(), 1);
        assert_eq!(loaded.sources.get("source1").unwrap().offset, 12345);
    }
}
