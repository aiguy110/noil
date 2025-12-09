# 04: Source Reader

Implement log file reading with multiline coalescing and offset tracking.

## Location

`src/source/reader.rs`

## Core Types

```rust
pub struct LogRecord {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
    pub raw_text: String,
    pub file_offset: u64,  // For checkpointing
}

pub struct SourceReader {
    source_id: String,
    path: PathBuf,
    timestamp_extractor: TimestampExtractor,
    read_config: ReadConfig,

    // Internal state
    file: Option<File>,
    current_offset: u64,
    buffered_line: Option<BufferedLine>,
}

struct BufferedLine {
    text: String,
    timestamp: DateTime<Utc>,
    start_offset: u64,
}
```

## Implementation

### `SourceReader::new(source_id: String, config: &SourceConfig) -> Result<Self>`

Initialize reader from config. Don't open file yet.

### `SourceReader::open(&mut self) -> Result<()>`

- Open file
- Seek based on `read_config.start`:
  - `beginning`: seek to 0
  - `end`: seek to end
  - `stored_offset`: seek to provided offset (for checkpoint restore)

### `async fn next_record(&mut self) -> Result<Option<LogRecord>>`

Core reading loop:

1. Read next line from file
2. Try to extract timestamp
3. If timestamp found:
   - If we have a buffered line, emit it as a LogRecord
   - Buffer this new line
4. If no timestamp (continuation line):
   - Append to buffered line's text (with newline)
5. On EOF:
   - If `follow: true`, wait and retry (use `tokio::time::sleep` or inotify)
   - If `follow: false`, emit buffered line if any, return None

### Multiline Handling

Lines without timestamps are continuations of the previous line:

```
2025-12-04T02:42:11Z Starting process
  Stack trace line 1
  Stack trace line 2
2025-12-04T02:42:12Z Process complete
```

The first log record should contain all three lines of the stack trace.

### File Following

For `follow: true`, after EOF:
- Sleep briefly (100ms)
- Check if file has grown
- Also handle file rotation: if inode changes, reopen from beginning

Simple implementation for MVP: just poll with sleep. Can optimize with inotify later.

### Offset Tracking

Track byte offset for checkpointing:
- `start_offset`: position where this record began
- After emitting a record, update `current_offset`

### Watermark

Each reader should track its latest emitted timestamp as a watermark:

```rust
pub fn watermark(&self) -> Option<DateTime<Utc>>
```

Returns the timestamp of the last emitted record, or None if nothing emitted yet.

## Error Handling

- File not found: return error (don't silently wait)
- Read errors: return error
- Permission denied: return error
- Parse errors: based on config (`drop` or `panic`)

## Integration Point

The reader will be wrapped in an async stream for the sequencer:

```rust
impl SourceReader {
    pub fn into_stream(self) -> impl Stream<Item = Result<LogRecord>>
}
```

Use `async_stream` crate or manual `Stream` impl.

## Acceptance Criteria

- Reads log files and extracts records with timestamps
- Multiline records coalesced correctly
- Offset tracking enables resume from checkpoint
- Follow mode detects new lines
- Handles file rotation (inode change)
