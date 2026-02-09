<p align="center">
  <img src="frontend/assets/noil_icon_v1_transparent.png" alt="Noil Icon" width="128">
</p>

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
- **Distributed deployment**: Supports distributed collection and central correlation of logs from multiple remote machines.

## Capabilities

Noil uses a capability-based configuration model. Every instance runs the same binary — capabilities are determined by which config sections are present:

| Config section | Capability |
|---|---|
| `sources` | Read local log files |
| `remote_collectors` | Pull logs from remote Noil instances |
| `collector` | Serve batched logs to other Noil instances |
| `fiber_types` | Store logs and run fiber processing |

At least one input (`sources` or `remote_collectors`) must be configured. Sections can be freely combined — a single instance can read local files, pull from remote collectors, serve logs to others, and run fiber processing all at once.

### Common deployment patterns

**Local analysis** — read local files with fiber processing:
```yaml
sources: { ... }
fiber_types: { ... }
```

**Edge collector** — read local files and serve them to a central instance (no storage):
```yaml
sources: { ... }
collector: { ... }
```

**Central correlator** — pull from remote collectors, store and process:
```yaml
remote_collectors: { ... }
fiber_types: { ... }
```

**Hybrid** — read local files, pull from remote collectors, serve to others, and process:
```yaml
sources: { ... }
remote_collectors: { ... }
collector: { ... }
fiber_types: { ... }
```

### Example distributed architecture

```
┌─────────────────────────────────────────────────┐
│               Central Data Center               │
│                                                 │
│  ┌──────────────────────────────────────────┐   │
│  │             Central Instance             │   │
│  │  remote_collectors + fiber_types         │   │
│  │  • Merges collector streams              │   │
│  │  • Fiber processing                      │   │
│  │  • DuckDB storage                        │   │
│  │  • Web UI                                │   │
│  └─▲───────────────▲───────────────▲────────┘   │
│    │               │               │            │
└────┼───────────────┼───────────────┼────────────┘
     │               │               │
     │ (HTTP Pull)   │ (HTTP Pull)   │ (HTTP Pull)
     │               │               │
┌────┴─────┐    ┌────┴─────┐    ┌────┴─────┐
│Collector1│    │Collector2│    │Collector3│
│ Edge VM  │    │ Edge VM  │    │ Edge VM  │
│sources + │    │sources + │    │sources + │
│collector │    │collector │    │collector │
└──────────┘    └──────────┘    └──────────┘
```

See `samples/sample-config.yml` for a complete example with all sections documented.

For distributed deployment details (protocol, epoch batching, backpressure), see [specs/COLLECTOR_MODE.md](specs/COLLECTOR_MODE.md).

## Status

This project is a work in progress. Watch for updates.
