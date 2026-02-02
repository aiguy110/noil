# Noil

**noil** /nɔɪl/ *noun* — short, tangled fibers combed out during textile processing.

---

Noil is a log correlation system that ingests logs from multiple sources, sequences them into global timestamp order, and groups related lines into "fibers" based on configurable rules.

## What it does

- **Multi-source ingestion**: Reads logs from multiple files, extracts timestamps, handles multiline records
- **Global sequencing**: Merges streams into timestamp order using watermark-based coordination
- **Fiber correlation**: Groups related logs across sources using key-based matching and temporal proximity
- **Queryable storage**: Stores raw logs and fiber memberships in DuckDB for exploration

## Design highlights

- **Two-phase architecture**: Capture (ingest and store raw logs) is separate from processing (compute fiber memberships). Reprocess anytime with new rules.
- **Logical clock**: Fiber timeouts use log timestamps, not wall clock. Historical replay is deterministic.
- **Dynamic keys**: Fiber keys enable matching while open, then release on close. Attributes persist. This handles identifier reuse (e.g., thread IDs recycled between requests).
- **Single binary**: Sequencer, fiber processor, storage, and web UI ship as one `noil` executable.
- **Distributed deployment**: Supports collector/parent architecture for correlating logs from multiple remote machines.

## Deployment Modes

Noil supports three operation modes:

### Standalone Mode (Default)

Single-instance deployment with local sources, fiber processing, and web UI. Best for:
- Single machine deployments
- Development and testing
- All logs on local filesystem

```bash
noil --config config.yml  # mode: standalone
```

### Collector Mode

Lightweight instance that reads local logs and serves batched, ordered streams to a parent via HTTP. Best for:
- Edge nodes and remote machines
- Minimal resource footprint (no fiber processing, no full database)
- Network-resilient buffering

```bash
noil --config collector-config.yml  # mode: collector
```

**Key features**:
- Reads local log sources
- Sequences into global timestamp order
- Batches into time-windowed epochs
- Serves batches via HTTP API
- Buffers locally during network issues
- Checkpoint-based crash recovery

### Parent Mode

Central instance that pulls from multiple collectors, performs fiber processing, and provides the full UI. Best for:
- Central correlation point for distributed logs
- Multi-site deployments
- Centralized fiber processing and storage

```bash
noil --config parent-config.yml  # mode: parent
```

**Key features**:
- Pulls batches from multiple collectors
- Hierarchical sequencing with watermark coordination
- Full fiber processing and correlation
- DuckDB storage for all logs and fibers
- Complete web UI for exploration

### Example Distributed Architecture

```
┌─────────────────────────────────────────────────────┐
│              Central Data Center                     │
│                                                       │
│  ┌─────────────────────────────────────────────┐   │
│  │           Parent Instance                    │   │
│  │  • Merges collector streams                  │   │
│  │  • Fiber processing                          │   │
│  │  • DuckDB storage                            │   │
│  │  • Web UI                                    │   │
│  └──▲───────────────▲────────────────▲─────────┘   │
│     │               │                │              │
└─────┼───────────────┼────────────────┼──────────────┘
      │               │                │
      │ (HTTP Pull)   │ (HTTP Pull)    │ (HTTP Pull)
      │               │                │
┌─────┴────┐    ┌────┴─────┐    ┌────┴─────┐
│Collector1│    │Collector2│    │Collector3│
│ Edge VM  │    │ Edge VM  │    │ Edge VM  │
└──────────┘    └──────────┘    └──────────┘
```

See `samples/collector-config.yml` and `samples/parent-config.yml` for complete examples.

For detailed collector mode architecture, protocol specification, and implementation details, see [specs/COLLECTOR_MODE.md](specs/COLLECTOR_MODE.md).

## Status

This project is a work in progress. Watch for updates.
