# Unified Mode Refactor

## Context

Noil currently has three operation modes (standalone, collector, parent) that are mutually exclusive and dispatched via a `mode` enum. This creates unnecessary complexity — every instance is the same binary, and the distinction is really about which capabilities are enabled. The refactor eliminates the mode enum entirely: every instance runs the same unified pipeline, conditionally enabling capabilities based on which config sections are present.

Key motivations:
- Simpler mental model: one instance type, mix-and-match capabilities
- An instance can simultaneously read local files, pull from remote collectors, AND serve as a collector
- Eliminates the `collector.listen` / `web.listen` duplication (single listen address)
- `fiber_types` key presence controls log storage (no extra boolean flag)

## Config Shape (After)

```yaml
# No mode field

sources:              # Optional: local file sources
  app_log:
    type: file
    path: /var/log/app.log
    ...

remote_collectors:    # Optional: pull from remote instances (replaces parent.collectors)
  endpoints:
    - id: node1
      url: http://10.0.0.1:7104
      retry_interval: 5s
      timeout: 30s
  poll_interval: 1s
  backpressure:
    strategy: block
    buffer_limit: 10000

collector:            # Optional: serve logs to other instances (loses `listen` field)
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: block
  checkpoint:
    enabled: true
    interval_seconds: 30

fiber_types:          # Optional: presence (even empty) enables log storage + fiber processing
  my_fiber: ...       # Absence = no log storage, only checkpoints in DB

pipeline: ...
sequencer: ...
storage: ...
web:
  listen: 127.0.0.1:7104   # Single listen address for web UI + collector protocol
```

## Implementation Phases

### Phase 1: Config Structs (`src/config/types.rs`)

1. **Remove** `OperationMode` enum, `default_mode()`, and `mode` field from `Config`
2. **Add** `remote_collectors: Option<RemoteCollectorsConfig>` to `Config` (replaces `parent: Option<ParentConfig>`)
   ```rust
   pub struct RemoteCollectorsConfig {
       pub endpoints: Vec<CollectorEndpoint>,  // reuse existing CollectorEndpoint
       #[serde(with = "humantime_serde")]
       pub poll_interval: Duration,
       pub backpressure: BackpressureConfig,
   }
   ```
3. **Remove** `parent: Option<ParentConfig>` from `Config`
4. **Rename** `CollectorConfig` to `CollectorServingConfig`, remove its `listen` field. Keep `epoch_duration`, `buffer`, `checkpoint`, `status_ui`. The YAML key stays `collector`.
5. **Change** `fiber_types: HashMap<String, FiberTypeConfig>` to `Option<HashMap<String, FiberTypeConfig>>` with custom deserializer:
   ```rust
   #[serde(default, deserialize_with = "deserialize_fiber_types")]
   pub fiber_types: Option<HashMap<String, FiberTypeConfig>>,
   ```
   - Key absent → `None` (no log storage)
   - `fiber_types:` (null) → `Some({})` (store logs, no rules yet)
   - `fiber_types: {}` → `Some({})` (store logs, no rules yet)
   - `fiber_types:` with entries → `Some({...})`
6. **Add** helper methods to `Config`:
   - `has_local_sources()` → `!self.sources.is_empty()`
   - `has_remote_sources()` → `self.remote_collectors.as_ref().map_or(false, |c| !c.endpoints.is_empty())`
   - `has_collector_serving()` → `self.collector.is_some()`
   - `stores_logs()` → `self.fiber_types.is_some()`
   - `fiber_types_or_empty()` → returns `&HashMap` (empty if None)

### Phase 2: Validation (`src/config/parse.rs`)

1. **Replace** `validate_mode_config()` with `validate_config_capabilities()`:
   - Must have at least one input (local sources or collectors)
   - If `remote_collectors` present: validate endpoints non-empty, IDs unique, URLs non-empty
   - If `collector` serving present: must have local sources (nothing to serve otherwise)
2. **Update** `validate_fiber_type()`: remove `mode: OperationMode` param. Skip source-reference validation when `config.has_remote_sources()` (sources may come from remote instances).
3. **Update** `add_auto_source_fibers()`: operate on `Option<HashMap>` — only run when `fiber_types.is_some()`
4. **Update** all callers of validation functions to remove mode parameter

### Phase 3: Unified Pipeline (`src/cli/run.rs`)

Replace the three functions (`run_standalone_mode`, `run_collector_mode`, `run_parent_mode`) with a single `run_pipeline`:

