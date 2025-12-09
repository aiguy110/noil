# 05: Sequencer

Implement the local sequencer that merges multiple source streams into global timestamp order.

## Location

`src/sequencer/local.rs`, `src/sequencer/merge.rs`

## Core Concept

The sequencer receives log records from multiple sources and emits them in timestamp order. It uses watermarks to determine when it's safe to emit.

**Watermark rule**: A record can be emitted when its timestamp is less than the minimum watermark across all sources.

## Types

```rust
pub struct Sequencer {
    sources: HashMap<String, SourceState>,
    heap: BinaryHeap<Reverse<HeapEntry>>,
    safety_margin: Duration,
}

struct SourceState {
    watermark: Option<DateTime<Utc>>,
    active: bool,
}

struct HeapEntry {
    timestamp: DateTime<Utc>,
    record: LogRecord,
}

impl Ord for HeapEntry {
    // Order by timestamp (min-heap via Reverse)
}
```

## Implementation

### `Sequencer::new(source_ids: Vec<String>, safety_margin: Duration) -> Self`

Initialize with known source IDs. All start with `watermark: None, active: true`.

### `fn push(&mut self, record: LogRecord)`

Add a record to the heap.

### `fn update_watermark(&mut self, source_id: &str, watermark: DateTime<Utc>)`

Update the watermark for a source. Called after a source emits a record.

### `fn mark_source_done(&mut self, source_id: &str)`

Mark a source as inactive (EOF with no follow). Its watermark is no longer considered in the minimum calculation.

### `fn emit_ready(&mut self) -> Vec<LogRecord>`

Emit all records that are safe to emit:

```rust
fn emit_ready(&mut self) -> Vec<LogRecord> {
    let min_watermark = self.compute_min_watermark();
    let Some(threshold) = min_watermark else {
        return vec![]; // No watermarks yet, can't emit
    };

    let threshold = threshold - self.safety_margin;
    let mut result = vec![];

    while let Some(Reverse(entry)) = self.heap.peek() {
        if entry.timestamp <= threshold {
            result.push(self.heap.pop().unwrap().0.record);
        } else {
            break;
        }
    }
    result
}

fn compute_min_watermark(&self) -> Option<DateTime<Utc>> {
    self.sources
        .values()
        .filter(|s| s.active)
        .filter_map(|s| s.watermark)
        .min()
}
```

### `fn flush_all(&mut self) -> Vec<LogRecord>`

Emit all remaining records (called at shutdown or when all sources done).

## Async Wrapper

Create an async runner that:

1. Spawns a task per source reader
2. Receives records via channel
3. Pushes to sequencer, updates watermarks
4. Periodically calls `emit_ready()` and sends to downstream

```rust
pub async fn run_sequencer(
    sources: Vec<SourceReader>,
    output: mpsc::Sender<LogRecord>,
    config: SequencerConfig,
) -> Result<()>
```

Use `tokio::select!` to handle multiple source channels.

## Watermark Strategy

After a source emits record with timestamp T:
- Update that source's watermark to T
- This means "source won't emit anything with timestamp < T"

For MVP, this simple approach works. The `safety_margin` config handles clock skew.

## Edge Cases

- **Source with no records yet**: watermark is None, blocks emission from all sources
- **Source completes (EOF, no follow)**: mark inactive, no longer blocks
- **All sources complete**: flush remaining heap contents
- **Out-of-order within source**: log warning but accept (source reader should prevent this)

## Channel Backpressure

Use bounded channels. If downstream is slow, sequencer blocks on send. This propagates backpressure to source readers.

## Acceptance Criteria

- Records emitted in timestamp order (within safety margin tolerance)
- Multiple sources merged correctly
- Watermark logic prevents emitting records that might be reordered
- Handles source completion gracefully
- Bounded memory usage (backpressure works)
