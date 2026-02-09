# Noil

Noil is a log correlation system that ingests logs from multiple sources, sequences them into global timestamp order, and groups them into "fibers" based on configurable rules. Named after the short, tangled fibers combed out during textile processing.

## Core Concepts

### Fibers

A fiber is a group of log lines that are related by shared attributes and temporal proximity. A single log line may belong to multiple fibers. Fiber membership is computed at ingestion time based on fiber type rules.

### Fiber Types

A fiber type is a template that defines:

- **Temporal constraint**: Maximum time gap between consecutive logs (session windowing) or from the first log
- **Attributes**: Named values extracted from logs or derived via interpolation; persist on fiber after close
- **Keys**: Attributes marked with `key: true`; used for fiber matching/merging while open, released on close
- **Source patterns**: Per-source regex patterns that filter logs and extract attributes

Each fiber is identified by a UUID assigned at creation. Keys enable matching and merging: when a log's keys match an open fiber, it joins that fiber; when keys match multiple open fibers of the same type, those fibers merge. Fibers of different types are never merged and are processed completely independently (enabling parallelization). Keys only exist while a fiber is open—when closed, keys are released but attributes persist.

Note: `max_gap` may be `infinite` (never closes due to time). One use case: each source can have its own never-closing fiber that serves as a jumping-off point for navigation in the UI.

### Sources

A source is a log file (or future: other log origins) with configuration for:

- Timestamp extraction pattern and format
- File path
- Read mode (start position, follow behavior)

Multiline handling: if a line doesn't match the timestamp pattern, it's treated as a continuation of the previous line.

## Binary and CLI

Noil ships as a single `noil` binary containing sequencer, fiber processor, storage writer, and web server with embedded frontend assets.

### Configuration File Location

The application uses a single `config.yml` file. Resolution order:

1. `--config <path>` argument (if provided)
2. `~/.config/noil/config.yml`
3. `/etc/noil/config.yml`

### Subcommands

**`noil run`** (or just `noil`): Start the application. Requires valid config.

```bash
noil                              # Use default config locations
noil --config ./my-config.yml     # Use specific config
noil run --config ./my-config.yml # Explicit run subcommand
```

**`noil config init`**: Generate starter configuration file with comprehensive comments.

```bash
noil config init           # Write to /etc/noil/config.yml, fallback to ~/.config/noil/config.yml
noil config init --stdout  # Print to stdout instead of writing file
```

The generated config file should be heavily commented, explaining all available settings, their defaults, and example values. This serves as living documentation.

### Future Subcommands (Not MVP)

- `noil config validate` — Check config file for errors
- `noil reprocess` — Re-run fiber processing on stored raw logs
- `noil status` — Show pipeline health and statistics

## Architecture

### Two-Phase Design

**Phase 1: Capture** — Get logs from sources into durable raw log storage, preserving global timestamp order.

**Phase 2: Process** — Apply fiber rules to captured logs. Can be re-run with new rules without re-reading source files.

### Pipeline Components

```
Source Readers → Sequencer → Raw Log Store → Fiber Processor → Fiber Memberships
```

**Source Reader**: Reads log files, extracts timestamps, coalesces multiline records, tracks file offsets for checkpointing.

**Sequencer**: Merges multiple source streams into global timestamp order using a min-heap. Emits logs (or batches) only when safe based on watermarks from all sources.

**Raw Log Store**: Durable storage (DuckDB initially; storage backend is abstracted for future flexibility). Source of truth for reprocessing.

**Fiber Processor**: Evaluates fiber type rules against each log, manages active fiber sessions, computes fiber membership.

**Storage Writer**: Batched writes to storage for both raw logs and fiber memberships.

### Distributed Deployment

For logs on remote machines, Noil supports hierarchical sequencing:

```
Remote Agents → Site Sequencers → Central Sequencer → Raw Log Store
```

**Agent**: Runs on remote machine, reads local logs, sequences locally, batches into epochs, serves batches to parent sequencer on request.

**Hierarchical Sequencers**: Parent sequencers merge streams from children using watermark-based coordination. Pull-based model: parent requests batches when ready, children buffer locally.

**Epoch-based batching**: Instead of perfect per-record ordering, sequencers emit batches covering time epochs with the guarantee that all logs in batch N have timestamps ≤ all logs in batch N+1.

## Configuration

All configuration lives in a single `config.yml` file. Noil uses a capability-based configuration model — capabilities are enabled by which config sections are present:

