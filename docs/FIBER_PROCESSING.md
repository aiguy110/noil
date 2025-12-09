# Fiber Processing: Semantics and Logic

This document explains the reasoning behind Noil's fiber correlation model and provides detailed semantics for how logs are grouped into fibers.

## The Core Problem

Correlating logs across multiple sources is challenging because:

1. **Different identifiers in different sources**: A request might be identified by `thread-5` in one service and `thread-69` in another, connected only by a shared MAC address or request ID that appears in some (but not all) log lines.

2. **Identifier reuse**: Thread IDs, connection IDs, and similar identifiers are frequently reused. `thread-5` might handle request A, then immediately handle request B. Naive correlation would incorrectly merge these.

3. **Delayed correlation**: The identifier that connects logs across sources (e.g., a MAC address) might not appear until partway through a request. Earlier logs need to be retrospectively associated.

4. **Temporal proximity matters**: Logs from the same logical operation are typically close in time, but exact timing varies.

## The Solution: Attributes, Keys, and Explicit Session Control

Noil uses a simple but powerful model:

### Attributes

Attributes are named values associated with fibers. They come in two forms:

**Extracted Attributes**: Captured directly from log lines via regex named capture groups.

**Derived Attributes**: Computed from other attributes via string interpolation. For example:

```yaml
- name: connection_id
  type: string
  derived: "${src_ip}:${src_port}->${dst_ip}:${dst_port}"
```

A derived attribute is only defined when all referenced attributes have values. Derived attributes can reference other derived attributes, provided there are no circular dependencies (validated at config load time).

A derived attribute with no `${}` references is a static value, always defined:

```yaml
- name: source_marker
  type: string
  derived: "my_static_value"
```

This is useful for single-threaded logs where you want consecutive lines grouped until a time gap or explicit close.

### Keys

Any attribute can be designated as a **key** by setting `key: true`. Keys enable fiber matching and merging:

- When a log is processed, its extracted/derived keys are compared against keys on open fibers
- If any key matches, the log joins that fiber
- If keys match multiple fibers, those fibers are merged
- Keys only exist while a fiber is open; when a fiber closes, its keys are released

Attributes persist on the fiber even after close; keys do not.

### Fiber Identity

Each fiber is identified by a UUID assigned at creation. This provides stable identity regardless of which keys come and go during the fiber's lifetime.

### Fiber Lifecycle

A fiber can be **open** or **closed**:

- **Open**: Actively accepting logs, has keys that can match incoming logs
- **Closed**: No longer accepts logs, keys released, attributes retained for querying

Closing happens via:
- **Temporal gap**: No matching logs for `max_gap` duration
- **Explicit close**: A pattern with `close: true` matches

## Pattern-Level Session Control

Patterns can specify actions that control key lifecycle:

### `release_matching_peer_keys`

```yaml
- regex: 'thread-(?P<thread_id>\d+) Received request'
  release_matching_peer_keys: [thread_id]
```

Before processing this log:
1. For each key listed in `release_matching_peer_keys`
2. If this pattern extracts a value for that key
3. Remove that `(key_name, value)` pair from all **other** open fibers

This is useful for "request start" lines. When `thread-5` starts a new request, any existing fiber holding `thread_id=5` should release it, preventing the new request from merging with the old one.

