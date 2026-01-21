use crate::config::types::{ParseErrorStrategy, ReadConfig, ReadStart, SourceConfig};
use crate::source::timestamp::{TimestampError, TimestampExtractor};
use crate::storage::checkpoint::SharedSourceState;
use chrono::{DateTime, Utc};
use futures::Future;
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("timestamp error: {0}")]
    Timestamp(#[from] TimestampError),

    #[error("parse error: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone)]
pub struct LogRecord {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
    pub raw_text: String,
    pub file_offset: u64,
}

#[derive(Debug)]
struct BufferedLine {
    text: String,
    timestamp: DateTime<Utc>,
    start_offset: u64,
}

pub struct SourceReader {
    source_id: String,
    path: PathBuf,
    timestamp_extractor: TimestampExtractor,
    read_config: ReadConfig,
    parse_error_strategy: ParseErrorStrategy,

    // Internal state
    file: Option<BufReader<File>>,
    current_offset: u64,
    buffered_line: Option<BufferedLine>,
    last_watermark: Option<DateTime<Utc>>,
    last_emitted_offset: u64, // Offset after the last emitted record (for checkpointing)
    file_inode: Option<u64>,
    eof_reached: bool,

    // Shared state for checkpointing
    shared_state: Option<SharedSourceState>,
}

impl SourceReader {
    /// Create a new SourceReader from config
    pub fn new(
        source_id: String,
        config: &SourceConfig,
        parse_error_strategy: ParseErrorStrategy,
    ) -> Result<Self, ReaderError> {
        let timestamp_extractor =
            TimestampExtractor::new(&config.timestamp.pattern, &config.timestamp.format)?;

        Ok(Self {
            source_id,
            path: config.path.clone(),
            timestamp_extractor,
            read_config: config.read.clone(),
            parse_error_strategy,
            file: None,
            current_offset: 0,
            buffered_line: None,
            last_watermark: None,
            last_emitted_offset: 0,
            file_inode: None,
            eof_reached: false,
            shared_state: None,
        })
    }

    /// Create a new SourceReader with explicit offset for checkpoint restore
    pub fn new_with_offset(
        source_id: String,
        config: &SourceConfig,
        parse_error_strategy: ParseErrorStrategy,
        offset: u64,
    ) -> Result<Self, ReaderError> {
        let mut reader = Self::new(source_id, config, parse_error_strategy)?;
        reader.current_offset = offset;
        reader.last_emitted_offset = offset;
        // Override read_config.start to use the restored offset instead of the config value.
        // This ensures open() seeks to the checkpoint offset rather than beginning/end.
        reader.read_config.start = crate::config::types::ReadStart::StoredOffset;
        Ok(reader)
    }

    /// Get the source ID
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Open the file and seek to the appropriate position
    pub fn open(&mut self) -> Result<(), ReaderError> {
        let file = File::open(&self.path)?;
        let metadata = file.metadata()?;
        self.file_inode = Some(get_inode(&metadata));

        let mut buf_reader = BufReader::new(file);

        // Seek based on read config
        match self.read_config.start {
            ReadStart::Beginning => {
                buf_reader.seek(SeekFrom::Start(0))?;
                self.current_offset = 0;
            }
            ReadStart::End => {
                let end = buf_reader.seek(SeekFrom::End(0))?;
                self.current_offset = end;
            }
            ReadStart::StoredOffset => {
                buf_reader.seek(SeekFrom::Start(self.current_offset))?;
            }
        }

        self.file = Some(buf_reader);
        self.eof_reached = false;

        // Update shared state with the file inode so checkpoints capture it
        // even if no records are emitted (e.g., when resuming at EOF)
        self.update_shared_state();

        Ok(())
    }

    /// Read the next log record
    pub async fn next_record(&mut self) -> Result<Option<LogRecord>, ReaderError> {
        loop {
            // Ensure file is open
            if self.file.is_none() {
                self.open()?;
            }

            // Try to read next line
            let mut line = String::new();
            let bytes_read = {
                let file = self.file.as_mut().unwrap();
                file.read_line(&mut line)?
            };

            // Handle EOF
            if bytes_read == 0 {
                self.eof_reached = true;

                // If we have a buffered line, emit it
                if let Some(buffered) = self.buffered_line.take() {
                    let record = LogRecord {
                        id: Uuid::new_v4(),
                        timestamp: buffered.timestamp,
                        source_id: self.source_id.clone(),
                        raw_text: buffered.text,
                        file_offset: buffered.start_offset,
                    };
                    self.last_watermark = Some(record.timestamp);
                    // Update checkpoint offset to current position (EOF reached)
                    self.last_emitted_offset = self.current_offset;
                    self.update_shared_state();
                    return Ok(Some(record));
                }

                // Check if we should follow
                if self.read_config.follow {
                    // Check for file rotation
                    if self.check_file_rotation()? {
                        // File was rotated, reopen from beginning
                        self.file = None;
                        self.current_offset = 0;
                        self.eof_reached = false;
                        continue;
                    }

                    // Wait briefly for new content
                    sleep(Duration::from_millis(100)).await;

                    // Check if file has grown
                    if let Some(file) = &mut self.file {
                        let current_pos = file.stream_position()?;
                        let file_len = file.get_ref().metadata()?.len();
                        if file_len > current_pos {
                            // File has grown, continue reading
                            self.eof_reached = false;
                            continue;
                        }
                    }

                    continue;
                } else {
                    // Not following, we're done
                    return Ok(None);
                }
            }

            // Update offset
            let line_start_offset = self.current_offset;
            self.current_offset += bytes_read as u64;

            // Remove trailing newline
            let line = line.trim_end_matches(&['\n', '\r'][..]).to_string();

            // Try to extract timestamp
            match self.timestamp_extractor.extract(&line) {
                Ok(Some(timestamp)) => {
                    // This is a new log line with timestamp
                    // If we have a buffered line, emit it first
                    if let Some(buffered) = self.buffered_line.take() {
                        let record = LogRecord {
                            id: Uuid::new_v4(),
                            timestamp: buffered.timestamp,
                            source_id: self.source_id.clone(),
                            raw_text: buffered.text,
                            file_offset: buffered.start_offset,
                        };
                        self.last_watermark = Some(record.timestamp);

                        // Update checkpoint offset to point after this emitted record
                        self.last_emitted_offset = line_start_offset;
                        self.update_shared_state();

                        // Buffer the new line
                        self.buffered_line = Some(BufferedLine {
                            text: line,
                            timestamp,
                            start_offset: line_start_offset,
                        });

                        return Ok(Some(record));
                    } else {
                        // No buffered line, just buffer this one
                        self.buffered_line = Some(BufferedLine {
                            text: line,
                            timestamp,
                            start_offset: line_start_offset,
                        });
                    }
                }
                Ok(None) => {
                    // No timestamp found - check if this looks like a continuation line
                    // Continuation lines typically start with whitespace
                    let is_continuation = line.starts_with(char::is_whitespace);

                    if is_continuation {
                        if let Some(buffered) = &mut self.buffered_line {
                            // Append to buffered line
                            buffered.text.push('\n');
                            buffered.text.push_str(&line);
                        } else {
                            // No buffered line to append to
                            // This is a malformed log (continuation without initial line)
                            match self.parse_error_strategy {
                                ParseErrorStrategy::Drop => {
                                    // Skip this line
                                    continue;
                                }
                                ParseErrorStrategy::Panic => {
                                    return Err(ReaderError::ParseError(format!(
                                        "continuation line without initial timestamp: {}",
                                        line
                                    )));
                                }
                            }
                        }
                    } else {
                        // Not a continuation line and no timestamp - parse error
                        match self.parse_error_strategy {
                            ParseErrorStrategy::Drop => {
                                // Skip this line
                                continue;
                            }
                            ParseErrorStrategy::Panic => {
                                return Err(ReaderError::ParseError(format!(
                                    "line without timestamp: {}",
                                    line
                                )));
                            }
                        }
                    }
                }
                Err(e) => {
                    // Timestamp extraction failed
                    match self.parse_error_strategy {
                        ParseErrorStrategy::Drop => {
                            // Skip this line
                            continue;
                        }
                        ParseErrorStrategy::Panic => {
                            return Err(ReaderError::Timestamp(e));
                        }
                    }
                }
            }
        }
    }

    /// Get the watermark (timestamp of last emitted record)
    pub fn watermark(&self) -> Option<DateTime<Utc>> {
        self.last_watermark
    }

    /// Get the checkpoint offset (byte position to resume from after last emitted record)
    pub fn checkpoint_offset(&self) -> u64 {
        self.last_emitted_offset
    }

    /// Get the file inode for checkpoint purposes
    pub fn file_inode(&self) -> Option<u64> {
        self.file_inode
    }

    /// Get the file path
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Attach shared state for checkpointing and return both the reader and the state
    pub fn with_shared_state(mut self) -> (Self, SharedSourceState) {
        use crate::storage::checkpoint::SourceCheckpointState;
        use std::sync::{Arc, Mutex};

        let state = Arc::new(Mutex::new(SourceCheckpointState {
            offset: self.last_emitted_offset,
            inode: self.file_inode.unwrap_or(0),
            last_timestamp: self.last_watermark,
        }));
        self.shared_state = Some(state.clone());
        (self, state)
    }

    /// Update the shared state with current checkpoint values
    fn update_shared_state(&self) {
        if let Some(ref state) = self.shared_state {
            if let Ok(mut guard) = state.lock() {
                guard.offset = self.last_emitted_offset;
                guard.inode = self.file_inode.unwrap_or(0);
                guard.last_timestamp = self.last_watermark;
            }
        }
    }

    /// Check if the file has been rotated (inode changed)
    fn check_file_rotation(&self) -> Result<bool, ReaderError> {
        if let Some(original_inode) = self.file_inode {
            match std::fs::metadata(&self.path) {
                Ok(metadata) => {
                    let current_inode = get_inode(&metadata);
                    Ok(current_inode != original_inode)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // File doesn't exist yet, not rotated
                    Ok(false)
                }
                Err(e) => Err(e.into()),
            }
        } else {
            Ok(false)
        }
    }

    /// Convert into a stream
    pub fn into_stream(self) -> SourceReaderStream {
        SourceReaderStream { reader: self }
    }
}

/// Stream wrapper for SourceReader
pub struct SourceReaderStream {
    reader: SourceReader,
}

impl futures::Stream for SourceReaderStream {
    type Item = Result<LogRecord, ReaderError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Create a future for next_record and poll it
        let fut = self.reader.next_record();
        tokio::pin!(fut);

        match fut.poll(cx) {
            Poll::Ready(Ok(Some(record))) => Poll::Ready(Some(Ok(record))),
            Poll::Ready(Ok(None)) => Poll::Ready(None),
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

// Platform-specific inode retrieval
#[cfg(unix)]
fn get_inode(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

#[cfg(not(unix))]
fn get_inode(metadata: &Metadata) -> u64 {
    // On non-Unix platforms, use file size and modified time as a proxy
    // This is not perfect but better than nothing
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    metadata.len().hash(&mut hasher);
    if let Ok(modified) = metadata.modified() {
        modified.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{SourceConfig, SourceType, TimestampConfig};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_config(path: PathBuf, pattern: &str, format: &str) -> SourceConfig {
        SourceConfig {
            source_type: SourceType::File,
            path,
            timestamp: TimestampConfig {
                pattern: pattern.to_string(),
                format: format.to_string(),
            },
            read: ReadConfig {
                start: ReadStart::Beginning,
                follow: false,
            },
        }
    }

    #[tokio::test]
    async fn test_single_line_log() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z First log line").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Second log line").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Read first record
        let record1 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record1.source_id, "test");
        assert_eq!(record1.raw_text, "2025-12-04T10:00:00Z First log line");
        assert_eq!(record1.file_offset, 0);

        // Read second record
        let record2 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record2.raw_text, "2025-12-04T10:00:01Z Second log line");

        // No more records
        let record3 = reader.next_record().await.unwrap();
        assert!(record3.is_none());
    }

    #[tokio::test]
    async fn test_multiline_log() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z Starting process").unwrap();
        writeln!(temp_file, "  Stack trace line 1").unwrap();
        writeln!(temp_file, "  Stack trace line 2").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Process complete").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Read first record (should include continuation lines)
        let record1 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(
            record1.raw_text,
            "2025-12-04T10:00:00Z Starting process\n  Stack trace line 1\n  Stack trace line 2"
        );

        // Read second record
        let record2 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record2.raw_text, "2025-12-04T10:00:01Z Process complete");

        // No more records
        let record3 = reader.next_record().await.unwrap();
        assert!(record3.is_none());
    }

    #[tokio::test]
    async fn test_watermark() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z First").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Second").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Initially no watermark
        assert!(reader.watermark().is_none());

        // After reading first record
        let record1 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(reader.watermark().unwrap(), record1.timestamp);

        // After reading second record
        let record2 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(reader.watermark().unwrap(), record2.timestamp);
    }

    #[tokio::test]
    async fn test_offset_tracking() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z First").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Second").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Read first record
        let record1 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record1.file_offset, 0);

        // Read second record
        let record2 = reader.next_record().await.unwrap().unwrap();
        // Second record should start where first record ends
        // First line is "2025-12-04T10:00:00Z First\n" = 27 bytes
        assert_eq!(record2.file_offset, 27);

        // Verify offsets are different (each record has its own starting position)
        assert!(record2.file_offset > record1.file_offset);
    }

    #[tokio::test]
    async fn test_resume_from_offset() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z First").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Second").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        // Read first record and capture offset
        let mut reader1 =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();
        reader1.open().unwrap();
        let _record1 = reader1.next_record().await.unwrap().unwrap();
        let checkpoint_offset = reader1.checkpoint_offset();

        // Create new reader starting from checkpoint
        let mut config2 = config.clone();
        config2.read.start = ReadStart::StoredOffset;
        let mut reader2 = SourceReader::new_with_offset(
            "test".to_string(),
            &config2,
            ParseErrorStrategy::Panic,
            checkpoint_offset,
        )
        .unwrap();
        reader2.open().unwrap();

        // Should read second record
        let record2 = reader2.next_record().await.unwrap().unwrap();
        assert_eq!(record2.raw_text, "2025-12-04T10:00:01Z Second");

        // No more records
        let record3 = reader2.next_record().await.unwrap();
        assert!(record3.is_none());
    }

    #[tokio::test]
    async fn test_start_at_end() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z First").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Second").unwrap();
        temp_file.flush().unwrap();

        let mut config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );
        config.read.start = ReadStart::End;

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Should read nothing (started at end)
        let record = reader.next_record().await.unwrap();
        assert!(record.is_none());
    }

    #[tokio::test]
    async fn test_parse_error_drop() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z First").unwrap();
        writeln!(temp_file, "INVALID LINE").unwrap();
        writeln!(temp_file, "2025-12-04T10:00:01Z Second").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Drop).unwrap();

        reader.open().unwrap();

        // Read first record
        let record1 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record1.raw_text, "2025-12-04T10:00:00Z First");

        // Invalid line should be dropped, read second record
        let record2 = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record2.raw_text, "2025-12-04T10:00:01Z Second");
    }

    #[tokio::test]
    async fn test_parse_error_panic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "INVALID LINE").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Should return error
        let result = reader.next_record().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Should read nothing
        let record = reader.next_record().await.unwrap();
        assert!(record.is_none());
    }

    #[tokio::test]
    async fn test_last_line_without_timestamp_emitted_at_eof() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "2025-12-04T10:00:00Z Only line").unwrap();
        temp_file.flush().unwrap();

        let config = create_test_config(
            temp_file.path().to_path_buf(),
            r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)",
            "iso8601",
        );

        let mut reader =
            SourceReader::new("test".to_string(), &config, ParseErrorStrategy::Panic).unwrap();

        reader.open().unwrap();

        // Should emit the buffered line at EOF
        let record = reader.next_record().await.unwrap().unwrap();
        assert_eq!(record.raw_text, "2025-12-04T10:00:00Z Only line");

        // No more records
        let record2 = reader.next_record().await.unwrap();
        assert!(record2.is_none());
    }
}