- `sources`: Read local log files (optional)
- `remote_collectors`: Pull logs from remote Noil instances (optional)
- `collector`: Serve batched logs to other Noil instances (optional)
- `fiber_types`: Enable log storage and fiber processing (optional; presence controls storage)

At least one input (`sources` or `remote_collectors`) must be configured.

**⚠️ IMPORTANT FOR AI AGENTS**: Noil's configuration system is fundamentally different from typical application configs. The raw YAML string (with comments and formatting) is the ground truth, NOT the deserialized structs. Configuration changes from the UI are persisted ONLY to the database, and startup reconciliation uses git-style 3-way merging. **See [specs/CONFIG_SYSTEM.md](specs/CONFIG_SYSTEM.md) for critical details before working on any config-related task.**

### Example Configuration

```yaml
# =============================================================================
# NOIL CONFIGURATION
# =============================================================================
# This file configures log sources, fiber correlation rules, and system settings.
# 
# Config file locations (in order of precedence):
#   1. Path specified via --config argument
#   2. ~/.config/noil/config.yml
#   3. /etc/noil/config.yml

# =============================================================================
# SOURCES
# =============================================================================
# Define log files to ingest. Each source needs a unique ID and timestamp config.

sources:
  nginx_access:
    type: file
    path: /var/log/nginx/access.log
    timestamp:
      # Regex must contain a named capture group 'ts' for the timestamp
      pattern: '^\[(?P<ts>[^\]]+)\]'
      # Format: strptime format string, 'iso8601', 'epoch', or 'epoch_ms'
      format: '%d/%b/%Y:%H:%M:%S %z'
    read:
      # Where to start reading: 'beginning', 'end', or 'stored_offset'
      start: beginning
      # Whether to continue watching for new lines after EOF
      follow: true
      
  application_log:
    type: file
    path: /var/log/app/app.log
    timestamp:
      pattern: '^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)'
      format: iso8601
    read:
      start: beginning
      follow: true

# =============================================================================
# REMOTE COLLECTORS (optional)
# =============================================================================
# Pull logs from remote Noil instances that have collector serving enabled.
# Omit this section if this instance only reads local files.

# remote_collectors:
#   endpoints:
#     - id: node1
#       url: http://10.0.0.1:7104
#       retry_interval: 5s
#       timeout: 30s
#   poll_interval: 1s
#   backpressure:
#     strategy: block
#     buffer_limit: 10000

# =============================================================================
# COLLECTOR SERVING (optional)
# =============================================================================
# Serve batched logs to other Noil instances via /collector/* HTTP API.
# Requires local sources. Omit if this instance does not serve logs.

# collector:
#   epoch_duration: 10s
#   buffer:
#     max_epochs: 100
#     strategy: block
#   checkpoint:
#     enabled: true
#     interval_seconds: 30

# =============================================================================
# FIBER TYPES (optional)
# =============================================================================
# Define rules for correlating logs into fibers. Presence of this section
# (even if empty) enables log storage and fiber processing. Each fiber type specifies:
#   - Temporal constraints (max time gap between related logs)
#   - Attributes (extracted from logs or derived via interpolation)
#   - Keys (attributes used for fiber matching/merging)
#   - Per-source patterns for matching and extraction

fiber_types:
  request_trace:
    description: "Traces requests across program1 and program2"
    temporal:
      # Maximum gap between consecutive logs in a fiber
      # Use 'infinite' for fibers that never close due to time
      max_gap: 5s
      # 'session': gap measured between consecutive logs
      # 'from_start': all logs must be within max_gap of first log
      gap_mode: session
      
    attributes:
      # Extracted attributes: captured via regex named groups
      - name: mac
        type: mac         # string, ip, mac, int, float
        key: true         # This attribute is a key for fiber matching
        
      - name: program1_thread
        type: string
        key: true
        
      - name: program2_thread
        type: string
        key: true
        
      - name: ip
        type: ip
        # key: false is default — captured but not used for matching
        
      - name: src_port
        type: int
        
      - name: dst_port
        type: int
        
      # Derived attributes: computed via interpolation from other attributes
      # Only defined when all referenced attributes have values
      - name: connection
        type: string
        key: true
        derived: "${ip}:${src_port}->${dst_port}"
        
    sources:
      program1:
        patterns:
          # release_matching_peer_keys: for each listed key extracted by this
          # pattern, remove that (key, value) from OTHER open fibers first
          - regex: 'thread-(?P<program1_thread>\d+) Received.*from (?P<ip>\d+\.\d+\.\d+\.\d+)'
            release_matching_peer_keys: [program1_thread]
            
          - regex: 'thread-(?P<program1_thread>\d+).*MAC (?P<mac>[0-9a-f:]+)'
          - regex: 'thread-(?P<program1_thread>\d+)'
          
      program2:
        patterns:
          - regex: 'thread-(?P<program2_thread>\d+).*MAC (?P<mac>[0-9a-f:]+)'
          
          # release_self_keys: remove these keys from THIS fiber after processing
          # (value not needed — removes by key name)
          # close: close the fiber after processing (releases all keys)
          - regex: 'thread-(?P<program2_thread>\d+) Request complete'
            release_self_keys: [program2_thread]
            close: true
            
          - regex: 'thread-(?P<program2_thread>\d+)'

  # Example: single-threaded log where consecutive lines group until gap
  simple_log:
    description: "Groups consecutive log lines from a single-threaded service"
    temporal:
      max_gap: 1s
    attributes:
      # Derived attribute with no ${} references = static value, always defined
      - name: source_marker
        type: string
        key: true
        derived: "simple_log_fiber"
    sources:
      simple_service:
        patterns:
          - regex: 'END OF REQUEST'
            close: true
          - regex: '.+'  # Match any line

  # Example: per-source fiber that never closes
  nginx_all:
    description: "All nginx logs as a single fiber"
    temporal:
      max_gap: infinite
    attributes:
      - name: source_marker
        type: string
        key: true
        derived: "nginx"
    sources:
      nginx_access:
        patterns:
          - regex: '.+'

# =============================================================================
# PIPELINE SETTINGS
# =============================================================================

pipeline:
  backpressure:
    # How to handle slow downstream: 'block', 'drop', 'buffer_in_memory'
    strategy: block
    # For buffer_in_memory: max records before falling back to drop
    buffer_limit: 10000
  errors:
    # What to do on unparseable lines: 'drop' or 'panic'
    on_parse_error: drop
  checkpoint:
    # Checkpoints are stored in the DuckDB database (no separate file)
    enabled: true
    interval_seconds: 30

# =============================================================================
# SEQUENCER SETTINGS
# =============================================================================

sequencer:
  # Duration of each batch epoch (used for distributed deployments)
  batch_epoch_duration: 10s
  # Safety margin when computing watermarks
  watermark_safety_margin: 1s

# =============================================================================
# STORAGE SETTINGS
# =============================================================================

storage:
  # Path to DuckDB database file
  path: /var/lib/noil/noil.duckdb
  # Records per batch insert
  batch_size: 1000
  # Max time before flushing incomplete batch
  flush_interval_seconds: 5

# =============================================================================
# WEB SERVER SETTINGS
# =============================================================================

web:
  # Address to bind web UI, API, and collector protocol (if enabled)
  listen: 127.0.0.1:7104
  # Set to 0.0.0.0:7104 to allow external connections
```

