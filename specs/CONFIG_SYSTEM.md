# Noil Configuration System Architecture

## Critical Context for AI Agents

**This configuration system is fundamentally different from typical application configs. Read this entire document before working on config-related tasks.**

## The Key Paradigm: YAML String as Ground Truth

Unlike most applications that treat their configuration file as a simple input to be deserialized into runtime structs, Noil treats the **raw YAML string itself** as the authoritative representation of configuration state.

### Why This Matters

**DO NOT** assume you can:
- Round-trip config through parse → modify structs → serialize
- Generate YAML from scratch when saving config changes
- Normalize or reformat YAML when persisting changes

**YOU MUST**:
- Preserve the exact YAML string, including whitespace and comments
- Edit YAML as text when making configuration changes
- Store the raw YAML string in the database, not just deserialized values

## The Dual Nature of Configuration

Noil maintains configuration in two parallel representations:

### 1. YAML String (Storage Representation)
- **Purpose**: Versioning, diffing, display, persistence
- **Location**: Config file on disk, `config_versions.yaml_content` column in database
- **Characteristics**: Preserves formatting, comments, whitespace
- **Used for**: Hashing (SHA-256), version control, user display, conflict resolution
- **Flow direction**: Input from user → validation → storage (never serialize back)

### 2. Config Structs (Runtime Representation)
- **Purpose**: Application logic, validation, execution
- **Location**: In-memory Rust structs (see `src/config/types.rs`)
- **Characteristics**: Typed, validated, deserialized
- **Used for**: Pipeline operation, fiber processing, web server configuration, **validating YAML changes**
- **Flow direction**: YAML string → deserialize → validate → discard (for validation) OR use (for runtime)

**Critical principle**: You freely deserialize YAML → Config for validation and runtime use, but you NEVER serialize Config → YAML for storage. The transformation is one-way for storage purposes.

## Configuration Sources and Persistence

### Initial Load (First Startup)

**File**: `src/config/parse.rs:32-70` (`load_config_with_yaml()`)

1. Read config file from disk (YAML string)
2. Parse into `Config` struct for validation
3. **Store both**: raw YAML string AND parsed struct
4. Compute SHA-256 hash of YAML string
5. Save to database as `ConfigVersion` with `source = "file"`

**Critical**: Use `load_config_with_yaml()`, NOT `load_config()`, when you need both representations.

### Subsequent Loads (Normal Startup)

**File**: `src/config/reconcile.rs:34-102` (`reconcile_config_on_startup()`)

Noil performs a **git-style 3-way merge** to reconcile:
- Config file content (potentially modified by user)
- Active database config (potentially modified through UI)

**The reconciliation process**:

1. **Check for existing conflicts** - Halt if previous conflict unresolved
2. **Compute hashes** - File hash vs. active DB hash
3. **Four possible states**:

   **a) First run** (`src/config/reconcile.rs:59-91`)
   - No database version exists
   - Import file config as initial version
   - Set as active

   **b) No changes** (`src/config/reconcile.rs:94-96`)
   - File hash == DB hash
   - Continue with current config

   **c) Fast-forward possible** (`src/config/reconcile.rs:217-318`)
   - Only DB changed: Offer to export DB config to file
   - Only file changed: Offer to import file config to DB
   - User chooses to accept or reject

   **d) Diverged - 3-way merge** (`src/config/reconcile.rs:321-495`)
   - Both file and DB changed since last sync
   - Perform line-by-line 3-way merge using last known common ancestor
   - On clean merge: Create merged `ConfigVersion` with `source = "merge"`
   - On conflicts: Write conflict markers to file (git-style) and halt

**Conflict marker format**:
```yaml
<<<<<<< FILE
# Configuration from file
fiber_types:
  my_fiber:
    max_gap: 5s
=======
# Configuration from database
fiber_types:
  my_fiber:
    max_gap: 10s
>>>>>>> DB
```

User must manually resolve, then restart.

