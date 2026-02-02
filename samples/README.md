# Noil Sample Configurations

This directory contains sample configurations and log files that demonstrate Noil's features.

## Available Configurations

### 1. Standalone Mode (sample-config.yml)

Full-featured single-instance deployment with sources, fiber processing, and web UI.

**Use case**: Single machine deployment, all logs on local filesystem.

```bash
# From the project root
cargo run -- --config samples/sample-config.yml

# Or if noil is installed
noil --config samples/sample-config.yml
```

After starting, the web UI will be available at http://localhost:7104

### 2. Collector Mode (collector-config.yml)

Lightweight deployment for edge nodes that read local logs and serve batches to a parent.

**Use case**: Deploy on remote machines where logs are generated.

```bash
noil --config samples/collector-config.yml
```

The collector's status UI will be available at http://<collector-host>:7105

**Key features**:
- Minimal resource usage (no fiber processing, no full database)
- HTTP API for parent to pull batches
- Local buffering for network resilience
- Checkpoint-based crash recovery

### 3. Parent Mode (parent-config.yml)

Central instance that pulls from multiple collectors, performs fiber processing, and provides full UI.

**Use case**: Deploy in a central location to correlate logs from multiple edge nodes.

```bash
noil --config samples/parent-config.yml
```

The parent's web UI will be available at http://<parent-host>:7104

**Key features**:
- Pulls from multiple collectors via HTTP
- Hierarchical sequencing with watermark-based coordination
- Full fiber processing and correlation
- DuckDB storage for all logs and fibers
- Complete web UI for exploration

## Deployment Architectures

### Single Machine (Standalone Mode)

```
┌──────────────────────────────┐
│   Noil Standalone Instance   │
│                              │
│  • Local log sources         │
│  • Fiber processing          │
│  • DuckDB storage            │
│  • Web UI                    │
└──────────────────────────────┘
```

Use `sample-config.yml`

### Distributed Deployment (Collector + Parent)

```
      ┌─────────────────────────────────┐
      │    Parent Instance (Central)    │
      │  • Fiber processing             │
      │  • DuckDB storage               │
      │  • Web UI                       │
      └──▲─────────────▲────────────▲──┘
         │             │            │
         │  HTTP Pull  │  HTTP Pull │  HTTP Pull
         │             │            │
    ┌────┴─────┐  ┌───┴──────┐  ┌─┴──────────┐
    │Collector1│  │Collector2│  │Collector3  │
    │Edge Node │  │Edge Node │  │Edge Node   │
    └──────────┘  └──────────┘  └────────────┘
```

Use `collector-config.yml` on edge nodes, `parent-config.yml` centrally.

## Quick Start

## Sample Log Files

### `logs/program1.log`
Frontend proxy service that:
- Receives client connections with IP addresses and ports
- Authenticates clients via MAC address
- Forwards requests to a backend server
- Returns responses to clients

**Demonstrates**: Thread-based processing, IP/MAC extraction, connection lifecycle

### `logs/program2.log`
Backend service that:
- Processes requests forwarded from program1
- Queries database based on MAC address
- Generates and returns response payloads

**Demonstrates**: Backend processing, request completion markers, thread correlation

### `logs/nginx_access.log`
Standard nginx access log in Common Log Format.

**Demonstrates**: Different timestamp format, simple log capture

### `logs/simple_service.log`
Single-threaded service with clear request boundaries.

**Demonstrates**: Session-based grouping, explicit close markers

## Expected Fiber Correlation

When you run Noil with this configuration, you should see:

### Request Trace Fibers (3 total)

**Fiber 1**: First request
- Program1 thread-1 + Program2 thread-5
- MAC: aa:bb:cc:11:22:33
- Client: 192.168.1.100:45678
- Backend: 10.0.0.5:8080
- ~25 log lines across both programs
- Closed when program2 thread-5 completes

**Fiber 2**: Second request
- Program1 thread-2 + Program2 thread-6
- MAC: dd:ee:ff:44:55:66
- Client: 192.168.1.101:45679
- Backend: 10.0.0.5:8080
- ~25 log lines across both programs
- Closed when program2 thread-6 completes

**Fiber 3**: Third request (demonstrates thread reuse)
- Program1 thread-1 + Program2 thread-7
- MAC: 11:22:33:aa:bb:cc
- Client: 192.168.1.102:45680
- Backend: 10.0.0.5:8080
- ~15 log lines across both programs
- Note: thread-1 is reused from Fiber 1 (released via `release_matching_peer_keys`)
- Closed when program2 thread-7 completes

