# Noil

Noil is a log correlation system that ingests logs from multiple sources, sequences them into global timestamp order, and groups them into "fibers" based on configurable rules. Named after the short, tangled fibers combed out during textile processing.

## Core Concepts

### Fibers

A fiber is a group of log lines that are related by shared attributes and temporal proximity. A single log line may belong to multiple fibers. Fiber membership is computed at ingestion time based on fiber type rules.

### Fiber Types

A fiber type is a template that defines:

- **Temporal constraint**: Maximum time gap between consecutive logs (session windowing) or from the first log
- **Key attributes**: Attributes that form the fiber's identity (e.g., `request_id`)
- **Enrichment attributes**: Optional attributes populated opportunistically
- **Source patterns**: Per-source regex patterns that filter logs and extract attributes

A fiber's identity is the tuple `(fiber_type, key_attribute_values, session_id)`. Two logs belong to the same fiber if they match the same fiber type, produce identical key attribute values, and fall within the temporal continuity constraint.

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

**Raw Log Store**: Durable storage (ClickHouse) of all ingested logs. Source of truth for reprocessing.

**Fiber Processor**: Evaluates fiber type rules against each log, manages active fiber sessions, computes fiber membership.

**Storage Writer**: Batched writes to ClickHouse for both raw logs and fiber memberships.

### Distributed Deployment

For logs on remote machines, Noil supports hierarchical sequencing:

```
Remote Agents → Site Sequencers → Central Sequencer → Raw Log Store
```

**Agent**: Runs on remote machine, reads local logs, sequences locally, batches into epochs, serves batches to parent sequencer on request.

**Hierarchical Sequencers**: Parent sequencers merge streams from children using watermark-based coordination. Pull-based model: parent requests batches when ready, children buffer locally.

**Epoch-based batching**: Instead of perfect per-record ordering, sequencers emit batches covering time epochs with the guarantee that all logs in batch N have timestamps ≤ all logs in batch N+1.

## Configuration

All configuration lives in a single `config.yml` file. The file is divided into sections for sources, fiber types, pipeline settings, storage, and web server.

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
# FIBER TYPES
# =============================================================================
# Define rules for correlating logs into fibers. Each fiber type specifies:
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
        match: first  # first | all
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
    enabled: true
    interval_seconds: 30
    path: /var/lib/noil/checkpoint.json

# =============================================================================
# SEQUENCER SETTINGS
# =============================================================================

sequencer:
  # For distributed mode: duration of each batch epoch
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
  # Address to bind the web UI
  listen: 127.0.0.1:8080
  # Set to 0.0.0.0:8080 to allow external connections
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

### Fiber Memberships Table

```sql
CREATE TABLE fiber_memberships (
    log_id UUID NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL,
    config_version UBIGINT NOT NULL,
    fiber_type VARCHAR NOT NULL,
    fiber_id VARCHAR NOT NULL,
    key_attributes JSON,
    enrichment_attributes JSON,
    PRIMARY KEY (config_version, fiber_type, fiber_id, log_id)
);

CREATE INDEX idx_memberships_log ON fiber_memberships(log_id);
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

A fiber is eligible for closure when `logical_clock - fiber.last_updated > max_gap`. This works correctly for both live and historical ingestion.

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
- Network errors in distributed mode: retry with backoff, wait forever for first pass
- Storage write errors: retry with backoff, block pipeline on persistent failure

### Checkpointing

Checkpoint state includes:

- Per-source: file path, offset, inode, latest timestamp
- Sequencer: watermark, sequence counter
- Fiber processor: active fiber state (for crash recovery)

Checkpoints written periodically to configured path.

## UI Requirements (Future)

- Display all log lines in a fiber in chronological order
- Colorized visual delineation between logs from different sources
- For any log line, show other fibers it belongs to
- Navigate between fibers
- Back-tracking history of visited fibers

## Additional Documentation

- **[docs/FIBER_PROCESSING.md](docs/FIBER_PROCESSING.md)**: Detailed explanation of fiber correlation semantics, the primary/associative key model, session markers, and worked examples.

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
- **Key**: An attribute marked with `key: true`; used for fiber matching and merging while fiber is open
- **Derived Attribute**: An attribute computed via string interpolation from other attributes (e.g., `"${ip}:${port}"`)
- **Pattern**: A regex that matches log lines and extracts named capture groups as attributes
- **release_matching_peer_keys**: Pattern action that removes matching key values from other open fibers before processing
- **release_self_keys**: Pattern action that removes specified keys from the current fiber after processing
- **Watermark**: A timestamp guarantee that no earlier logs will arrive from a source
- **Sequencer**: A component that merges log streams into global timestamp order
