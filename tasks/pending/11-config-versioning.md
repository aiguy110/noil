# 11: Configuration Versioning System

Implement config versioning to replace hardcoded `1u64` throughout the codebase.

## Priority

HIGH - Foundational to reprocessing feature.

## Current State

Config version is hardcoded in two places:
- `src/cli/run.rs:66`: `let config_version = 1u64;`
- `src/pipeline/runner.rs:69`: `let config_version = 1u64;`

## Requirements

1. Generate unique version ID when config is loaded
2. Propagate version through entire pipeline
3. Store version in database with logs and fiber memberships
4. Support querying by config version

## Implementation

### Version Generation Strategy

Use content hash of config file for deterministic versioning:

```rust
// src/config/version.rs
use std::path::Path;

pub fn compute_config_version(config_path: &Path) -> Result<u64, std::io::Error> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let content = std::fs::read_to_string(config_path)?;
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    Ok(hasher.finish())
}
```

### Integration in run.rs

Replace hardcoded version:

```rust
// src/cli/run.rs
use crate::config::version::compute_config_version;

async fn run_pipeline(config_path: &PathBuf) -> Result<(), RunError> {
    let config = load_config(config_path)?;
    let config_version = compute_config_version(config_path)
        .map_err(|e| RunError::Config(ConfigError::Io(e)))?;

    info!(config_version = config_version, "Loaded config");
    // ... rest unchanged
}
```

### Pipeline Propagation

The config_version is already passed to:
- `DuckDbStorage::new(&path, config_version)`
- `FiberProcessor::from_config(&config, config_version)`
- `StoredLog { config_version, ... }`
- `FiberMembership { config_version, ... }`

No changes needed to downstream code.

## Files to Modify

1. Create `src/config/version.rs`
2. Update `src/config/mod.rs` to export version module
3. Update `src/cli/run.rs:66` to call `compute_config_version`
4. Update `src/pipeline/runner.rs:69` (or remove if not needed - check if this duplicates run.rs)

## Acceptance Criteria

- [ ] Config version computed from file content hash
- [ ] Same config file produces same version
- [ ] Different config produces different version
- [ ] Version logged at startup
- [ ] Version stored with all logs and memberships
- [ ] Checkpoint validates config version on restore (already implemented)
