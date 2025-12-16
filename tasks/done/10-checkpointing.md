# 10: Checkpointing

Implement checkpoint save/restore for crash recovery.

## Location

`src/storage/checkpoint.rs`

## Checkpoint Contents

```rust
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,  // Schema version for migrations
    pub timestamp: DateTime<Utc>,
    pub config_version: u64,
    pub sources: HashMap<String, SourceCheckpoint>,
    pub sequencer: SequencerCheckpoint,
    pub fiber_processors: HashMap<String, FiberProcessorCheckpoint>,
}

#[derive(Serialize, Deserialize)]
pub struct SourceCheckpoint {
    pub path: PathBuf,
    pub offset: u64,
    pub inode: u64,  // To detect file rotation
    pub last_timestamp: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize)]
pub struct SequencerCheckpoint {
    pub watermarks: HashMap<String, DateTime<Utc>>,
}

#[derive(Serialize, Deserialize)]
pub struct FiberProcessorCheckpoint {
    pub open_fibers: Vec<OpenFiberCheckpoint>,
    pub logical_clock: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct OpenFiberCheckpoint {
    pub fiber_id: Uuid,
    pub keys: HashMap<String, String>,
    pub attributes: HashMap<String, serde_json::Value>,
    pub first_activity: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
}
```

## Checkpoint Manager

```rust
pub struct CheckpointManager {
    path: PathBuf,
    interval: Duration,
    last_save: Instant,
}

impl CheckpointManager {
    pub fn new(config: &CheckpointConfig) -> Self;

    pub fn load(&self) -> Result<Option<Checkpoint>>;

    pub fn save(&mut self, checkpoint: &Checkpoint) -> Result<()>;

    pub fn should_save(&self) -> bool {
        self.last_save.elapsed() >= self.interval
    }
}
```

## Save Implementation

```rust
pub fn save(&mut self, checkpoint: &Checkpoint) -> Result<()> {
    let json = serde_json::to_string_pretty(checkpoint)?;

    // Write to temp file first (atomic write)
    let temp_path = self.path.with_extension("tmp");
    std::fs::write(&temp_path, &json)?;
    std::fs::rename(&temp_path, &self.path)?;

    self.last_save = Instant::now();
    tracing::debug!("Checkpoint saved to {:?}", self.path);
    Ok(())
}
```

## Load Implementation

```rust
pub fn load(&self) -> Result<Option<Checkpoint>> {
    if !self.path.exists() {
        return Ok(None);
    }

    let json = std::fs::read_to_string(&self.path)?;
    let checkpoint: Checkpoint = serde_json::from_str(&json)?;

    // Version check
    if checkpoint.version != CURRENT_VERSION {
        tracing::warn!(
            "Checkpoint version mismatch: {} vs {}, ignoring",
            checkpoint.version,
            CURRENT_VERSION
        );
        return Ok(None);
    }

    Ok(Some(checkpoint))
}
```

## Integration Points

### Source Reader

On restore:
```rust
if let Some(source_cp) = checkpoint.sources.get(&self.source_id) {
    // Check if file is the same (inode match)
    let metadata = std::fs::metadata(&self.path)?;
    if metadata.ino() == source_cp.inode {
        self.seek_to(source_cp.offset)?;
    } else {
        tracing::warn!("File rotated since checkpoint, starting from beginning");
    }
}
```

### Sequencer

On restore:
```rust
for (source_id, watermark) in &checkpoint.sequencer.watermarks {
    self.update_watermark(source_id, *watermark);
}
```

### Fiber Processor

On restore:
```rust
for fiber_cp in &checkpoint.open_fibers {
    let fiber = OpenFiber {
        fiber_id: fiber_cp.fiber_id,
        keys: fiber_cp.keys.clone(),
        // ... reconstruct from checkpoint
    };
    self.open_fibers.insert(fiber.fiber_id, fiber);

    // Rebuild key index
    for (key_name, value) in &fiber.keys {
        self.key_index.insert((key_name.clone(), value.clone()), fiber.fiber_id);
    }
}
```

### Pipeline Integration

Periodic checkpoint during processing:

```rust
// In processor task
let mut checkpoint_mgr = CheckpointManager::new(&config.checkpoint);

loop {
    // ... process logs ...

    if checkpoint_mgr.should_save() {
        let checkpoint = Checkpoint {
            version: CURRENT_VERSION,
            timestamp: Utc::now(),
            sources: collect_source_checkpoints(&readers),
            sequencer: sequencer.checkpoint(),
            fiber_processors: processor.checkpoint(),
            config_version,
        };
        checkpoint_mgr.save(&checkpoint)?;
    }
}
```

## Config Version Mismatch

If checkpoint's config_version differs from current config:
- Log warning
- Option 1: Ignore checkpoint, start fresh (safe)
- Option 2: Use checkpoint anyway (may cause inconsistencies)

For MVP, start fresh on config change. Future: smarter migration.

## Recovery Semantics

After crash and restart with checkpoint:
- Some logs may be reprocessed (between last checkpoint and crash)
- Fiber memberships are idempotent (same log_id + fiber_id = no-op)
- Fiber state may diverge slightly (last_activity might differ)

This is acceptable for MVP. Future: use write-ahead log for stricter guarantees.

## Acceptance Criteria

- Checkpoint file created at configured path
- Checkpoint written at configured interval
- Atomic write (no corruption on crash during write)
- Restart resumes from checkpoint
- File rotation detected (inode change)
- Config version change handled gracefully
- Invalid/corrupt checkpoint handled (start fresh)