### Config Changes from UI

**File**: `src/web/api.rs:581-627` (`update_config()`)

**Current Implementation (MVP)**:
- User sends modified YAML string via `PUT /api/config`
- Validates YAML syntax and structure
- Computes version hash
- **DOES NOT ACTIVATE OR PERSIST** to database
- Returns validation status to user

**Important limitation**: The comment at `src/web/api.rs:622-624` states:
```rust
// For MVP, we validate the config but don't save it to the database.
// User must manually update config file and restart.
```

**Future implementation** (what you should implement when requested):
1. Accept edited YAML string from UI
2. Validate syntax and structure
3. Compute SHA-256 hash
4. Create `ConfigVersion` with `source = "ui"` and `parent_hash = current_active_hash`
5. Save to `config_versions` table with `is_active = true`
6. **DO NOT write to config file** - database becomes source of truth
7. Next startup: reconciliation will detect divergence and offer fast-forward

## Database Schema

**File**: `src/storage/duckdb.rs:202-242`

### `config_versions` Table
```sql
CREATE TABLE config_versions (
    version_hash VARCHAR(64) PRIMARY KEY,     -- SHA-256 of yaml_content
    parent_hash VARCHAR(64),                   -- Previous version (for merge lineage)
    yaml_content TEXT NOT NULL,                -- FULL YAML STRING (not normalized)
    created_at TIMESTAMPTZ NOT NULL,
    source VARCHAR(16) NOT NULL,               -- "file", "ui", or "merge"
    is_active BOOLEAN NOT NULL DEFAULT FALSE   -- Only one active at a time
);
```

**Indexes**:
- `idx_config_versions_created` (created_at DESC)
- `idx_config_versions_parent` (parent_hash)
- `idx_config_versions_active` (is_active)

### `config_state` Table
```sql
CREATE TABLE config_state (
    id INTEGER PRIMARY KEY,              -- Always 1 (singleton)
    has_conflict BOOLEAN,
    conflict_file_path VARCHAR,
    file_version_hash VARCHAR(64),       -- Last known file hash
    db_version_hash VARCHAR(64)          -- Last known DB active hash
);
```

**Purpose**: Tracks reconciliation state between file and database.

## Versioning Strategy

**File**: `src/config/version.rs:16-26`

### Version Hash Computation

```rust
pub fn compute_config_hash(yaml_content: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(yaml_content.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

**Key properties**:
- Deterministic: Same YAML content → same hash
- Sensitive: Any change (even whitespace) → different hash
- Collision-resistant: SHA-256 provides 256-bit security
- Human-readable: 64-character hex string

### Version Lineage

Each `ConfigVersion` records its `parent_hash`, creating a directed acyclic graph (DAG) of configuration evolution:

```
file_v1 → ui_v2 → ui_v3
    ↓
  file_v4 → merge_v5
