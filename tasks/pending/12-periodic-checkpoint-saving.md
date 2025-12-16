# 12: Periodic Checkpoint Saving

Implement periodic checkpoint saving with source reader state.

## Priority

MEDIUM - Needed for crash recovery reliability.

## Current State

- CheckpointManager exists (`src/storage/checkpoint.rs`)
- Checkpoint loaded and restored on startup (`src/cli/run.rs:69-96`)
- Source readers have `checkpoint_offset()` method
- **Missing**: Periodic saving during runtime

Current code note at `src/cli/run.rs:202-203`:
```rust
// Note: For periodic checkpoint saving, we would need to capture source state here
// This is deferred to a future iteration when we refactor to use shared state
```

## Problem

Source readers are moved into the sequencer task and not accessible from the main pipeline loop. Need a way to capture source state for checkpoints.

## Implementation Approach

Use `Arc<Mutex<>>` wrapper for source reader state that needs checkpointing.

### Shared Checkpoint State

```rust
// src/storage/checkpoint.rs (add)
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct SourceCheckpointState {
    pub offset: u64,
    pub inode: u64,
    pub last_timestamp: Option<DateTime<Utc>>,
}

pub type SharedSourceState = Arc<Mutex<SourceCheckpointState>>;
```

### Modify SourceReader

```rust
// src/source/reader.rs
impl SourceReader {
    pub fn with_shared_state(mut self) -> (Self, SharedSourceState) {
        let state = Arc::new(Mutex::new(SourceCheckpointState {
            offset: self.last_emitted_offset,
            inode: self.file_inode.unwrap_or(0),
            last_timestamp: self.last_watermark,
        }));
        self.shared_state = Some(state.clone());
        (self, state)
    }

    // Call this after each emit in poll_next()
    fn update_shared_state(&self) {
        if let Some(ref state) = self.shared_state {
            if let Ok(mut guard) = state.lock() {
                guard.offset = self.last_emitted_offset;
                guard.inode = self.file_inode.unwrap_or(0);
                guard.last_timestamp = self.last_watermark;
            }
        }
    }
}
```

### Main Pipeline Integration

```rust
// src/cli/run.rs
let mut shared_states: HashMap<String, SharedSourceState> = HashMap::new();
let mut readers_with_state = Vec::new();

for reader in readers {
    let source_id = reader.source_id().to_string();
    let (reader, state) = reader.with_shared_state();
    shared_states.insert(source_id, state);
    readers_with_state.push(reader);
}

// Start checkpoint save task
if config.pipeline.checkpoint.enabled {
    let checkpoint_path = config.pipeline.checkpoint.path.clone();
    let interval = config.pipeline.checkpoint.interval_seconds;
    let states = shared_states.clone();

    tokio::spawn(async move {
        let mut mgr = CheckpointManager::new(&checkpoint_path, Duration::from_secs(interval));
        let mut interval = tokio::time::interval(Duration::from_secs(interval));

        loop {
            interval.tick().await;
            let checkpoint = build_checkpoint(&states, config_version);
            if let Err(e) = mgr.save(&checkpoint) {
                tracing::error!("Failed to save checkpoint: {}", e);
            }
        }
    });
}
```

## Files to Modify

1. `src/storage/checkpoint.rs`: Add `SharedSourceState` type
2. `src/source/reader.rs`: Add shared state support
3. `src/cli/run.rs`: Wire up shared states and spawn checkpoint task

## Acceptance Criteria

- [ ] Source state captured in shared state on each emit
- [ ] Checkpoint task runs at configured interval
- [ ] Checkpoint includes all source offsets
- [ ] Recovery uses saved offsets (already works)
- [ ] No deadlocks or significant lock contention