### Attribute Types

- `string`: No normalization
- `ip`: Canonicalized (e.g., `192.168.001.001` → `192.168.1.1`)
- `mac`: Canonicalized lowercase with colons (e.g., `AA-BB-CC-11-22-33` → `aa:bb:cc:11:22:33`)
- `int`: Parsed as 64-bit signed integer
- `float`: Parsed as 64-bit float

### Derived Attributes

Derived attributes are computed via string interpolation:

```yaml
- name: connection_id
  type: string
  derived: "${src_ip}:${src_port}->${dst_ip}:${dst_port}"
```

- Only defined when all referenced attributes have values
- Can reference other derived attributes (no circular dependencies)
- Static values (no `${}` references) are always defined — useful for grouping all lines from a source

### Timestamp Formats

- Strptime format string (e.g., `'%Y-%m-%d %H:%M:%S'`)
- `iso8601`: ISO 8601 format
- `epoch`: Unix timestamp in seconds
- `epoch_ms`: Unix timestamp in milliseconds

## Storage Schema (DuckDB)

Noil uses embedded DuckDB for storage by default, providing zero-dependency deployment. The database is a single file that can be easily copied or backed up.

### Raw Logs Table

```sql
CREATE TABLE raw_logs (
    log_id UUID PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    source_id VARCHAR NOT NULL,
    raw_text VARCHAR NOT NULL,
    ingestion_time TIMESTAMPTZ DEFAULT now(),
    config_version UBIGINT NOT NULL
);

CREATE INDEX idx_raw_logs_timestamp ON raw_logs(timestamp);
CREATE INDEX idx_raw_logs_source ON raw_logs(source_id);
```

### Fibers Table