```

This enables:
- Audit trail of all config changes
- 3-way merge with common ancestor
- Rollback to previous versions
- Comparison across versions

## Usage in Pipeline

**File**: `src/cli/run.rs:109`

```rust
let config_version = compute_config_version(&yaml_content);
```

**Note**: Current code uses deprecated `compute_config_version()` which hashes to `u64`. This will be migrated to use SHA-256 hash string directly.

**Config version is attached to**:
- Every `LogRecord` (`raw_logs.config_version`)
- Every `FiberMembership` (`fiber_memberships.config_version`)
- Every `Fiber` (`fibers.config_version`)

**Why**: Enables reprocessing raw logs with new config rules while preserving old results for comparison.

## Reprocessing with Config Versions

**File**: `src/reprocessing/mod.rs:56-100`

Noil can reprocess raw logs with a different config version:

```rust
run_reprocessing(
    storage,
    new_config,           // Different fiber rules
    new_config_version,   // Different version hash
    time_range,
    clear_old_results,    // Whether to delete old fiber memberships
    state
)
```

**Workflow**:
1. Fetch raw logs (source of truth)
2. Apply new config's fiber rules
3. Create new `Fiber` records with `config_version = new_version`
4. Create new `FiberMembership` records with `config_version = new_version`
5. Optionally delete old results for this config version

**Result**: Same raw logs produce different fiber correlations based on different rules, all versioned and queryable.

## Validation Workflow

**IMPORTANT**: While you must preserve the YAML string for storage, you **should and must** deserialize it to Config structs as a validation step before persisting changes.

### Expected Flow for Config Changes

When the user proposes a config change (via UI or any other mechanism):

1. **Receive** the modified YAML string from the user
2. **Deserialize** to `Config` struct using `serde_yaml::from_str()`
3. **Validate** using the existing validation logic (`src/config/parse.rs:132-399`)
   - Check source references exist
   - Validate regex patterns compile
   - Check for circular dependencies in derived attributes
   - Ensure all pattern actions reference valid keys
4. **If validation fails**: Return error to user with specific message
5. **If validation succeeds**:
   - Compute SHA-256 hash of the **original YAML string** (not re-serialized)
   - Store the **original YAML string** in database
   - Discard the deserialized structs (they were only for validation)

### Example Implementation

**File**: `src/web/api.rs:581-627` shows the current pattern:

```rust
pub async fn update_config(
    State(state): State<Arc<AppState>>,
    body: String,  // Raw YAML string from user
) -> Result<Json<ConfigVersionDto>, (StatusCode, String)> {
    // Step 1: Parse to validate (GOOD!)
    let config: Config = serde_yaml::from_str(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid YAML: {}", e)))?;

    // Step 2: Run full validation (GOOD!)
    validate_config(&config)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Step 3: Hash the ORIGINAL string, not re-serialized (GOOD!)
    let version_hash = compute_config_hash(&body);

    // Step 4: Store the ORIGINAL string (GOOD!)
    storage.insert_config_version(&body, &version_hash, &parent_hash, "ui")?;

    // Config struct is dropped here - it was only for validation
    Ok(...)
}
```

**Key principle**: Deserialization is a validation tool, not a transformation step. The YAML string flows through unchanged.

## Common Pitfalls for AI Agents

### ❌ DON'T: Serialize Config Structs Back to YAML for Storage

```rust
// WRONG - destroys comments and formatting
let config = load_config(path)?;
// ... modify config struct ...
let yaml = serde_yaml::to_string(&config)?;  // BAD: round-tripping loses comments
storage.save_config(&yaml)?;  // Lost all comments and formatting!
```

The problem here is using **serialization as output**. The struct modification + re-serialization flow destroys the human-readable formatting.

### ✅ DO: Deserialize for Validation, But Edit YAML as Text

```rust
// CORRECT - preserves formatting
let yaml_content = fs::read_to_string(path)?;

// Deserialize to validate (GOOD - this is expected!)
let config: Config = serde_yaml::from_str(&yaml_content)?;
validate_config(&config)?;

// But DON'T serialize back - edit the YAML string directly
let modified_yaml = edit_yaml_preserving_structure(&yaml_content, changes)?;

// Validate the modified YAML too
let modified_config: Config = serde_yaml::from_str(&modified_yaml)?;
validate_config(&modified_config)?;

// Store the YAML string, not serialized structs
storage.save_config_version(&modified_yaml, "ui")?;
```

**Remember**: Deserialization is your validation tool. Serialization destroys formatting.

### ❌ DON'T: Write Config to File After UI Changes

```rust
// WRONG - violates the "DB is source of truth after UI edit" principle
fn update_config_from_ui(yaml: String) -> Result<()> {
    let config_path = resolve_config_path(None)?;
    fs::write(config_path, yaml)?;  // NO! Creates divergence
    Ok(())
}
```

### ✅ DO: Save to Database Only

```rust
// CORRECT - database becomes source of truth
fn update_config_from_ui(yaml: String, storage: &Storage) -> Result<String> {
    let hash = compute_config_hash(&yaml);
    let parent = storage.get_active_config_version()?.version_hash;
    storage.insert_config_version(&yaml, &hash, &parent, "ui")?;
    storage.mark_config_active(&hash)?;
    Ok(hash)
}
```

### ❌ DON'T: Ignore Reconciliation Results

```rust
// WRONG - may proceed with stale config
let config = load_config(path)?;
run_application(config)?;  // What if DB has newer version?
```

### ✅ DO: Always Reconcile at Startup

```rust
// CORRECT - handles divergence before proceeding
let config_path = resolve_config_path(args.config)?;
let storage = DuckDbStorage::new(&storage_path).await?;

match reconcile_config_on_startup(&config_path, &storage).await? {
    ReconcileResult::NoChange(hash) => {
        let yaml = storage.get_config_version(&hash)?.yaml_content;
        let config = parse_config(&yaml)?;
        run_application(config, hash)?;
    }
    ReconcileResult::Conflict { path } => {
        eprintln!("Conflict markers written to: {}", path);
        eprintln!("Resolve conflicts and restart.");
        std::process::exit(1);
    }
    // ... handle other cases
}
```

## File References for Config Code

| Component | File | Lines | Purpose |
|-----------|------|-------|---------|
| Path resolution | `src/config/mod.rs` | 37-58 | Resolves `~/.config/noil/config.yml`, `/etc/noil/config.yml` |
| YAML loading | `src/config/parse.rs` | 32-70 | `load_config_with_yaml()` - returns YAML + struct |
| Validation | `src/config/parse.rs` | 132-399 | Validates sources, regexes, derived attrs |
| Config structs | `src/config/types.rs` | 1-420 | Deserialized Rust types (capability-based, no mode enum) |
| Versioning | `src/config/version.rs` | 16-26 | SHA-256 hash computation |
| Reconciliation | `src/config/reconcile.rs` | 34-495 | 3-way merge logic |
| Diff generation | `src/config/diff.rs` | 1-163 | Unified diff for display |
| Database storage | `src/storage/duckdb.rs` | 202-242, 812-901 | Schema + CRUD operations |
| UI endpoints | `src/web/api.rs` | 490-658 | HTTP API for config management |
| Startup integration | `src/cli/run.rs` | 49-145 | Orchestrates reconciliation |
| Reprocessing | `src/reprocessing/mod.rs` | 56-100 | Config-versioned reprocessing |

## When Working on Config Tasks

Before implementing any config-related feature, ask yourself:

1. **Am I validating the config properly?**
   - DO deserialize YAML to Config structs for validation
   - DO run `validate_config()` to check semantics
   - DON'T serialize structs back to YAML for storage

2. **Am I preserving the YAML string?**
   - If you're loading config, use `load_config_with_yaml()`
   - If you're modifying config, edit the YAML string, don't serialize structs
   - Store the original/edited YAML string, not re-serialized output

3. **Am I computing the correct version hash?**
   - Hash the YAML string with `compute_config_hash()`
   - Store the hash with the YAML in `config_versions`

4. **Am I handling reconciliation?**
   - On startup, call `reconcile_config_on_startup()`
   - Handle conflicts by writing markers and halting

5. **Am I tracking lineage?**
   - Set `parent_hash` to the current active version
   - Set `source` to "file", "ui", or "merge" appropriately

6. **Am I maintaining the single active version invariant?**
   - Only one config version has `is_active = true`
   - Use `mark_config_active()` to atomically update

## Summary

Noil's configuration system is designed for:
- **Version control**: Git-style merging and conflict resolution
- **Auditability**: Full history of config changes with lineage
- **Reprocessability**: Reprocess logs with new rules while keeping old results
- **User experience**: Preserve formatting and comments in config files

The unusual aspects exist to support these goals. When in doubt, prioritize preserving the YAML string over convenience.