1. Initialize storage + reconcile config (always)
2. Determine capabilities from config helpers
3. Load checkpoint if enabled
4. **Local sources** (if `has_local_sources`): create source readers, start sequencer feeding `seq_tx`
5. **Remote sources** (if `has_remote_sources`): create collector clients/streams, spawn polling tasks feeding `seq_tx`
6. Drop `seq_tx` after all producers are set up (channel closes when all producers done)
7. **Stream fan-out**: If both fiber processing AND collector serving are enabled, insert a tee task that clones records to both the processor input and the epoch batcher input. If only one consumer, no tee needed.
8. **Fiber processing** (if `stores_logs`): create fiber processor, run processor + storage writer consuming from `seq_rx` (or tee output)
9. **No fiber processing**: drain `seq_rx` (records flow through but aren't stored)
10. **Collector serving** (if `has_collector_serving`): set up EpochBatcher + BatchBuffer, create `CollectorState`
11. **Web server** (always): start with optional `collector_state` and optional `fiber_processor`
12. Unified shutdown logic

Key reuse:
- `run_sequencer()` from `src/sequencer/merge.rs` — unchanged
- `run_processor()`, `run_writer()` from `src/pipeline/` — unchanged
- Collector polling loop logic from `src/parent/runner.rs` — extract into reusable functions
- EpochBatcher/BatchBuffer setup from `src/collector/runner.rs` — extract into reusable functions

### Phase 4: Merge Collector Endpoints into Web Server

1. **Modify** `src/web/server.rs` `run_server()`:
   - Add `collector_state: Option<Arc<CollectorState>>` parameter
   - Make `fiber_processor` parameter `Option<Arc<RwLock<FiberProcessor>>>`
   - Conditionally merge `/collector/*` routes when `collector_state` is `Some`
   - Conditionally include log/fiber API routes when fiber_processor is `Some`
2. **Remove** `src/collector/server.rs` `start_server()` function (the separate server)
3. **Refactor** `src/collector/runner.rs`: extract batcher/buffer setup into standalone functions callable from `run_pipeline`. `CollectorRunner` struct may be removed or simplified.
4. **Refactor** `src/parent/runner.rs`: extract collector polling logic into standalone functions. `ParentRunner` struct may be removed or simplified.

### Phase 5: Update All Mode References

Every `OperationMode` / `config.mode` reference must be replaced:

| File | Line(s) | Change |
|------|---------|--------|
| `src/pipeline/runner.rs` | 1, 84 | `config.mode == Parent` → `config.has_remote_sources()` |
| `src/web/api.rs` | 16, 787, 1517 | Same replacement |
| `src/pipeline/runner.rs` | 406 | Remove `mode` field from test Config construction |
| `src/web/api.rs` | 793 | `config.fiber_types.contains_key()` → `config.fiber_types_or_empty().contains_key()` |

### Phase 6: Interactive Config + YAML Builder

**`src/cli/interactive/mod.rs`**:
- Remove mode selection prompt
- Ask independent capability questions:
  1. "Read local log files?" → source loop (existing flow)
  2. "Pull logs from remote Noil instances?" → endpoint loop (existing parent flow)
  3. "Serve logs to other Noil instances?" → collector config
  4. "Enable log storage and fiber processing?" → controls fiber_types presence
- Remove mode-conditional listen address logic (always ask once)

**`src/cli/interactive/yaml_builder.rs`**:
- Replace `InteractiveConfig.mode: String` with capability booleans
- `build_yaml()` generates sections based on capability flags, not mode string
- When `enable_fiber_processing` is true, include `fiber_types:` key (even if empty placeholder)
- When false, omit `fiber_types` entirely (with comment explaining why)

**`src/cli/config.rs`**:
- Remove mode-based sample file selection
- Use single unified sample config
- Remove `--mode` CLI arg from `ConfigAction::Init` in `src/main.rs`

### Phase 7: Sample Configs + Documentation

1. **Replace** three sample configs with one `samples/sample-config.yml` showing all capabilities with comments explaining which sections are optional
2. **Update** `CLAUDE.md`: remove all mode references, update config documentation, update CLI docs
3. **Update** `specs/COLLECTOR_MODE.md` and `specs/CONFIG_SYSTEM.md` if they reference modes
4. **Update** `RESUME.md` or remove it

### Phase 8: Tests

| Test File | Changes |
|-----------|---------|
| `tests/config_phase1_tests.rs` | Remove `mode:` from YAML strings, remove `assert_eq!(config.mode, ...)`, update `CollectorConfig` → `CollectorServingConfig`, `parent` → `remote_collectors` |
| `tests/parent_phase3_tests.rs` | Replace `mode: parent` + `parent:` with `remote_collectors:` section. Remove `OperationMode` imports |
| `tests/collector_phase2_tests.rs` | Replace `mode: collector` with just `collector:` section. Update config struct references |
| `tests/end_to_end_tests.rs` | Update any mode references |
| `tests/config_tests.rs` | Update fiber_types access (`Option<HashMap>`) |
| `src/cli/interactive/yaml_builder.rs` tests | Rewrite for capability-based config |
| `src/pipeline/runner.rs` tests | Remove `mode` from test Config construction |

## Commit Strategy

This is a pre-release codebase — no backwards compatibility needed. Implement in logical commits on a feature branch:

1. **Commit 1**: Phases 1+2+5 atomically (types + validation + all mode reference updates). Must be atomic because changing types breaks compilation everywhere.
2. **Commit 2**: Phase 3 (unified run_pipeline)
3. **Commit 3**: Phase 4 (merge collector into web server)
4. **Commit 4**: Phase 6 (interactive + YAML builder)
5. **Commit 5**: Phases 7+8 (samples, docs, tests)

## Verification

1. `cargo check` after each commit
2. `cargo test` — all existing tests pass (updated)
3. Manual test: `noil config init --stdout` produces valid unified config
4. Manual test: `noil config validate` against a unified config
5. Manual test: run with local-sources-only config (replaces standalone)
6. Manual test: run with local-sources + collector-serving config (replaces collector)
7. Manual test: run with remote_collectors + fiber_types config (replaces parent)
8. Manual test: run with all capabilities enabled simultaneously (new capability)