### Simple Log Fibers (3 total)

**Fiber A**: Service startup logs
- 5 log lines before first REQUEST START
- Closed by 1s timeout

**Fiber B**: First request session
- 7 log lines from REQUEST START to END OF REQUEST
- Closed by END OF REQUEST pattern

**Fiber C**: Second request session
- 7 log lines from REQUEST START to END OF REQUEST
- Closed by END OF REQUEST pattern

**Fiber D**: Health check
- 1 log line
- Closed by 1s timeout

**Fiber E**: Third request session
- 4 log lines from REQUEST START to END OF REQUEST
- Closed by END OF REQUEST pattern

### Nginx All Fiber (1 total)

**Fiber N**: All nginx logs
- 9 log lines from nginx_access.log
- Never closed (max_gap: infinite)

## Features Demonstrated

### 1. Multiple Sources
Four different log sources with different timestamp formats:
- ISO 8601: `2025-01-11T10:00:00.100Z`
- Common Log: `11/Jan/2025:10:00:00 +0000`
- Custom format: `2025-01-11 10:00:00.000`

### 2. Key-Based Correlation
Logs from program1 and program2 are correlated via:
- **MAC address**: Primary correlation across both programs
- **Thread IDs**: Temporary keys released after request completion

### 3. Derived Attributes
- `client_connection`: `"${client_ip}:${client_port}"`
- `backend_connection`: `"${backend_ip}:${backend_port}"`
- `source_marker`: Static value for grouping all logs from a source

### 4. Attribute Types
- `string`: Thread IDs, derived connections
- `ip`: Client and backend IP addresses (canonicalized)
- `mac`: MAC addresses (normalized to lowercase with colons)
- `int`: Port numbers

### 5. Session Control Actions

**`release_matching_peer_keys`**: Used in program1 when thread receives new connection
```yaml
- regex: 'thread-(?P<program1_thread>\d+) Received connection...'
  release_matching_peer_keys: [program1_thread]
```
This prevents thread-1 from merging Fiber 1 and Fiber 3 together.

**`release_self_keys`**: Used in program2 when request completes
```yaml
- regex: 'thread-(?P<program2_thread>\d+) Request complete'
  release_self_keys: [program2_thread]
```
Allows the thread key to be reused for future requests.

**`close`**: Explicitly closes fibers
- program2 "Request complete" → closes request_trace fiber
- simple_service "END OF REQUEST" → closes simple_log fiber

### 6. Temporal Constraints

- **Session windowing** (request_trace, simple_log): max_gap measured between consecutive logs
- **Infinite duration** (nginx_all): Fiber never times out

### 7. Fiber Merging

When program1 thread-1 log matches MAC aa:bb:cc:11:22:33, and program2 thread-5 log also matches the same MAC, those two fibers merge into one request_trace fiber.

## Exploring the Results

After running Noil with this config:

1. **View fibers in the web UI** at http://localhost:7104
2. **Query the database** directly:
   ```bash
   sqlite3 /tmp/noil-sample.duckdb

   -- See all fibers
   SELECT fiber_id, fiber_type, attributes, first_activity, last_activity, closed
   FROM fibers;

   -- See fiber memberships
   SELECT f.fiber_type, COUNT(*) as log_count
   FROM fiber_memberships fm
   JOIN fibers f ON fm.fiber_id = f.fiber_id
   GROUP BY f.fiber_type;
   ```

3. **Examine raw logs**:
   ```sql
   SELECT timestamp, source_id, raw_text
   FROM raw_logs
   ORDER BY timestamp;
   ```

## Modifying the Samples

Try these experiments to learn more:

1. **Remove `release_matching_peer_keys`** from program1's "Received connection" pattern
   - Result: Fiber 1 and Fiber 3 will merge (both use thread-1)

2. **Remove `close: true`** from program2's "Request complete" pattern
   - Result: Fibers will only close due to 5s timeout

3. **Change `max_gap`** to 10s in request_trace
   - Result: More tolerance for gaps between related logs

4. **Add more log lines** to the sample files
   - Create your own scenarios and watch the correlation work

5. **Change `gap_mode`** to `from_start` in simple_log
   - Result: All logs must be within 1s of the first log (stricter grouping)
