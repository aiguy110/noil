# TODO: Deferred Work

This document tracks all deferred work, incomplete implementations, and areas requiring future development in the Noil codebase.

## High Priority

### 1. Configuration Versioning System

**Status:** Not implemented (hardcoded to version 1)

**Locations:**
- `src/cli/run.rs:65`
- `src/pipeline/runner.rs:69`

**Description:**
Configuration versioning is currently hardcoded to `1u64` throughout the codebase. A proper versioning system needs to be implemented to support the config versioning semantics described in CLAUDE.md.

**Impact:**
- Config versioning is foundational to the reprocessing feature
- Each log record and fiber membership carries a `config_version` field
- Enables processing in-flight logs with original config semantics
- Allows reprocessing historical logs with new rules
- Enables comparing results across config versions

**Requirements:**
- Implement version tracking when config is loaded/changed
- Generate unique version IDs (could be sequential, timestamp-based, or content hash)
- Propagate version through the entire pipeline
- Store version associations in database
- Support querying by config version

**Code References:**
```rust
// src/cli/run.rs:65
// TODO: Implement proper config versioning
let config_version = 1u64;

// src/pipeline/runner.rs:69
let config_version = 1u64; // TODO: Get from config versioning system
```

---

## Medium Priority

### 2. Checkpoint Source State Management

**Status:** Partially implemented, needs refactoring

**Location:** `src/cli/run.rs:202-203`

**Description:**
Periodic checkpoint saving of source reader state needs refactoring to use shared state instead of the current independent task spawning approach.

**Current Note:**
```rust
// Note: For periodic checkpoint saving, we would need to capture source state here
// This is deferred to a future iteration when we refactor to use shared state
```

**Impact:**
- Checkpoint management is incomplete
- Source reader state (file offsets, inodes, latest timestamps) not being periodically saved
- May lose progress on restart if crash occurs between checkpoints

**Requirements:**
- Design shared state mechanism for source readers
- Implement periodic checkpoint timer
- Capture complete source state (file path, offset, inode, latest timestamp)
- Coordinate checkpoint writes with sequencer watermark state
- Handle checkpoint restoration on startup

---

## Low Priority

### 3. Web Server Graceful Shutdown

**Status:** Incomplete implementation

**Location:** `src/cli/run.rs:283`

**Description:**
The web server shutdown process is not graceful - it simply aborts the task rather than properly signaling shutdown and waiting for cleanup.

**Current Implementation:**
```rust
// Note: web server doesn't gracefully shutdown yet, so we just abort it
web_handle.abort();
```

**Impact:**
- Web server resources may not be cleaned up properly on shutdown
- In-flight requests may be dropped abruptly
- Minimal practical impact for current use case

**Requirements:**
- Implement shutdown signaling mechanism for web server
- Wait for in-flight requests to complete (with timeout)
- Clean up resources before terminating
- Consider using tokio's cancellation tokens or axum's graceful shutdown support

---

## Future Enhancements (Not Currently Blocking)

The following items are mentioned in CLAUDE.md as "Future" or "Not MVP" but have no corresponding TODO markers in the code:

### 4. Hot Configuration Reload

**Status:** Not implemented, but architecture doesn't preclude it

**Description:**
First iteration does not support hot reload, but the design has been built to not preclude future implementation:
- No global/static config singletons
- Components receive config via explicit parameters
- Logs and fibers reference config by version ID
- Sequencer supports add/remove source operations
- Fiber processor rules abstracted behind trait

**Future Work:**
- Implement config file watching
- Design config transition strategy
- Handle in-flight logs with old config
- Coordinate config changes across pipeline components
- Update UI to reflect config changes

### 5. Additional Subcommands

**Status:** Not implemented (marked as "Not MVP" in CLAUDE.md)

**Planned Subcommands:**
- `noil config validate` — Check config file for errors
- `noil reprocess` — Re-run fiber processing on stored raw logs
- `noil status` — Show pipeline health and statistics

### 6. Distributed Deployment

**Status:** Architecture designed, not implemented

**Description:**
CLAUDE.md describes hierarchical sequencing for distributed deployment:
- Remote Agents on remote machines
- Site Sequencers for local aggregation
- Central Sequencer for global ordering
- Pull-based batch model with epoch-based batching

**Future Work:**
- Implement remote agent mode
- Design batch/epoch protocol
- Implement hierarchical sequencer
- Add network layer with retry/backoff
- Design distributed checkpoint coordination

### 7. Alternative Storage Backends

**Status:** Abstraction in place, only DuckDB implemented

**Description:**
Storage is abstracted behind a trait to allow future backends (e.g., ClickHouse for large-scale deployments). Currently only DuckDB is implemented.

**Future Work:**
- Implement ClickHouse backend for high-scale deployments
- Consider PostgreSQL backend for familiarity/tooling
- Add backend selection to config
- Benchmark and optimize for different backends

### 8. Web UI

**Status:** Basic API endpoints exist, no frontend implementation

**Requirements (from CLAUDE.md):**
- Display all log lines in a fiber in chronological order
- Colorized visual delineation between logs from different sources
- For any log line, show other fibers it belongs to
- Navigate between fibers
- Back-tracking history of visited fibers

**Current State:**
- API endpoints implemented in `src/web/api.rs`
- No frontend assets in `src/web/assets/`
- Server infrastructure in place via axum

---

## Summary

**Critical Path Items:**
1. Configuration versioning system (HIGH)
2. Checkpoint source state management (MEDIUM)

**Quality Improvements:**
3. Web server graceful shutdown (LOW)

**Future Expansion:**
4. Hot configuration reload
5. Additional CLI subcommands
6. Distributed deployment
7. Alternative storage backends
8. Web UI implementation

The codebase is largely complete for MVP functionality. The primary blockers for production use are configuration versioning (needed for reprocessing) and proper checkpoint management (needed for reliability).