```sql
CREATE TABLE fibers (
    fiber_id UUID PRIMARY KEY,
    fiber_type VARCHAR NOT NULL,
    config_version UBIGINT NOT NULL,
    attributes JSON,
    first_activity TIMESTAMPTZ NOT NULL,
    last_activity TIMESTAMPTZ NOT NULL,
    closed BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX idx_fibers_type ON fibers(fiber_type);
CREATE INDEX idx_fibers_config ON fibers(config_version);
```

### Fiber Memberships Table

A many-to-many join table linking logs to fibers.

```sql
CREATE TABLE fiber_memberships (
    log_id UUID NOT NULL,
    fiber_id UUID NOT NULL,
    config_version UBIGINT NOT NULL,
    PRIMARY KEY (log_id, fiber_id)
);

CREATE INDEX idx_memberships_fiber ON fiber_memberships(fiber_id);
```

### Storage Abstraction

Storage is abstracted behind a trait to allow future backends (e.g., ClickHouse for large-scale deployments):

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    async fn init_schema(&self) -> Result<(), StorageError>;
    async fn write_logs(&self, logs: &[LogRecord]) -> Result<(), StorageError>;
    async fn write_memberships(&self, memberships: &[FiberMembership]) -> Result<(), StorageError>;
    async fn query_fiber_logs(&self, fiber_id: &str, pagination: Pagination) -> Result<Vec<LogRecord>, StorageError>;
    async fn query_log_fibers(&self, log_id: Uuid) -> Result<Vec<FiberInfo>, StorageError>;
    // ...
}
```

## Key Design Decisions

### Versioned Configuration

Configuration is immutable and versioned. Each log record and fiber membership carries a `config_version`. This enables:

- Processing in-flight logs with original config semantics
- Reprocessing historical logs with new rules
- Comparing results across config versions

### Logical Clock

The sequencer's emit timestamp is the logical clock for fiber processing. "Current time" is the timestamp of the most recently emitted log, not wall clock. This makes historical replay deterministic.

### Soft Fiber Closure

A fiber is eligible for closure when `logical_clock - fiber.last_activity > max_gap`. This works correctly for both live and historical ingestion.

### Watermark-Based Sequencing

Each sequencer tracks a watermark: the timestamp before which no more logs will arrive. Parent sequencers compute their watermark as the minimum of child watermarks. Logs/batches are only emitted when their timestamp is below the watermark.

### Pull-Based Distribution

Parent sequencers pull batches from children rather than children pushing. This naturally handles backpressure and variable network conditions. Children buffer locally until acknowledged.

### Reprocessing

Raw logs are the source of truth. Fiber memberships can be recomputed at any time by scanning raw_logs with new fiber type rules. Old fiber memberships can be retained (for comparison) or dropped.

## Implementation Notes

### Language and Dependencies

- Implementation language: Rust
- Async runtime: tokio
- Database: DuckDB (via `duckdb` crate)
- Compression: zstd (via `zstd` crate, pure Rust)
- Serialization: serde + serde_yaml for config, serde_json for API/attributes
- Regex: `regex` crate
- CLI: `clap` crate
- Web framework: `axum`
- Heap: `std::collections::BinaryHeap`

### Hot Reload Preparation

First iteration does not support hot reload, but design should not preclude it:

- No global/static config singletons
- Components receive config via explicit parameters
- Logs and fibers reference config by version ID, not embedded values
- Sequencer supports add/remove source operations
- Fiber processor rules abstracted behind trait

### Error Handling

- Parse errors: configurable (`drop` or `panic`)
- Network errors in distributed deployments: retry with backoff, wait forever for first pass
- Storage write errors: retry with backoff, block pipeline on persistent failure

### Development Environment: Termux

When developing in a Termux environment, be aware of the following when using shell commands:

**Avoid complex I/O redirection**: Commands with stderr redirection and piping (e.g., `2>&1 | head`) may fail with permission errors due to task directory creation issues in Termux's restricted filesystem.

**Prefer built-in flags over pipes**: Instead of:
- `cargo build 2>&1 | head -20` ❌
- `cargo test 2>&1` ❌

Use:
- `cargo build --quiet` ✓
- `cargo test` ✓
- `cargo check` ✓

**If output truncation is needed**: Use tool-specific flags (like `--quiet`) or accept full output rather than piping to `head` or `tail`.

**Path handling**: All path and string parameters in the config file support:
- **Tilde expansion**: `~` expands to the home directory
- **Environment variable expansion**: `$env{VAR_NAME}` expands to the value of the environment variable

This applies to:
- Source file paths
- Storage database path
- `--config` CLI argument
- Any other string value in the configuration

**Note**: Environment variables use `$env{VAR}` syntax to distinguish them from derived attribute interpolation, which uses `${attr}` syntax.

Example:
```yaml
storage:
  path: $env{TMPDIR}/noil.duckdb  # Uses system temp directory

