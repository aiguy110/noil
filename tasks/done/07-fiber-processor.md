# 07: Fiber Processor

Implement the core fiber correlation algorithm.

## Required Context

Read these files before starting:
- `CLAUDE.md` — Architecture, config schema, storage schema
- `docs/FIBER_PROCESSING.md` — Algorithm details, key/attribute lifecycle, merge semantics, worked examples

## Location

`src/fiber/processor.rs`, `src/fiber/session.rs`, `src/fiber/rule.rs`

## Overview

The fiber processor receives sequenced logs and:
1. Matches them to open fibers via keys
2. Creates new fibers when no match
3. Merges fibers when a log matches multiple
4. Closes fibers on timeout or explicit close
5. Outputs fiber memberships and fiber updates

**Critical**: Each fiber type is processed independently. Implement per-type processors that can run in parallel.

## Compiled Rules (`rule.rs`)

```rust
pub struct CompiledFiberType {
    pub name: String,
    pub temporal: TemporalConfig,
    pub attributes: Vec<AttributeDef>,
    pub key_names: HashSet<String>,
    pub derived_order: Vec<String>,  // Topologically sorted
    pub derived_templates: HashMap<String, DerivedTemplate>,
    pub source_patterns: HashMap<String, Vec<CompiledPattern>>,
}

pub struct CompiledPattern {
    pub regex: Regex,
    pub release_matching_peer_keys: Vec<String>,
    pub release_self_keys: Vec<String>,
    pub close: bool,
    pub extracted_keys: HashSet<String>,  // Keys this pattern can extract
}

pub struct DerivedTemplate {
    pub template: String,
    pub dependencies: Vec<String>,  // Attribute names referenced
}
```

### `CompiledFiberType::from_config(name: &str, config: &FiberTypeConfig) -> Result<Self>`

- Compile all regexes
- Build dependency graph for derived attributes
- Topological sort for evaluation order
- Validate (see ticket 02)

## Fiber Session State (`session.rs`)

```rust
pub struct OpenFiber {
    pub fiber_id: Uuid,
    pub fiber_type: String,
    pub keys: HashMap<String, String>,  // key_name -> value
    pub attributes: HashMap<String, AttributeValue>,
    pub first_activity: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub log_ids: Vec<Uuid>,
}

pub enum AttributeValue {
    String(String),
    Int(i64),
    Float(f64),
    // IP and MAC stored as normalized strings
}
```

## Per-Type Processor (`processor.rs`)

```rust
pub struct FiberTypeProcessor {
    fiber_type: CompiledFiberType,
    open_fibers: HashMap<Uuid, OpenFiber>,
    key_index: HashMap<(String, String), Uuid>,  // (key_name, value) -> fiber_id
    logical_clock: DateTime<Utc>,
}
```

### `fn process_log(&mut self, log: &LogRecord) -> ProcessResult`

Implement the algorithm from FIBER_PROCESSING.md:

1. Find matching patterns for this log's source
2. Extract attributes using first matching pattern
3. Compute derived attributes
4. Execute `release_matching_peer_keys` (remove from OTHER fibers)
5. Find matching fibers via key index
6. Create, join, or merge fibers
7. Add keys to fiber, update attributes
8. Execute `release_self_keys`
9. Execute `close` if specified
10. Check for timeout closures (logical_clock - last_activity > max_gap)

```rust
pub struct ProcessResult {
    pub memberships: Vec<FiberMembership>,
    pub new_fibers: Vec<FiberRecord>,
    pub updated_fibers: Vec<FiberRecord>,
    pub closed_fiber_ids: Vec<Uuid>,
}
```

### Attribute Extraction

For each pattern in source's pattern list (in order):
- Try regex match
- If match, extract named capture groups
- First match wins, stop checking patterns

### Derived Attribute Computation

In topological order:
- Check if all dependencies have values
- If yes, interpolate template: `${name}` -> value
- Store result

### Key Index Maintenance

- On adding key: `key_index.insert((name, value), fiber_id)`
- On removing key: `key_index.remove(&(name, value))`
- On fiber merge: update all entries pointing to merged-away fiber

### Fiber Merging

When log matches multiple fibers:
1. Select survivor (oldest first_activity)
2. Merge keys: move all keys from other fibers to survivor
3. Merge attributes: latest wins (by last_activity), log warning on conflict
4. Merge log_ids
5. Update key_index entries
6. Mark other fibers as "merged into" survivor (for storage update)

### Timeout Checking

After processing each log, check all open fibers:
```rust
fn check_timeouts(&mut self) -> Vec<Uuid> {
    let mut closed = vec![];
    for fiber in self.open_fibers.values() {
        if let Some(max_gap) = self.fiber_type.temporal.max_gap {
            if self.logical_clock - fiber.last_activity > max_gap {
                closed.push(fiber.fiber_id);
            }
        }
    }
    // Remove closed fibers from open_fibers and key_index
    closed
}
```

## Multi-Type Coordinator

```rust
pub struct FiberProcessor {
    processors: HashMap<String, FiberTypeProcessor>,
}

impl FiberProcessor {
    pub fn process_log(&mut self, log: &LogRecord) -> Vec<ProcessResult> {
        // Process in parallel (rayon) or sequentially
        self.processors
            .values_mut()
            .map(|p| p.process_log(log))
            .collect()
    }
}
```

For MVP, sequential is fine. Parallel can be added later with rayon.

## Acceptance Criteria

- Logs correctly assigned to fibers based on keys
- Fiber merging works when log matches multiple fibers
- `release_matching_peer_keys` prevents old fibers from capturing new requests
- `release_self_keys` releases keys without closing fiber
- `close: true` closes fiber and releases all keys
- Timeout-based closure works with logical clock
- Different fiber types process independently
