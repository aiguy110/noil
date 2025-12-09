# 03: Timestamp Parsing

Implement timestamp extraction from log lines.

## Location

`src/source/timestamp.rs`

## Core Types

```rust
pub struct TimestampExtractor {
    pattern: Regex,
    format: TimestampFormat,
}

pub enum TimestampFormat {
    Strptime(String),
    Iso8601,
    Epoch,
    EpochMs,
}
```

## Implementation

### `TimestampExtractor::new(pattern: &str, format: &str) -> Result<Self, TimestampError>`

- Compile regex from pattern
- Validate regex has a capture group named `ts`
- Parse format string into `TimestampFormat` enum

### `TimestampExtractor::extract(&self, line: &str) -> Option<DateTime<Utc>>`

- Apply regex to line
- If no match, return `None`
- Extract the `ts` capture group
- Parse according to format:
  - `Strptime`: use chrono's strptime parsing
  - `Iso8601`: use `DateTime::parse_from_rfc3339` or similar
  - `Epoch`: parse as i64, convert to DateTime
  - `EpochMs`: parse as i64, divide by 1000 for seconds

Handle timezone-naive timestamps by assuming UTC.

## Strptime Considerations

Chrono's `NaiveDateTime::parse_from_str` uses strptime-like format codes. Common patterns:

- `%Y-%m-%d %H:%M:%S` - standard datetime
- `%d/%b/%Y:%H:%M:%S %z` - nginx combined log format
- `%Y-%m-%dT%H:%M:%S%.fZ` - ISO with fractional seconds

If format includes `%z`, parse as `DateTime<FixedOffset>` then convert to UTC.
If no timezone, parse as `NaiveDateTime` and assume UTC.

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum TimestampError {
    #[error("regex compilation failed: {0}")]
    InvalidRegex(#[from] regex::Error),

    #[error("pattern missing 'ts' capture group")]
    MissingTsGroup,

    #[error("failed to parse timestamp '{value}' with format '{format}': {source}")]
    ParseError {
        value: String,
        format: String,
        source: chrono::ParseError,
    },
}
```

## Unit Tests

Test cases:

1. ISO8601: `2025-12-04T02:42:11.011Z`
2. ISO8601 with offset: `2025-12-04T02:42:11+05:30`
3. Nginx format: `[04/Dec/2025:02:42:11 +0000]`
4. Epoch seconds: `1733280131`
5. Epoch milliseconds: `1733280131011`
6. Custom strptime: `2025-12-04 02:42:11,011` (comma for millis)
7. Line with no timestamp match returns `None`
8. Line with unparseable timestamp value returns appropriate error

## Acceptance Criteria

- All test cases pass
- Consistent UTC output regardless of input timezone
- Clear errors for invalid patterns/formats