sources:
  app_log:
    path: $env{LOG_DIR}/app.log  # Uses custom LOG_DIR environment variable
```

If an environment variable is not set, it is left unchanged (e.g., `$env{TMPDIR}` remains as literal text).

### Checkpointing

Checkpoint state includes:

- Per-source: file path, offset, inode, latest timestamp
- Sequencer: watermark, sequence counter
- Fiber processor: active fiber state (for crash recovery)

Checkpoints are stored in the DuckDB database and written periodically based on the configured interval.

## UI Requirements (Future)

- Display all log lines in a fiber in chronological order
- Colorized visual delineation between logs from different sources
- For any log line, show other fibers it belongs to
- Navigate between fibers
- Back-tracking history of visited fibers

## Additional Documentation

- **[specs/CONFIG_SYSTEM.md](specs/CONFIG_SYSTEM.md)**: **CRITICAL for AI agents** - Comprehensive explanation of Noil's unusual configuration system: YAML-as-ground-truth, database persistence, git-style 3-way merging, and version control. **Must read before working on any config-related task.**
- **[specs/COLLECTOR_MODE.md](specs/COLLECTOR_MODE.md)**: **Distributed Deployment Architecture** - Complete specification for distributed deployments: network protocol, epoch batching, backpressure, checkpointing, and failure scenarios. Read this before working on `collector` serving or `remote_collectors` features.
- **[docs/FIBER_PROCESSING.md](docs/FIBER_PROCESSING.md)**: Detailed explanation of fiber correlation semantics, the key/attribute model, session control actions, and worked examples.

## File Structure

```
noil/
├── Cargo.toml
├── CLAUDE.md
├── src/
│   ├── main.rs                  # CLI entry point, subcommand dispatch
│   ├── lib.rs
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── run.rs               # Main application runner
│   │   └── config.rs            # Config init subcommand
│   ├── config/
│   │   ├── mod.rs
│   │   ├── types.rs             # Config structs
│   │   ├── parse.rs             # YAML parsing and validation
│   │   └── generate.rs          # Starter config generation
│   ├── source/
│   │   ├── mod.rs
│   │   ├── reader.rs
│   │   └── timestamp.rs
│   ├── sequencer/
│   │   ├── mod.rs
│   │   ├── local.rs
│   │   └── merge.rs
│   ├── fiber/
│   │   ├── mod.rs
│   │   ├── processor.rs
│   │   ├── rule.rs
│   │   └── session.rs
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── traits.rs            # Storage trait definition
│   │   ├── duckdb.rs            # DuckDB implementation
│   │   └── checkpoint.rs
│   ├── pipeline/
│   │   ├── mod.rs
│   │   ├── channel.rs
│   │   └── backpressure.rs
│   └── web/
│       ├── mod.rs
│       ├── server.rs
│       ├── api.rs
│       └── assets/              # Embedded frontend assets
├── frontend/                    # Frontend source (compiled into assets/)
│   └── ...
└── tests/
    └── ...
```

## Glossary

- **Fiber**: A group of related log lines identified by UUID, with shared keys while open and persistent attributes
- **Fiber Type**: A rule template defining how to identify and group logs into fibers
- **Attribute**: A named value extracted from logs or derived via interpolation; persists on fiber after close
- **Key**: An attribute marked with `key: true`; used for fiber matching and merging while fiber is open; released when fiber closes
- **Derived Attribute**: An attribute computed via string interpolation from other attributes (e.g., `"${ip}:${port}"`); only defined when all referenced attributes have values
- **Pattern**: A regex that matches log lines and extracts named capture groups as attributes
- **release_matching_peer_keys**: Pattern action that removes matching `(key, value)` pairs from OTHER open fibers before processing (useful for "request start" patterns to prevent new requests from merging with old ones)
- **release_self_keys**: Pattern action that removes specified keys from THIS fiber after processing (useful for "request end" patterns)
- **close**: Pattern action that closes the fiber after processing, releasing all keys
- **Watermark**: A timestamp guarantee that no earlier logs will arrive from a source
- **Sequencer**: A component that merges log streams into global timestamp order
- **Logical Clock**: The timestamp of the most recently processed log; used for fiber timeout decisions instead of wall clock time
