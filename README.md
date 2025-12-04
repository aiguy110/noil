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

## Status

This project is a work in progress. Watch for updates.