**Validation**: Keys in `release_matching_peer_keys` must be extracted by this pattern (otherwise there's no value to match).

### `release_self_keys`

```yaml
- regex: 'thread-(?P<thread_id>\d+) Request complete'
  release_self_keys: [thread_id]
```

After processing this log:
1. For each key listed in `release_self_keys`
2. Remove that key from **this** fiber (regardless of value)

This is useful for "request end" lines. The fiber releases the thread ID so future logs with that thread ID don't incorrectly join this fiber.

**Note**: Unlike `release_matching_peer_keys`, the value doesn't need to be extracted by this pattern. The key is removed by name.

**Validation**: Keys in `release_self_keys` must be marked `key: true` in the attributes list.

### `close`

```yaml
- regex: 'Request complete'
  close: true
```

After processing this log, close the fiber. This releases all keys and prevents the fiber from accepting more logs.

## Processing Algorithm

When a log arrives:

```
1. Extract attributes from log using configured patterns
2. Compute derived attributes (if all dependencies satisfied)
3. Identify keys (attributes marked key: true that have values)
4. Execute release_matching_peer_keys:
   - For each key in the list that was extracted by this pattern:
     - Find all OTHER open fibers with that (key, value)
     - Remove the key from those fibers
5. Find matching fibers:
   - For each extracted key, check if any open fiber has that (key, value)
   - Collect all matching fiber IDs
6. Determine target fiber:
   - If no matches: create new fiber (assign UUID)
   - If one match: use that fiber
   - If multiple matches: merge all into one fiber
7. Add extracted keys to the fiber
8. Store extracted/derived attributes on fiber
   - If attribute already exists with different value: overwrite (latest wins)
   - Log a warning when overwriting with different value
9. Record log's membership in fiber
10. Execute release_self_keys:
    - For each key in the list:
      - Remove that key from this fiber (by name, regardless of value)
11. If close: true:
    - Close the fiber (release all keys, mark as closed)
```

## Fiber Merging

When a log matches multiple open fibers of the same type (via different keys), those fibers must be merged. Fibers of different types are never merged—they are processed completely independently.

### Merge Process

1. Select one fiber as the "survivor" (oldest by creation time)
2. Move all keys from other fibers to survivor
3. Move all attributes from other fibers to survivor
   - On conflict (same attribute, different value): latest log timestamp wins
   - **Log a warning** when conflicts occur
4. Update all log memberships to point to survivor
5. Delete the other fiber records

### Example

```
Fiber F1: keys={(mac, aa:bb:cc)}, attrs={ip: 10.0.0.1}
Fiber F2: keys={(request_id, xyz)}, attrs={ip: 10.0.0.2}

Log arrives: "MAC aa:bb:cc for request xyz"
  → Extracts mac=aa:bb:cc, request_id=xyz
  → Matches F1 via mac
  → Matches F2 via request_id
  → Merge F1 and F2

Result: F1 (survivor)
  keys={(mac, aa:bb:cc), (request_id, xyz)}
  attrs={ip: 10.0.0.2}  # Latest wins, warning logged
```

## Key Lifecycle

Keys exist only while a fiber is open. They are removed when:

1. **Fiber closes** (timeout or explicit `close: true`): All keys released
2. **`release_self_keys`**: Specified keys removed from this fiber
3. **`release_matching_peer_keys`**: Matching keys removed from other fibers
4. **Overwritten**: If a new log extracts a different value for the same key name, the old value is replaced (the old value is no longer in the key index)

### Key Index

The fiber processor maintains a global index:

```
key_index: HashMap<(KeyName, Value), FiberId>
```

This enables O(1) lookup when a log extracts a key. When keys are released or fibers merge, the index is updated.

## Attribute Lifecycle

Unlike keys, attributes persist for the lifetime of the fiber (including after close). They are:

- Added when extracted/derived from logs
- Updated if a later log extracts a different value (latest wins, with warning)
- Retained when fiber closes
- Merged when fibers merge (latest wins on conflict, with warning)

## Temporal Constraints

### Fiber-Level `max_gap`

Controls when a fiber closes due to inactivity:

- **Session mode** (`gap_mode: session`): Fiber closes when `logical_clock - fiber.last_activity > max_gap`
- **From-start mode** (`gap_mode: from_start`): Fiber closes when `logical_clock - fiber.first_activity > max_gap`

### Infinite `max_gap`

Setting `max_gap: infinite` creates fibers that never close due to time. Useful for:
- Per-source "catch-all" fibers as navigation entry points
- Correlation by long-lived identifiers

## Worked Example

### Configuration

```yaml
fiber_types:
  request_trace:
    temporal:
      max_gap: 5s
      gap_mode: session
    attributes:
      - name: mac
        type: mac
        key: true
      - name: program1_thread
        type: string
        key: true
      - name: program2_thread
        type: string
        key: true
      - name: ip
        type: ip
    sources:
      program1:
        patterns:
          - regex: 'thread-(?P<program1_thread>\d+) Received.*from (?P<ip>\d+\.\d+\.\d+\.\d+)'
            release_matching_peer_keys: [program1_thread]
          - regex: 'thread-(?P<program1_thread>\d+).*MAC (?P<mac>[0-9a-f:]+)'
          - regex: 'thread-(?P<program1_thread>\d+)'
      program2:
        patterns:
          - regex: 'thread-(?P<program2_thread>\d+).*MAC (?P<mac>[0-9a-f:]+)'
          - regex: 'thread-(?P<program2_thread>\d+)'
```

### Log Files

**program1.log:**
```
2025-12-04T02:42:11,011 thread-5 Received important data from 10.10.10.42
2025-12-04T02:42:11,013 thread-5 Processing. IP associated with MAC aa:bb:cc:11:22:33
2025-12-04T02:42:11,014 thread-5 Sending results to program2
2025-12-04T02:42:11,031 thread-5 Received important data from 10.10.10.24
2025-12-04T02:42:11,033 thread-5 Processing. IP associated with MAC dd:ee:ff:44:55:66
2025-12-04T02:42:11,034 thread-5 Sending results to program2
```

**program2.log:**
```
2025-12-04T02:42:11,021 thread-69 Received data about MAC aa:bb:cc:11:22:33
2025-12-04T02:42:11,023 thread-69 Doing important stuff
2025-12-04T02:42:11,041 thread-96 Received data about MAC dd:ee:ff:44:55:66
2025-12-04T02:42:11,043 thread-96 Doing important stuff
```

### Processing Trace

Logs are processed in global timestamp order (sequencer output):

| Time | Source | Log Summary | Extracted | Action |
|------|--------|-------------|-----------|--------|
| :011 | prog1 | thread-5 Received from 10.10.10.42 | program1_thread=5, ip=10.10.10.42 | `release_matching_peer_keys`: remove (program1_thread,5) from others (no-op). No match → create **F1** with keys={(p1_thread,5)}, attrs={ip:10.10.10.42} |
| :013 | prog1 | thread-5 MAC aa:bb:cc | program1_thread=5, mac=aa:bb:cc | Match F1 via (p1_thread,5). Add key (mac,aa:bb:cc). F1: keys={(p1_thread,5),(mac,aa:bb:cc)} |
| :014 | prog1 | thread-5 Sending | program1_thread=5 | Match F1 via (p1_thread,5). No new keys. |
| :021 | prog2 | thread-69 MAC aa:bb:cc | program2_thread=69, mac=aa:bb:cc | Match F1 via (mac,aa:bb:cc). Add key (p2_thread,69). F1: keys={(p1_thread,5),(mac,aa:bb:cc),(p2_thread,69)} |
| :023 | prog2 | thread-69 stuff | program2_thread=69 | Match F1 via (p2_thread,69). |
| :031 | prog1 | thread-5 Received from 10.10.10.24 | program1_thread=5, ip=10.10.10.24 | `release_matching_peer_keys`: **remove (p1_thread,5) from F1**. F1: keys={(mac,aa:bb:cc),(p2_thread,69)}. No match → create **F2** with keys={(p1_thread,5)}, attrs={ip:10.10.10.24} |
| :033 | prog1 | thread-5 MAC dd:ee:ff | program1_thread=5, mac=dd:ee:ff | Match F2 via (p1_thread,5). Add key (mac,dd:ee:ff). |
| :034 | prog1 | thread-5 Sending | program1_thread=5 | Match F2 via (p1_thread,5). |
| :041 | prog2 | thread-96 MAC dd:ee:ff | program2_thread=96, mac=dd:ee:ff | Match F2 via (mac,dd:ee:ff). Add key (p2_thread,96). |
| :043 | prog2 | thread-96 stuff | program2_thread=96 | Match F2 via (p2_thread,96). |

### Result

**Fiber F1** (5 logs):
- Keys: `{(mac, aa:bb:cc:11:22:33), (program2_thread, 69)}`
- Attributes: `{mac: aa:bb:cc:11:22:33, program1_thread: 5, program2_thread: 69, ip: 10.10.10.42}`
- Note: `program1_thread` is still an attribute even though the key was released

**Fiber F2** (5 logs):
- Keys: `{(program1_thread, 5), (mac, dd:ee:ff:44:55:66), (program2_thread, 96)}`
- Attributes: `{mac: dd:ee:ff:44:55:66, program1_thread: 5, program2_thread: 96, ip: 10.10.10.24}`

The two requests are correctly separated despite sharing `thread-5` in program1.

## Data Structures

### Fiber Processor State

```rust
struct FiberProcessor {
    fiber_types: Vec<CompiledFiberType>,
    
    // Active (open) fibers
    open_fibers: HashMap<FiberId, OpenFiber>,
    
    // Key index: (key_name, value) → fiber_id
    key_index: HashMap<(String, String), FiberId>,
    
    // Logical clock (timestamp of last processed log)
    logical_clock: DateTime<Utc>,
}

struct OpenFiber {
    fiber_id: FiberId,  // UUID
    fiber_type: String,
    
    // Current keys (used for matching)
    keys: HashMap<String, String>,  // key_name → value
    
    // All attributes (persist after close)
    attributes: HashMap<String, AttributeValue>,
    
    // Timing
    first_activity: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    
    // Log membership
    log_ids: Vec<LogId>,
}
```

### Compiled Fiber Type

```rust
struct CompiledFiberType {
    name: String,
    temporal: TemporalConfig,
    
    // Attribute definitions
    attributes: Vec<AttributeDef>,
    
    // Which attributes are keys
    key_attributes: HashSet<String>,
    
    // Derived attribute computation order (topologically sorted)
    derived_order: Vec<String>,
    derived_templates: HashMap<String, DerivedTemplate>,
    
    // Per-source patterns
    sources: HashMap<String, Vec<CompiledPattern>>,
}

struct CompiledPattern {
    regex: Regex,
    release_matching_peer_keys: Vec<String>,
    release_self_keys: Vec<String>,
    close: bool,
}
```

## Validation Rules

At configuration load time, validate:

1. **No circular derived dependencies**: Build dependency graph, check for cycles
2. **`release_matching_peer_keys` are extractable**: Each key listed must be a capture group in the pattern's regex
3. **`release_self_keys` are keys**: Each key listed must have `key: true` in attributes
4. **`release_matching_peer_keys` are keys**: Each key listed must have `key: true` in attributes  
5. **Derived references exist**: All `${name}` references must correspond to defined attributes
6. **No duplicate attribute names**: Within a fiber type, attribute names must be unique

## Edge Cases

### Fiber With No Keys

A fiber can exist with no keys (all released via `release_self_keys` or peer releases). It's still open and can have logs added if:
- A future log extracts a key that gets added to this fiber (via some other matching mechanism)
- Actually, without keys there's no way to match... 

If all keys are released from an open fiber, subsequent logs can't match it. The fiber will eventually close due to `max_gap`. This is expected behavior — the fiber represents a completed logical operation.

### Key Value Changes

If a fiber has `thread_id=5` and a new log (matched via another key like `mac`) extracts `thread_id=7`:
- The attribute `thread_id` is updated to `7` (latest wins, warning logged)
- The key index is updated: `(thread_id, 5)` removed, `(thread_id, 7)` added
- Future logs with `thread_id=5` won't match this fiber

### Same Key, Multiple Fibers

The key index maps `(key_name, value)` → single `FiberId`. If two fibers somehow both have the same key (shouldn't happen due to merge logic), the index only tracks one. This is a bug if it occurs.

### Fiber Closes With Pending Keys

When a fiber closes (timeout or explicit), all its keys are removed from the key index. The keys are effectively released into the pool for new fibers.

### Multiple Fiber Types

A single log can match multiple fiber types. Each fiber type is processed completely independently:

- Separate key indexes per fiber type
- No merging across fiber types
- A log can belong to multiple fibers of different types simultaneously

This independence means fiber processing can be parallelized across fiber types with no synchronization required.
