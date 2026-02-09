pub fn generate_starter_config() -> String {
    r#"# =============================================================================
# NOIL CONFIGURATION
# =============================================================================
# This file configures log sources, fiber correlation rules, and system settings.
#
# Noil uses a capability-based configuration model. Each instance runs the same
# binary — capabilities are enabled by which config sections are present:
#
#   sources:             Read local log files (optional)
#   remote_collectors:   Pull logs from remote Noil instances (optional)
#   collector:           Serve batched logs to other Noil instances (optional)
#   fiber_types:         Enable log storage and fiber processing (optional)
#
# At least one input (sources or remote_collectors) must be configured.
# Sections can be freely combined for different deployment patterns.
#
# Config file locations (in order of precedence):
#   1. Path specified via --config argument
#   2. ~/.config/noil/config.yml
#   3. /etc/noil/config.yml

# =============================================================================
# SOURCES (optional)
# =============================================================================
# Define local log files to ingest. Each source needs a unique ID and timestamp
# config. Omit this section entirely if this instance only pulls from remote
# collectors.

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

  program1:
    type: file
    path: /var/log/program1.log
    timestamp:
      pattern: '^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})'
      format: '%Y-%m-%d %H:%M:%S'
    read:
      start: beginning
      follow: true

  program2:
    type: file
    path: /var/log/program2.log
    timestamp:
      pattern: '^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})'
      format: '%Y-%m-%d %H:%M:%S'
    read:
      start: beginning
      follow: true

  simple_service:
    type: file
    path: /var/log/simple_service.log
    timestamp:
      pattern: '^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})'
      format: '%Y-%m-%d %H:%M:%S'
    read:
      start: beginning
      follow: true

# =============================================================================
# REMOTE COLLECTORS (optional)
# =============================================================================
# Pull logs from remote Noil instances that have collector serving enabled.
# Omit this section if this instance only reads local files.
#
# remote_collectors:
#   endpoints:
#     - id: node1                    # Unique identifier for this collector
#       url: http://10.0.0.1:7104   # URL of the remote instance
#       retry_interval: 5s           # Retry delay on connection failure
#       timeout: 30s                 # HTTP request timeout
#     - id: node2
#       url: http://10.0.0.2:7104
#       retry_interval: 5s
#       timeout: 30s
#   poll_interval: 1s               # How often to check for new batches
#   backpressure:
#     strategy: block
#     buffer_limit: 10000

# =============================================================================
# COLLECTOR SERVING (optional)
# =============================================================================
# Serve batched, ordered logs to other Noil instances via the /collector/* HTTP
# API on the web.listen address. Requires local sources to have something to
# serve. Omit this section if this instance does not need to serve logs.
#
# collector:
#   epoch_duration: 10s
#   buffer:
#     max_epochs: 100              # Max epochs to buffer before overflow
#     strategy: block              # block | drop_oldest | wait_forever
#   checkpoint:
#     enabled: true
#     interval_seconds: 30
#   status_ui:
#     enabled: true                # Read-only status page

# =============================================================================
# AUTO-GENERATED SOURCE FIBERS
# =============================================================================
# By default, Noil automatically creates a fiber type for each source named
# {source_name}_all. Each auto-generated fiber:
#   - Collects all logs from that source
#   - Never closes (max_gap: infinite)
#   - Provides a convenient UI navigation starting point
#
# This feature is enabled by default. To disable it, uncomment:
# auto_source_fibers: false
#
# You can also manually define a fiber type with the same name to override
# the auto-generated version with custom settings.

# =============================================================================
# FIBER TYPES (optional)
# =============================================================================
# Define rules for correlating logs into fibers. The presence of this section
# (even if empty) enables log storage and fiber processing. Omitting it
# entirely means logs flow through but are not stored — useful for instances
# that only serve as collectors.

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
  batch_epoch_duration: 10s
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
# Single listen address for web UI, API, and collector protocol (if enabled).

web:
  listen: 127.0.0.1:7104
  # Set to 0.0.0.0:7104 to allow external connections
"#
    .to_string()
}
